//! Pageant as the session's agent channel (GwzTransportWindowsParityPlan.md, step 3.6; TR1.8 §4 and §5; TR4.8, part 2).
//!
//! The agent client speaks to a channel it can write to and read from. Pageant is not a stream: one request is one
//! `WM_COPYDATA` round trip (step 3.5's exchange). So the channel buffers what the client writes until it holds one
//! whole request, and the first read performs the exchange and then serves the reply as a stream of bytes, as the
//! client reads them. Everything else is the rules of TR1.8 §5:
//!
//! - **The exchange runs on a thread of its own.** `SendMessageTimeoutW` cannot be interrupted, so the setup thread,
//!   which owns the connection's clocks and its cancel, never makes the call: it waits in slices on the control, as
//!   every other wait of the setup does. That is what lets the connection's stall and setup-budget clocks end the
//!   attempt while Pageant's prompt is open (operator decision D5 (b): the confirmation wait is in the Network phase,
//!   there is no LocalInteraction), and lets Ctrl-C return at once while the blocked thread retires.
//! - **One send, never resent.** A request that timed out may still be running in Pageant. The thread holds the request
//!   until the send returns, reads nothing after a timeout, and the channel that gave up is down for good.
//! - **The send's own bound is finite and never zero** ([`REQUEST_CAP`]), also when `--ssh-timeout 0` has turned the
//!   clocks off. When both clocks are off it is the only bound, and the blocked thread ends by itself at it.
//! - **The window's identity is checked** before the send and again after the answer ([`pin::pinned_exchange`]).
//!
//! A thread whose send is still blocked when its channel is dropped keeps only what it owns (the request, the mapping
//! and a handle to the window's pin), so nothing borrowed outlives the setup; it ends at the latest at the cap, and
//! the setup does not wait for it.
use super::{
    agent_client::Channel,
    agent_job::Control,
    agent_source::AgentFault,
    pageant_exchange::{MAPPING_SIZE, MAX_PAYLOAD, PageantError},
};
use pin::pinned_exchange;
use std::{
    io::{self, Read, Write},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::Duration,
};

