//! Plan step 3.6's rows that need no operating system: the channel over a scripted Pageant window. It buffers the
//! agent client's request, performs the one exchange on a thread of its own at the first read, serves the reply, and
//! gives up under the connection's clocks and a cancel while that thread retires; the identity pin is checked on both
//! sides of the send. The same rows over a synthetic Pageant window are `windows_tests.rs`'s.
use super::*;
use crate::git::endpoint::{
    agent_client::Agent,
    agent_job::{Job, TimeoutReason, timeout_reason, wall_clock},
    agent_source::AgentFault,
};
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread,
    time::Instant,
};

type Answer = Box<dyn Fn(&[u8], Duration) -> Result<Vec<u8>, PageantError> + Send + Sync>;

/// A Pageant window that answers as scripted, and records what it was sent and how many sends have returned.
struct Fake {
    answer: Answer,
    /// The answers `owner_pinned` gives, in order; true once they run out.
    owners: Mutex<VecDeque<bool>>,
    sent: Mutex<Vec<(Vec<u8>, Duration)>>,
    returned: AtomicUsize,
}

impl Fake {
    fn new(
        answer: impl Fn(&[u8], Duration) -> Result<Vec<u8>, PageantError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            answer: Box::new(answer),
            owners: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            returned: AtomicUsize::new(0),
        })
    }

    fn owners(self: &Arc<Self>, answers: &[bool]) -> Arc<Self> {
        self.owners.lock().unwrap().extend(answers.iter().copied());
        self.clone()
    }

    fn requests(&self) -> Vec<Vec<u8>> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(request, _)| request.clone())
            .collect()
    }

    fn bounds(&self) -> Vec<Duration> {
        self.sent.lock().unwrap().iter().map(|(_, b)| *b).collect()
    }

    fn returned(&self) -> usize {
        self.returned.load(Ordering::SeqCst)
    }

    /// Waits until `count` sends have returned.
    fn wait_returned(&self, count: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        while self.returned() < count {
            assert!(Instant::now() < until, "the send never returned");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl PageantWindow for Fake {
    fn owner_pinned(&self) -> bool {
        self.owners.lock().unwrap().pop_front().unwrap_or(true)
    }

    fn send(&self, request: &[u8], bound: Duration) -> Result<Vec<u8>, PageantError> {
        self.sent.lock().unwrap().push((request.to_vec(), bound));
        let answer = (self.answer)(request, bound);
        self.returned.fetch_add(1, Ordering::SeqCst);
        answer
    }
}

fn clocks(stall: Duration, aggregate: Option<Duration>) -> Arc<Control> {
    Control::scripted(
        aggregate.map(|within| Instant::now() + within),
        stall,
        Duration::from_secs(5),
        wall_clock(),
    )
}

fn string(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(bytes);
    out
}

/// An identities answer listing `keys`.
fn identities_answer(keys: &[&[u8]]) -> Vec<u8> {
    let mut out = vec![12];
    out.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    for key in keys {
        out.extend(string(key));
        out.extend(string(b"comment"));
    }
    out
}

const KEY: &[u8] = b"a key blob";

fn listing(_: &[u8], _: Duration) -> Result<Vec<u8>, PageantError> {
    Ok(identities_answer(&[KEY]))
}

/// The agent client over a channel to `window`, with `cap` as the send's bound.
fn agent(window: &Arc<Fake>, cap: Duration, control: &Arc<Control>) -> Agent<PageantChannel> {
    Agent::new(
        PageantChannel::with_cap(window.clone(), cap),
        control.clone(),
    )
}

const CAP: Duration = Duration::from_secs(5);

#[test]
fn a_written_request_is_buffered_and_the_first_read_performs_the_exchange() {
    let window = Fake::new(|_, _| Ok(vec![12, 0, 0, 0, 0]));
    let mut channel = PageantChannel::with_cap(window.clone(), CAP);
    let control = clocks(Duration::ZERO, None);
    channel.write_all(&[0, 0, 0, 1, 11]).unwrap();
    assert!(window.requests().is_empty(), "nothing is sent by a write");
    let mut byte = [0u8; 1];
    // The first read starts the exchange, which has no reply yet.
    assert_eq!(
        channel.read(&mut byte).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    channel.wait(false, &control).unwrap();
    // The reply is served as the agent client reads it: a four-byte length, then the message, a byte at a time.
    let mut served = Vec::new();
    while served.len() < 9 {
        assert_eq!(channel.read(&mut byte).unwrap(), 1);
        served.push(byte[0]);
    }
    assert_eq!(served, [0, 0, 0, 5, 12, 0, 0, 0, 0]);
    assert_eq!(window.requests(), [vec![11]]);
    // Another request goes the same way, and is a second exchange.
    channel.write_all(&[0, 0, 0, 1, 11]).unwrap();
    assert_eq!(
        channel.read(&mut byte).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    channel.wait(false, &control).unwrap();
    assert_eq!(channel.read(&mut byte).unwrap(), 1);
    assert_eq!(window.requests().len(), 2);
}

#[test]
fn the_agent_client_lists_and_signs_one_exchange_per_request() {
    let window = Fake::new(|request, _| {
        Ok(match request[0] {
            11 => identities_answer(&[KEY]),
            13 => [vec![14], string(b"a signature")].concat(),
            other => panic!("request {other}"),
        })
    });
    let control = clocks(Duration::ZERO, None);
    let mut agent = agent(&window, CAP, &control);
    assert_eq!(agent.identities().unwrap(), [KEY.to_vec()]);
    let signature = agent.sign(KEY, b"data", 4).unwrap();
    assert_eq!(signature.as_deref(), Some(&b"a signature"[..]));
    let requests = window.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], [11]);
    let expected = [vec![13], string(KEY), string(b"data"), vec![0, 0, 0, 4]].concat();
    assert_eq!(requests[1], expected);
}

#[test]
fn a_request_over_the_mapping_is_refused_before_anything_is_dispatched() {
    let window = Fake::new(listing);
    let mut channel = PageantChannel::with_cap(window.clone(), CAP);
    // The largest frame goes; one byte more does not, and the length it declares is refused as soon as it is known.
    let mut frame = (MAX_PAYLOAD as u32).to_be_bytes().to_vec();
    frame.extend(vec![13; MAX_PAYLOAD]);
    channel.write_all(&frame).unwrap();
    let mut channel = PageantChannel::with_cap(window.clone(), CAP);
    let mut over = ((MAX_PAYLOAD + 1) as u32).to_be_bytes().to_vec();
    over.extend(vec![13; MAX_PAYLOAD + 1]);
    assert_eq!(
        channel.write(&over).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut byte = [0u8; 1];
    assert!(channel.read(&mut byte).is_err());
    assert!(window.requests().is_empty());
    // A frame that is not complete when the client reads is not a request either.
    let mut channel = PageantChannel::with_cap(window.clone(), CAP);
    channel.write_all(&[0, 0, 0, 9, 11]).unwrap();
    assert_eq!(
        channel.read(&mut byte).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(window.requests().is_empty());
}

#[test]
fn the_send_is_bounded_by_a_finite_cap_that_is_never_zero_with_the_clocks_on_or_off() {
    // The stall and setup-budget clocks bound the wait on this side; the call's own bound is the cap, whether or not
    // they run (`--ssh-timeout 0` turns them off).
    for (stall, aggregate) in [
        (Duration::ZERO, None),
        (Duration::from_secs(9), Some(Duration::from_secs(30))),
        (Duration::ZERO, Some(Duration::from_secs(30))),
    ] {
        let window = Fake::new(listing);
        let control = clocks(stall, aggregate);
        agent(&window, Duration::from_millis(777), &control)
            .identities()
            .unwrap();
        assert_eq!(window.bounds(), [Duration::from_millis(777)]);
    }
    assert_eq!(send_bound(Duration::ZERO), Duration::from_millis(1));
    // The cap production uses is a whole number of seconds: finite, nonzero, and within the 150 s an open allows each
    // attempt (`open_attempt_ms`), so the open's own backstop never ends first.
    assert_eq!(send_bound(REQUEST_CAP), REQUEST_CAP);
    assert!(REQUEST_CAP >= Duration::from_secs(1) && REQUEST_CAP <= Duration::from_secs(150));
}

/// How long a "confirmation prompt" lasts in the rows below.
const PROMPT: Duration = Duration::from_millis(500);
const STALL: Duration = Duration::from_millis(200);

#[test]
fn a_reply_after_the_stall_limit_fails_the_attempt_and_one_before_it_succeeds() {
    // Late: the stall clock runs while Pageant's prompt is open, and the attempt ends at the limit.
    let window = Fake::new(|_, _| {
        thread::sleep(PROMPT);
        Ok(identities_answer(&[KEY]))
    });
    let control = clocks(STALL, None);
    let mut late = agent(&window, CAP, &control);
    let begun = Instant::now();
    let error = late.identities().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
    assert!(begun.elapsed() < PROMPT, "{:?}", begun.elapsed());
    // The request is still out: the reply that comes later is never used, and the channel stays down.
    assert_eq!(window.returned(), 0);
    assert_eq!(
        late.sign(KEY, b"data", 0).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    window.wait_returned(1);
    assert_eq!(window.requests().len(), 1, "no resend");
    // Early: the same limit, a reply inside it.
    let window = Fake::new(|_, _| {
        thread::sleep(Duration::from_millis(40));
        Ok(identities_answer(&[KEY]))
    });
    let control = clocks(STALL, None);
    assert_eq!(
        agent(&window, CAP, &control).identities().unwrap(),
        [KEY.to_vec()]
    );
}

#[test]
fn with_the_stall_off_a_slow_reply_succeeds_and_a_silent_pageant_ends_at_the_cap() {
    // The reply that failed the stall above succeeds once the clocks are off (`--ssh-timeout 0`).
    let window = Fake::new(|_, _| {
        thread::sleep(PROMPT);
        Ok(identities_answer(&[KEY]))
    });
    let control = clocks(Duration::ZERO, None);
    assert_eq!(
        agent(&window, CAP, &control).identities().unwrap(),
        [KEY.to_vec()]
    );
    // A Pageant that never answers: the send returns at its own bound, the cap, as `SendMessageTimeoutW` does.
    let cap = Duration::from_millis(300);
    let silent = Fake::new(|_, bound| {
        thread::sleep(bound);
        Err(PageantError::Timeout)
    });
    let control = clocks(Duration::ZERO, None);
    let begun = Instant::now();
    let error = agent(&silent, cap, &control).identities().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        timeout_reason(&error).is_none(),
        "the cap is not a clock's timeout: {error:?}"
    );
    let took = begun.elapsed();
    assert!(
        took >= cap && took < Duration::from_secs(2),
        "ended at the cap: {took:?}"
    );
    assert_eq!(silent.requests().len(), 1, "no resend");
}

fn poll<T: Send + 'static>(job: &mut Job<T>, within: Duration) -> Option<io::Result<T>> {
    let until = Instant::now() + within;
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return Some(result);
        }
        if Instant::now() >= until {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_cancel_returns_at_once_while_the_blocked_request_retires() {
    let (entered_tx, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let window = Fake::new(move |_, _| {
        entered_tx.send(()).unwrap();
        let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(10));
        Ok(identities_answer(&[KEY]))
    });
    let held = window.clone();
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent(&held, CAP, &control).identities()
    })
    .unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let begun = Instant::now();
    job.cancel();
    let result = poll(&mut job, Duration::from_secs(2)).expect("the cancel returns");
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
    assert!(begun.elapsed() < Duration::from_millis(500));
    // The request is still blocked, and ends on its own once Pageant lets it go.
    assert_eq!(window.returned(), 0);
    release.send(()).unwrap();
    window.wait_returned(1);
}

#[test]
fn a_window_that_is_not_the_pinned_owner_refuses_before_and_after_the_send() {
    // Before: nothing is sent.
    let window = Fake::new(listing).owners(&[false]);
    let error = pinned_exchange(&*window, &[11], CAP).unwrap_err();
    assert_eq!(error, PageantError::Replaced);
    assert!(window.requests().is_empty());
    // After: the reply is dropped and the result is the refusal.
    let window = Fake::new(listing).owners(&[true, false]);
    let error = pinned_exchange(&*window, &[11], CAP).unwrap_err();
    assert_eq!(error, PageantError::Replaced);
    assert_eq!(window.requests().len(), 1);
    // A send that failed is the send's error, and the owner is not asked again.
    let window = Fake::new(|_, _| Err(PageantError::Timeout)).owners(&[true, false]);
    assert_eq!(
        pinned_exchange(&*window, &[11], CAP).unwrap_err(),
        PageantError::Timeout
    );
}

#[test]
fn a_pageant_that_went_away_is_the_agent_not_running_and_never_another_agent() {
    for gone in [PageantError::NoWindow, PageantError::Replaced] {
        let window = Fake::new(move |_, _| Err(gone));
        let control = clocks(Duration::ZERO, None);
        let error = agent(&window, CAP, &control).identities().unwrap_err();
        // The agent client keeps an error's kind and no more: the kind is `NotFound`, which the setup reports as
        // `Unavailable` with the cause `NotFound`, and the host words from that and the source.
        assert_eq!(
            error.kind(),
            AgentFault::NotRunning.into_error().kind(),
            "{gone:?}"
        );
    }
    // A Pageant that will not serve the request is the agent refusing this account.
    let window = Fake::new(|_, _| Err(PageantError::Rejected));
    let control = clocks(Duration::ZERO, None);
    let error = agent(&window, CAP, &control).identities().unwrap_err();
    assert_eq!(error.kind(), AgentFault::Denied.into_error().kind());
    // A damaged reply is a protocol error; the channel does not recover from it.
    let window = Fake::new(|_, _| Err(PageantError::BadReplyType(99)));
    let control = clocks(Duration::ZERO, None);
    let mut damaged = agent(&window, CAP, &control);
    assert_eq!(
        damaged.identities().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(window.requests().len(), 1);
}
