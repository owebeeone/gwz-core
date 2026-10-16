//! Plan step 3.6's rows over a synthetic Pageant: a window of this process, on a thread of its own, that answers
//! `WM_COPYDATA` as Pageant does (`pageant_exchange::receiver`). The channel, the probe and the pin are the product's;
//! only the program behind the window is a stand-in. **No parity claim rests on these**: the rows that need PuTTY's own
//! `pageant.exe`, in the operator's console session, are `ssh_tests/real_pageant.rs`'s and stay owed.
use super::{sys, *};
use crate::git::endpoint::{
    agent_client::Agent,
    agent_job::{Job, TimeoutReason, timeout_reason, wall_clock},
    agent_source::{AgentFault, AgentSource},
    pageant_exchange::receiver::{Receiver, Reply},
    ssh_tests::agent_fixture_pipe::{Fixture as Keys, Request},
};
use std::{
    ffi::OsStr,
    task::{Context, Poll, Waker},
    time::Instant,
};

const NO_KEYS: [u8; 5] = [12, 0, 0, 0, 0];
const CAP: Duration = Duration::from_secs(10);
const STALL: Duration = Duration::from_millis(200);

fn control(stall: Duration) -> Arc<Control> {
    Control::scripted(None, stall, Duration::from_secs(5), wall_clock())
}

/// A Pageant that holds `keys`: the fixture agent's answers, served through the window and not through its pipe.
fn pageant_with(keys: &Keys) -> Receiver {
    Receiver::start(Reply::Handler(Box::new(keys.agent.responder())))
}

/// The agent client over the window `pageant` made, pinned as the probe pins it, with `cap` as the send's bound.
fn agent(pageant: &Receiver, control: &Arc<Control>, cap: Duration) -> Agent<PageantChannel> {
    let pin = sys::pin_of(pageant.window).expect("the window has an owner");
    let window = sys::open(&pin).expect("the caller's identity");
    Agent::new(
        PageantChannel::with_cap(Arc::new(window), cap),
        control.clone(),
    )
}

#[test]
fn the_probe_finds_a_window_by_pageants_own_class_and_title_and_pins_its_owner() {
    assert!(
        sys::find().is_none(),
        "a Pageant is running on this desktop; the test will not use it"
    );
    let pageant = Receiver::start_pageant(Reply::Message(NO_KEYS.to_vec()));
    let pin = sys::find().expect("the hidden Pageant window is found");
    assert_eq!(pin.window(), pageant.window as usize);
    assert_eq!(pin.process(), std::process::id());
    assert!(pin.holder().is_some(), "the owner's process is held");
    // A visible Pageant beats the variable, which is not consulted; the source is what the probe found.
    let pipe = OsStr::new(r"\\.\pipe\whatever-the-variable-names");
    assert_eq!(
        AgentSource::capture(Some(pipe)),
        AgentSource::Pageant {
            sock_ignored: true,
            pin: pin.clone()
        }
    );
    drop(pageant);
    // Gone, the probe finds none, and the variable's pipe is the source.
    assert!(sys::find().is_none());
    assert!(matches!(
        AgentSource::capture(Some(pipe)),
        AgentSource::NamedPipe(_)
    ));
}

#[test]
fn the_agent_client_lists_and_signs_through_a_window_under_its_pin() {
    let keys = Keys::new();
    let pageant = pageant_with(&keys);
    let control = control(Duration::ZERO);
    let mut agent = agent(&pageant, &control, CAP);
    let listed = agent.identities().unwrap();
    assert_eq!(listed, keys.blobs());
    // RSA at flag 4 (`rsa-sha2-512`), the only one libssh2 on Windows offers.
    let signature = agent.sign(&listed[1], b"data to sign", 4).unwrap();
    assert!(signature.is_some_and(|signature| !signature.is_empty()));
    assert_eq!(
        keys.requests(),
        [Request::List, Request::Sign { key: 1, flags: 4 }]
    );
    // Each request had a mapping of its own.
    let mut names: Vec<_> = pageant.seen().into_iter().map(|seen| seen.name).collect();
    assert_eq!(names.len(), 2);
    names.dedup();
    assert_eq!(names.len(), 2);
}

#[test]
fn a_window_whose_owner_is_not_the_pinned_process_is_refused_and_sent_nothing() {
    let keys = Keys::new();
    let pageant = pageant_with(&keys);
    let real = sys::pin_of(pageant.window).unwrap();
    // The pin names another process for the window: as if the window had been replaced by another program's.
    let other = PageantPin::new(real.window(), std::process::id() + 4, 0, None);
    let window = sys::open(&other).unwrap();
    let control = control(Duration::ZERO);
    let mut agent = Agent::new(PageantChannel::with_cap(Arc::new(window), CAP), control);
    let error = agent.identities().unwrap_err();
    assert_eq!(error.kind(), AgentFault::NotRunning.into_error().kind());
    assert!(pageant.seen().is_empty(), "nothing was sent");
    assert!(keys.requests().is_empty());
}

#[test]
fn a_pageant_that_went_away_is_refused_and_the_openssh_pipe_is_never_tried() {
    // Pageant answers while it is there; the OpenSSH agent's pipe holds keys that would authenticate.
    let pageant_keys = Keys::new();
    let pipe_keys = Keys::new();
    let pageant = pageant_with(&pageant_keys);
    let control = control(Duration::ZERO);
    let mut first = agent(&pageant, &control, CAP);
    assert!(!first.identities().unwrap().is_empty());
    let pin = sys::pin_of(pageant.window).unwrap();
    drop(pageant);
    // The window is gone: a new channel on the same pin is the agent not running, not another agent.
    let gone = sys::open(&pin).unwrap();
    let mut second = Agent::new(
        PageantChannel::with_cap(Arc::new(gone), CAP),
        control.clone(),
    );
    let error = second.identities().unwrap_err();
    assert_eq!(error.kind(), AgentFault::NotRunning.into_error().kind());
    assert!(pipe_keys.requests().is_empty(), "the pipe was never tried");
}