mod pin;
pub(crate) use pin::{PageantPin, PageantWindow};
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod sys;
        pub(crate) use sys::{find, open};
    }
}
// The rows that make a Pageant of their own pin its window as the probe does.
cfg_if::cfg_if! { if #[cfg(all(windows, test))] { pub(crate) use sys::pin_of; } }

/// The longest one request waits in `SendMessageTimeoutW`, and so the longest a blocked thread outlives its setup.
///
/// With the default clocks (`--ssh-timeout` 9 s, a 30 s setup budget) the connection gives up long before this, and
/// the thread then retires at the cap. With `--ssh-timeout 0` this is the whole bound. It must be finite, because
/// `SendMessageTimeoutW` takes no "forever" and a zero is an immediate timeout, not an absence of one; and it is
/// long enough for a person to read a prompt, find a passphrase and answer. 120 seconds is as long as the SSH clock
/// lets a person take at a credential helper's sign-in window, but it is not that allowance (Pageant has none, D5 (b)):
/// it bounds this call and nothing else. It is also inside the 150 s that an open grants each attempt
/// (`OPEN_ATTEMPT_MS`), so the open's own backstop never ends the wait first.
pub(crate) const REQUEST_CAP: Duration = Duration::from_secs(120);

/// The bound passed to the send for a request under `cap`: at least a millisecond, so that the call is never made
/// with a zero timeout.
pub(crate) fn send_bound(cap: Duration) -> Duration {
    cap.max(Duration::from_millis(1))
}

/// What the exchange thread hands back.
type Reply = Result<Vec<u8>, PageantError>;

enum State {
    /// Collecting the bytes of one request frame.
    Writing(Vec<u8>),
    /// The exchange is out; its reply arrives on the receiver.
    Waiting(Receiver<Reply>),
    /// A reply, framed as the agent client reads it, and how much of it has been read.
    Serving(Vec<u8>, usize),
    /// A reply that arrived and is not yet served.
    Arrived(Reply),
    /// Down for good: the exchange failed, or the wait for it was given up.
    Failed,
}

/// An agent channel over one Pageant window.
pub(crate) struct PageantChannel {
    window: Arc<dyn PageantWindow>,
    cap: Duration,
    state: State,
}

impl PageantChannel {
    pub(crate) fn new(window: Arc<dyn PageantWindow>) -> Self {
        Self {
            window,
            cap: REQUEST_CAP,
            state: State::Writing(Vec::new()),
        }
    }

    /// Starts the exchange of `request` on a thread of its own. The thread owns the request and a share of the
    /// window until the send returns.
    fn dispatch(&self, request: Vec<u8>) -> io::Result<Receiver<Reply>> {
        let (reply, arrival) = mpsc::channel();
        let (window, bound) = (self.window.clone(), send_bound(self.cap));
        thread::Builder::new()
            .name("gwz-pageant".into())
            .spawn(move || {
                // A channel that was dropped while this was blocked has nobody to tell; the result is dropped here.
                let _ = reply.send(pinned_exchange(&*window, &request, bound));
            })?;
        Ok(arrival)
    }

    /// Turns an arrived reply into the bytes the client reads, or the error it ends the channel with.
    fn receive(&mut self, reply: Reply) -> io::Result<()> {
        match reply {
            Ok(message) => {
                let mut framed = (message.len() as u32).to_be_bytes().to_vec();
                framed.extend_from_slice(&message);
                self.state = State::Serving(framed, 0);
                Ok(())
            }
            Err(error) => {
                self.state = State::Failed;
                Err(match error {
                    // A Pageant that is gone, or is no longer the one GWZ found, is an agent that is not running.
                    // It never selects another agent.
                    PageantError::NoWindow | PageantError::Replaced => {
                        AgentFault::NotRunning.into_error()
                    }
                    // A Pageant that will not serve the request, as one run by another account does not (it cannot
                    // open a mapping made for this account alone): the agent refused this account.
                    PageantError::Rejected => AgentFault::Denied.into_error(),
                    other => other.into(),
                })
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl PageantChannel {
            /// A channel whose send is bounded by `cap`, for the rows that cannot wait 120 seconds.
            pub(crate) fn with_cap(window: Arc<dyn PageantWindow>, cap: Duration) -> Self {
                Self {
                    cap,
                    ..Self::new(window)
                }
            }
        }
    }
}

/// The request in a complete frame of `bytes`, if it holds exactly one.
fn request_of(bytes: &[u8]) -> io::Result<&[u8]> {
    let Some(head) = bytes.first_chunk::<4>() else {
        return Err(io::ErrorKind::InvalidData.into());
    };
    let length = u32::from_be_bytes(*head) as usize;
    if length == 0 || length > MAX_PAYLOAD || bytes.len() != 4 + length {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(&bytes[4..])
}

impl Write for PageantChannel {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let State::Writing(frame) = &mut self.state else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        frame.extend_from_slice(data);
        // A request that cannot fit the mapping is refused as soon as the frame shows it, before anything is
        // dispatched.
        let declared = frame
            .first_chunk::<4>()
            .map(|head| u32::from_be_bytes(*head) as usize);
        if frame.len() > MAPPING_SIZE || declared.is_some_and(|length| length > MAX_PAYLOAD) {
            self.state = State::Failed;
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for PageantChannel {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        match std::mem::replace(&mut self.state, State::Failed) {
            State::Writing(frame) => {
                let request = match request_of(&frame) {
                    Ok(request) => request.to_vec(),
                    Err(error) => return Err(error),
                };
                self.state = State::Waiting(self.dispatch(request)?);
                Err(io::ErrorKind::WouldBlock.into())
            }
            State::Waiting(arrival) => match arrival.try_recv() {
                Ok(reply) => {
                    self.receive(reply)?;
                    self.read(out)
                }
                Err(TryRecvError::Empty) => {
                    self.state = State::Waiting(arrival);
                    Err(io::ErrorKind::WouldBlock.into())
                }
                Err(TryRecvError::Disconnected) => Err(io::ErrorKind::BrokenPipe.into()),
            },
            State::Arrived(reply) => {
                self.receive(reply)?;
                self.read(out)
            }
            State::Serving(bytes, taken) => {
                let count = out.len().min(bytes.len() - taken);
                out[..count].copy_from_slice(&bytes[taken..taken + count]);
                self.state = if taken + count == bytes.len() {
                    State::Writing(Vec::new())
                } else {
                    State::Serving(bytes, taken + count)
                };
                Ok(count)
            }
            State::Failed => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}

impl Channel for PageantChannel {
    /// Waits for the exchange's reply in slices of the control's quantum, so that its clocks and a cancel end the wait
    /// where they fall. A wait that ends in an error leaves the channel down; the thread that holds the request
    /// retires on its own.
    fn wait(&mut self, _writing: bool, control: &Control) -> io::Result<()> {
        let State::Waiting(arrival) = &self.state else {
            // A write never waits, and a reply that is here needs no wait.
            return control.check();
        };
        let mut arrived = None;
        let waited = control.wait_step(|slice| match arrival.recv_timeout(slice) {
            Ok(reply) => {
                arrived = Some(Ok(reply));
                Ok(true)
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => {
                arrived = Some(Err(io::ErrorKind::BrokenPipe));
                Ok(true)
            }
        });
        if let Err(error) = waited {
            self.state = State::Failed;
            return Err(error);
        }
        match arrived {
            Some(Ok(reply)) => self.state = State::Arrived(reply),
            Some(Err(kind)) => {
                self.state = State::Failed;
                return Err(kind.into());
            }
            None => {}
        }
        Ok(())
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
cfg_if::cfg_if! { if #[cfg(all(test, windows))] { mod windows_tests; } }
