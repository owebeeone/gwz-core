//! A loopback TCP proxy in front of a fixture server that can end its
//! connections from the server's side: at once, or when the client next sends
//! bytes on them, or by resetting them. It plays a server that closes an idle
//! connection, one that closes it just as the client starts an exchange on it,
//! and a peer or middlebox that sends a reset in the middle of one.
use super::socket_wait::wait_readable;
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// How long `arm_existing` and `freeze_existing` wait for a link's client socket to be quiet.
const SETTLE: Duration = Duration::from_secs(10);

/// How many waits in a row (each lasts a [`TICK`]) find a client's socket empty, after a request, before the request
/// holds. A segment the client has sent can still be on its way: loopback delivers through a queue of its own, which
/// on a loaded machine is now and then milliseconds behind the sender (the last packet of an exchange, sent as its
/// session went idle, reached the proxy 3 ms after the test had armed the link).
const QUIET_TICKS: u32 = 3;

/// How long a forwarding thread waits for its socket to be readable before it looks at its link again.
const TICK: Duration = Duration::from_millis(20);

/// What a link does with the bytes its client sends, and the requests to change it.
///
/// A request (`arm_existing`, `freeze_existing`) takes effect for the bytes the client sends after the call returns,
/// not for bytes it had sent before it: those are in the socket already, or on their way to it, and the forwarding
/// thread, which may be slow to wake on a loaded machine, has still to read them. So the forwarding thread applies a
/// request only when it has found the client's socket empty for [`QUIET_TICKS`] waits since the request, and the
/// caller waits for that.
#[derive(Default)]
struct Gate {
    /// Ends the link, unforwarded, when the client next sends bytes.
    armed: AtomicBool,
    /// Drops the client's bytes and keeps the link open: a server that stalls.
    frozen: AtomicBool,
    /// `arm_existing` asked; `armed` is set once the client's socket has been quiet.
    arm_asked: AtomicBool,
    /// `freeze_existing` asked; `frozen` is set once the client's socket has been quiet.
    freeze_asked: AtomicBool,
    /// The link's forwarding thread has ended, so it settles nothing more.
    ended: AtomicBool,
    /// The error the client's next read is made to fail with, as a test's seam for what a socket can answer.
    fault: Mutex<Option<io::Error>>,
}
impl Gate {
    /// The client's socket has been quiet: what was asked now holds.
    fn settle(&self) {
        if self.arm_asked.swap(false, Ordering::AcqRel) {
            self.armed.store(true, Ordering::Release);
        }
        if self.freeze_asked.swap(false, Ordering::AcqRel) {
            self.frozen.store(true, Ordering::Release);
        }
    }

    /// The error the next read is to fail with, if a test asked for one.
    fn take_fault(&self) -> Option<io::Error> {
        self.fault.lock().unwrap().take()
    }

    fn asked(&self) -> bool {
        self.arm_asked.load(Ordering::Acquire) || self.freeze_asked.load(Ordering::Acquire)
    }

    fn waiting(&self) -> bool {
        !self.ended.load(Ordering::Acquire) && self.asked()
    }
}
/// Marks a gate's forwarding thread as ended, however the thread ends.
struct Ends(Arc<Gate>);
impl Drop for Ends {
    fn drop(&mut self) {
        self.0.ended.store(true, Ordering::Release);
    }
}

struct Link {
    client: TcpStream,
    server: TcpStream,
    gate: Arc<Gate>,
    /// The link is being reset: its forwarding threads end without a shutdown, so that the last handle closed ends
    /// the connection with a reset and not a FIN.
    reset: Arc<AtomicBool>,
}
impl Link {
    fn cut(&self) {
        let _ = self.client.shutdown(Shutdown::Both);
        let _ = self.server.shutdown(Shutdown::Both);
    }

    /// Resets the link: both sockets close with a linger of zero once every handle to them is closed, which sends a
    /// reset (an RST, which the client reports as a reset connection) and no FIN.
    fn reset(self) {
        self.reset.store(true, Ordering::Release);
        for socket in [&self.client, &self.server] {
            let _ = socket2::SockRef::from(socket).set_linger(Some(Duration::ZERO));
        }
    }
}

pub(crate) struct CutProxy {
    pub(crate) port: u16,
    /// The listeners' addresses, which `drop` connects to so that each accept loop sees the stop.
    wake: Vec<SocketAddr>,
    links: Arc<Mutex<Vec<Link>>>,
    accepted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl CutProxy {
    pub(crate) fn start(target: u16) -> Self {
        let listeners = super::loopback::bind();
        let port = listeners[0].local_addr().unwrap().port();
        let wake = listeners
            .iter()
            .map(|listener| listener.local_addr().unwrap())
            .collect();
        let links = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        for listener in listeners {
            let (shared, count, stopping) = (links.clone(), accepted.clone(), stop.clone());
            thread::spawn(move || {
                for client in listener.incoming() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(client) = client else {
                        continue;
                    };
                    let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else {
                        continue;
                    };
                    let gate = Arc::new(Gate::default());
                    let reset = Arc::new(AtomicBool::new(false));
                    let link = Link {
                        client: client.try_clone().unwrap(),
                        server: server.try_clone().unwrap(),
                        gate: gate.clone(),
                        reset: reset.clone(),
                    };
                    let (upstream, downstream) = (
                        (client.try_clone().unwrap(), server.try_clone().unwrap()),
                        (server, client),
                    );
                    shared.lock().unwrap().push(link);
                    // Counted once the link is registered, so that a link `connections` counts is one the
                    // marks and cuts reach.
                    count.fetch_add(1, Ordering::AcqRel);
                    let also = reset.clone();
                    thread::spawn(move || {
                        forward(upstream.0, upstream.1, Some(gate), &also);
                    });
                    thread::spawn(move || forward(downstream.0, downstream.1, None, &reset));
                }
            });
        }
        Self {
            port,
            wake,
            links,
            accepted,
            stop,
        }
    }

