//! Tests of the host context, the session context and `open` (session plan CS1.4).

use super::*;
use crate::model::ErrorCode;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

fn no_environment() -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_byte_pairs(Vec::new()).unwrap()
}

/// A job that finishes once its flag is set.
struct Latch(Arc<AtomicBool>);

impl SupervisedJob for Latch {
    fn poll(&mut self) -> bool {
        self.0.load(SeqCst)
    }
}

/// A job whose every poll panics. It counts its polls and marks its drop.
struct AlwaysPanics {
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl SupervisedJob for AlwaysPanics {
    fn poll(&mut self) -> bool {
        self.polls.fetch_add(1, SeqCst);
        panic!("a supervised job panicked");
    }
}

impl Drop for AlwaysPanics {
    fn drop(&mut self) {
        self.dropped.store(true, SeqCst);
    }
}

fn always_panics() -> (AlwaysPanics, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let job = AlwaysPanics {
        polls: polls.clone(),
        dropped: dropped.clone(),
    };
    (job, polls, dropped)
}

/// Waits, at most ten seconds, until `polls` counts a first poll.
fn await_first_poll(polls: &AtomicUsize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while polls.load(SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the job was never polled");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_public_types_cross_threads() {
    // Drivers share a host context across threads and open sessions from
    // any of them (§5.6, §10); the Python bridge's pump owns the client end.
    fn shareable<T: Send + Sync>() {}
    shareable::<HostContext>();
    shareable::<SessionOptions>();
    shareable::<EnvironmentSnapshot>();
    shareable::<ClientChannel>();
}

#[test]
fn open_creates_the_session_context_from_its_options() {
    let host = HostContext::new();
    let environment =
        EnvironmentSnapshot::from_byte_pairs(vec![(b"NAME".to_vec(), b"value".to_vec())]).unwrap();
    let mut options = SessionOptions::new(host.clone(), environment);
    options.limits.running_operations = 2;
    let channel = open(options).unwrap();
    let session = channel.session();
    assert_eq!(session.limits().running_operations, 2);
    assert!(session.environment().get("NAME").is_some());
    assert!(session.host().same_as(&host));
}

#[test]
fn open_refuses_invalid_limits_with_invalid_request() {
    let mut options = SessionOptions::new(HostContext::new(), no_environment());
    options.limits.operation_table =
        options.limits.running_operations + options.limits.queued_operations - 1;
    assert_eq!(open(options).unwrap_err().code, ErrorCode::InvalidRequest);
}

#[test]
fn sessions_share_the_host_context_they_are_given() {
    let host = HostContext::new();
    let first = open(SessionOptions::new(host.clone(), no_environment())).unwrap();
    let second = open(SessionOptions::new(host.clone(), no_environment())).unwrap();
    assert!(first.session().host().same_as(second.session().host()));
    assert!(!first.session().host().same_as(&HostContext::new()));
}

#[test]
fn dropping_a_host_context_ends_its_supervisor_thread_once_its_jobs_finish() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    assert!(!watch.running(), "no thread before the first job");
    let finished = Arc::new(AtomicBool::new(false));
    host.supervise(Box::new(Latch(finished.clone()))).unwrap();
    assert!(watch.running());
    drop(host);
    assert!(
        !watch.wait_ended(Duration::from_millis(100)),
        "the supervisor outlives its host context while a job runs"
    );
    finished.store(true, SeqCst);
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "the supervisor ends once its jobs have finished"
    );
}

#[test]
fn a_host_context_that_never_supervised_ends_at_once() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    drop(host);
    assert!(watch.wait_ended(Duration::ZERO));
    assert!(!watch.running());
}

#[test]
fn an_open_session_keeps_its_host_context() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    host.supervise(Box::new(Latch(Arc::new(AtomicBool::new(true)))))
        .unwrap();
    // The driver's handle moves into the session.
    let channel = open(SessionOptions::new(host, no_environment())).unwrap();
    assert!(
        !watch.wait_ended(Duration::from_millis(100)),
        "a session holds its host context"
    );
    drop(channel);
    assert!(watch.wait_ended(Duration::from_secs(10)));
}

#[test]
fn a_job_that_panics_is_quarantined_and_never_polled_again() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let (job, polls, dropped) = always_panics();
    host.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    // With only a quarantined job, the supervisor waits as with none.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(polls.load(SeqCst), 1, "polled exactly once");
    assert!(!dropped.load(SeqCst), "kept while the host context lives");
    drop(host);
    assert!(
        watch.wait_ended(Duration::from_secs(1)),
        "only quarantined jobs remain, so the supervisor ends"
    );
    assert!(dropped.load(SeqCst), "the quarantined job drops as it ends");
}

#[test]
fn a_quarantined_job_does_not_stop_the_others() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let (job, polls, _dropped) = always_panics();
    host.supervise(Box::new(job)).unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    host.supervise(Box::new(Latch(finished.clone()))).unwrap();
    await_first_poll(&polls);
    drop(host);
    assert!(
        !watch.wait_ended(Duration::from_millis(100)),
        "the latch runs"
    );
    finished.store(true, SeqCst);
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "the latch was polled to its end"
    );
    assert_eq!(polls.load(SeqCst), 1);
}