#[test]
fn the_source_opens_pageant_and_never_the_pipe_when_both_run() {
    // B06 and X9 at the source: Pageant wins; the pipe agent sees nothing, and a Pageant with no key is an empty
    // list, which the agent step refuses (the pipe's keys are not tried).
    let pipe = Keys::new();
    let with_keys = Keys::new();
    let pageant = pageant_with(&with_keys);
    let pin = sys::pin_of(pageant.window).unwrap();
    let source = AgentSource::Pageant {
        sock_ignored: true,
        pin,
    };
    let control = control(Duration::ZERO);
    let mut agent = source.open(control.clone()).unwrap();
    assert_eq!(agent.identities().unwrap(), with_keys.blobs());
    assert!(pipe.requests().is_empty());
    let empty = Receiver::start(Reply::Message(NO_KEYS.to_vec()));
    let source = AgentSource::Pageant {
        sock_ignored: true,
        pin: sys::pin_of(empty.window).unwrap(),
    };
    let mut agent = source.open(control).unwrap();
    assert!(agent.identities().unwrap().is_empty());
    assert!(pipe.requests().is_empty());
}

#[test]
fn neither_agent_present_refuses_before_any_agent_connection_opens() {
    // B07: no Pageant window, and the pipe `SSH_AUTH_SOCK` names is not there.
    let absent = format!(r"\\.\pipe\gwz-absent-pageant-row-{}", std::process::id());
    let source = AgentSource::capture(Some(OsStr::new(&absent)));
    assert!(matches!(source, AgentSource::NamedPipe(_)));
    let error = source
        .open(control(Duration::ZERO))
        .map(|_| ())
        .unwrap_err();
    assert_eq!(AgentFault::of_error(&error), Some(AgentFault::NotRunning));
}

#[test]
fn a_pageant_that_will_not_serve_the_request_is_the_agent_refusing_this_account() {
    // What Pageant does when it cannot open the mapping made for this account alone, as a Pageant of another user
    // cannot: it says it did not serve the request.
    let refusing = Receiver::start(Reply::Refuse);
    let error = agent(&refusing, &control(Duration::ZERO), CAP)
        .identities()
        .unwrap_err();
    assert_eq!(error.kind(), AgentFault::Denied.into_error().kind());
}

/// How long a "confirmation prompt" lasts in the rows below.
const PROMPT: Duration = Duration::from_millis(700);

fn prompting(delay: Duration) -> Receiver {
    Receiver::start(Reply::Late {
        delay,
        message: NO_KEYS.to_vec(),
    })
}

#[test]
fn a_prompt_that_outlasts_the_stall_limit_fails_the_attempt_and_one_inside_it_succeeds() {
    // Late: the stall clock runs while the window's thread is busy with its prompt.
    let slow = prompting(PROMPT);
    let begun = Instant::now();
    let error = agent(&slow, &control(STALL), CAP).identities().unwrap_err();
    assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
    assert!(begun.elapsed() < PROMPT, "{:?}", begun.elapsed());
    // Early: the same stall, an answer inside it.
    let quick = prompting(Duration::from_millis(40));
    assert!(
        agent(&quick, &control(STALL), CAP)
            .identities()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn with_the_stall_off_the_send_still_has_a_finite_nonzero_bound() {
    // `--ssh-timeout 0`: the answer that failed the stall above succeeds, so the call was not made with a zero.
    let slow = prompting(PROMPT);
    assert!(
        agent(&slow, &control(Duration::ZERO), CAP)
            .identities()
            .unwrap()
            .is_empty()
    );
    // A window that never answers ends at the cap, with the exchange's own timeout.
    let cap = Duration::from_millis(300);
    let silent = prompting(Duration::from_secs(2));
    let begun = Instant::now();
    let error = agent(&silent, &control(Duration::ZERO), cap)
        .identities()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(timeout_reason(&error).is_none());
    // The wait is the system timer's, in whole milliseconds, and `Instant` is the performance counter: a timer that
    // fires on its tick can read 0.3 ms short of the cap (299.7 ms of 300 on dabeest), so the lower bound allows a tick.
    assert!(
        begun.elapsed() + Duration::from_millis(16) >= cap
            && begun.elapsed() < Duration::from_millis(1800),
        "{:?}",
        begun.elapsed()
    );
}

#[test]
fn a_cancel_returns_at_once_while_the_blocked_send_retires() {
    let pageant = prompting(Duration::from_secs(2));
    let window = pageant.window as usize;
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        let pin = sys::pin_of(window as windows_sys::Win32::Foundation::HWND).unwrap();
        let channel = PageantChannel::with_cap(Arc::new(sys::open(&pin).unwrap()), CAP);
        Agent::new(channel, control).identities()
    })
    .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while pageant.seen().is_empty() {
        assert!(
            Instant::now() < until,
            "the request never reached the window"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let begun = Instant::now();
    job.cancel();
    let result = loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            break result;
        }
        assert!(Instant::now() < until, "the cancel did not return");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
    assert!(begun.elapsed() < Duration::from_millis(500));
    // The receiver's thread, and the send blocked on it, are let go when the window is.
    drop(pageant);
}