    /// Connections accepted so far.
    pub(crate) fn connections(&self) -> usize {
        self.accepted.load(Ordering::Acquire)
    }

    /// Ends every current connection now: the client reads EOF.
    pub(crate) fn cut_all(&self) {
        for link in self.links.lock().unwrap().iter() {
            link.cut();
        }
    }

    /// Resets every current connection now, as a peer that crashes, or a middlebox that drops its state, does: the
    /// client reads a reset connection and no end of stream (State review P2-5).
    pub(crate) fn reset_all(&self) {
        let links = std::mem::take(&mut *self.links.lock().unwrap());
        for link in links {
            link.reset();
        }
    }

    /// Ends the oldest connection now, and no other.
    pub(crate) fn cut_oldest(&self) {
        if let Some(link) = self.links.lock().unwrap().first() {
            link.cut();
        }
    }

    /// Every current connection ends, unforwarded, when its client next sends
    /// bytes. Later connections are not affected. What the client sent before
    /// the call is forwarded, and the call returns when that is done.
    pub(crate) fn arm_existing(&self) {
        self.mark(|gate| gate.arm_asked.store(true, Ordering::Release));
    }

    /// Every current connection stays open but forwards nothing more from its
    /// client. Later connections are not affected. What the client sent before
    /// the call is forwarded, and the call returns when that is done.
    pub(crate) fn freeze_existing(&self) {
        self.mark(|gate| gate.freeze_asked.store(true, Ordering::Release));
    }

    /// The next read of each current link's client socket fails with `error()` instead of reading (a test's seam for
    /// a socket's read that fails, or only seems to).
    pub(crate) fn fail_next_client_read(&self, error: impl Fn() -> io::Error) {
        for link in self.links.lock().unwrap().iter() {
            *link.gate.fault.lock().unwrap() = Some(error());
        }
    }

    /// Asks every current link's gate, then waits for each forwarding thread to find its client's socket empty
    /// (a link that has ended settles nothing and is not waited for). The client sends nothing in the meantime: the
    /// caller is the test that drives it.
    fn mark(&self, ask: impl Fn(&Gate)) {
        let gates: Vec<Arc<Gate>> = self
            .links
            .lock()
            .unwrap()
            .iter()
            .map(|link| link.gate.clone())
            .collect();
        for gate in &gates {
            ask(gate);
        }
        let deadline = Instant::now() + SETTLE;
        for gate in &gates {
            while gate.waiting() {
                assert!(
                    Instant::now() < deadline,
                    "a link's client socket never went empty"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

impl Drop for CutProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cut_all();
        // Wake the accept loops so that they see the stop.
        for address in &self.wake {
            let _ = TcpStream::connect(address);
        }
    }
}

/// Whether a failed read says nothing of its connection, which goes on: the read was interrupted.
fn retried(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
}

/// Copies `from` to `to`. Upstream, `client` holds the link's gate. A reset
/// link ends at once, closing its sockets without a shutdown.
///
/// The thread waits for its socket to be readable (a [`TICK`] at most) and then reads what is there, and the socket
/// has no read timeout: on Windows a read that raced its own timeout answered `ERROR_IO_PENDING` (997) now and then,
/// about once in five thousand exchanges of the `idle_loss` rows, which ended the link (a session the pool saw lost
/// that no test had cut) and, were it tried again, lost the bytes the read had been given.
fn forward(mut from: TcpStream, mut to: TcpStream, client: Option<Arc<Gate>>, reset: &AtomicBool) {
    let _ends = client.clone().map(Ends);
    let mut buffer = [0; 16 * 1024];
    // The waits in a row that found the client's socket empty since a request was seen.
    let mut quiet = 0;
    loop {
        let outcome = match client.as_ref().and_then(|gate| gate.take_fault()) {
            Some(error) => Err(error),
            None => match wait_readable(&from, TICK) {
                Ok(true) => from.read(&mut buffer),
                Ok(false) => Err(io::Error::from(io::ErrorKind::TimedOut)),
                Err(error) => Err(error),
            },
        };
        let count = match outcome {
            Ok(0) => break,
            Ok(count) => {
                quiet = 0;
                count
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if reset.load(Ordering::Acquire) {
                    return;
                }
                // Nothing is waiting in the socket: once that has held for a while, what the link was asked to
                // do holds from here.
                if let Some(gate) = &client {
                    if gate.asked() {
                        quiet += 1;
                        if quiet >= QUIET_TICKS {
                            gate.settle();
                            quiet = 0;
                        }
                    } else {
                        quiet = 0;
                    }
                }
                continue;
            }
            // A read that says nothing of the connection is tried again.
            Err(error) if retried(&error) => {
                if reset.load(Ordering::Acquire) {
                    return;
                }
                continue;
            }
            Err(_) => break,
        };
        if reset.load(Ordering::Acquire) {
            return;
        }
        if let Some(gate) = &client {
            if gate.armed.load(Ordering::Acquire) {
                break;
            }
            if gate.frozen.load(Ordering::Acquire) {
                continue;
            }
        }
        if to.write_all(&buffer[..count]).is_err() {
            break;
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
