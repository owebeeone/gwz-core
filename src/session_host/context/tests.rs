//! Tests of the host context, the session context and `open` (session plan
//! CS1.4, extended by CS1.9).

use super::*;
use crate::model::ErrorCode;
use std::sync::Barrier;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

fn no_environment() -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_byte_pairs(Vec::new()).unwrap()
}

/// The bound the shutdown tests use in place of the 5-second cleanup bound.
const BOUND: Duration = Duration::from_millis(300);

/// How late a shutdown may return past its bound on a loaded machine.
const SLACK: Duration = Duration::from_millis(700);

/// What nothing pending reports: no peer took part, so no peer cleanup
/// occurred (§8's `(0, false)`).
const NOTHING_PENDING: ShutdownReport = ShutdownReport {
    pending_local_work: 0,
    peer_cleanup_confirmed: false,
};

const ONE_PENDING: ShutdownReport = ShutdownReport {
    pending_local_work: 1,
    peer_cleanup_confirmed: false,
};

/// A job's polls and drops, and the flag that finishes it.
#[derive(Clone, Default)]
struct Counts {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    finish: Arc<AtomicBool>,
}

impl Counts {
    fn job(&self) -> Box<dyn SupervisedJob> {
        Box::new(Counted(self.clone()))
    }

    fn polls(&self) -> usize {
        self.polls.load(SeqCst)
    }

    fn drops(&self) -> usize {
        self.drops.load(SeqCst)
    }
}

/// A job that finishes once its flag is set, counting its polls and drops.
struct Counted(Counts);

impl SupervisedJob for Counted {
    fn poll(&mut self) -> bool {
        self.0.polls.fetch_add(1, SeqCst);
        self.0.finish.load(SeqCst)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.drops.fetch_add(1, SeqCst);
    }
}

/// Waits, at most ten seconds, for `callers`, so a hang fails the test
/// instead of stalling it.
fn join_bounded<R>(callers: Vec<thread::JoinHandle<R>>) -> Vec<R> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !callers.iter().all(thread::JoinHandle::is_finished) {
        assert!(Instant::now() < deadline, "a call never returned");
        thread::sleep(Duration::from_millis(5));
    }
    callers
        .into_iter()
        .map(|caller| caller.join().unwrap())
        .collect()
}

/// Shuts `host` down within `bound` from `count` threads at once, each
/// through its own handle, and returns each report and how long it took.
fn shut_down(host: &HostContext, bound: Duration, count: usize) -> Vec<(ShutdownReport, Duration)> {
    let start = Arc::new(Barrier::new(count));
    let callers = (0..count)
        .map(|_| {
            let (host, start) = (host.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                let began = Instant::now();
                (host.shutdown_within(bound), began.elapsed())
            })
        })
        .collect();
    join_bounded(callers)
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
    shareable::<ShutdownReport>();
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

#[test]
fn shutdown_returns_at_its_bound_while_a_job_runs_and_a_second_call_returns_the_same_report() {
    assert_eq!(CLEANUP_BOUND, Duration::from_secs(5), "reuse design §7");
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let job = Counts::default();
    host.supervise(job.job()).unwrap();
    let [(report, took)] = shut_down(&host, BOUND, 1).try_into().unwrap();
    assert!(took >= BOUND, "it waited for the job: {took:?}");
    assert!(took < BOUND + SLACK, "and returned at its bound: {took:?}");
    assert_eq!(report, ONE_PENDING);
    // The unfinished job stays under the supervisor, which polls it to its
    // end and then stops, while the host context lives.
    job.finish.store(true, SeqCst);
    assert!(watch.wait_ended(Duration::from_secs(10)));
    assert_eq!(job.drops(), 1);
    // A second call, from another handle, now finds nothing left, and still
    // returns the first call's report: it disposes nothing again.
    assert_eq!(host.clone().shutdown_within(BOUND), ONE_PENDING);
    assert_eq!(job.drops(), 1);
}

#[test]
fn shutdown_returns_once_its_jobs_finish_and_releases_the_supervisor() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let job = Counts::default();
    host.supervise(job.job()).unwrap();
    let finish = job.finish.clone();
    let finisher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        finish.store(true, SeqCst);
    });
    let [(report, took)] = shut_down(&host, Duration::from_secs(60), 1)
        .try_into()
        .unwrap();
    finisher.join().unwrap();
    // The supervisor reported the finish while shutdown waited, so the wait
    // held no lock the supervisor needs.
    assert_eq!(report, NOTHING_PENDING);
    assert!(took < Duration::from_secs(10), "not at its bound: {took:?}");
    assert_eq!(job.drops(), 1, "the finished job was dropped");
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "the supervisor stops without waiting for the host context's drop"
    );
}

#[test]
fn concurrent_shutdowns_wait_for_the_first_and_return_its_report() {
    let host = HostContext::new();
    let job = Counts::default();
    host.supervise(job.job()).unwrap();
    // Six calls that each waited a bound in turn would take six bounds.
    for (report, took) in shut_down(&host, BOUND, 6) {
        assert_eq!(report, ONE_PENDING);
        assert!(took < BOUND + SLACK, "{took:?}");
    }
    assert_eq!(job.drops(), 0);
    job.finish.store(true, SeqCst);
}

#[test]
fn a_quarantined_job_appears_in_the_pending_report() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let (job, polls, dropped) = always_panics();
    host.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    let [(report, took)] = shut_down(&host, Duration::from_secs(60), 1)
        .try_into()
        .unwrap();
    assert_eq!(report, ONE_PENDING, "it never finishes, so it is pending");
    assert!(
        took < Duration::from_secs(10),
        "and not waited for: {took:?}"
    );
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "then the supervisor stops, dropping it"
    );
    assert!(dropped.load(SeqCst));
    assert_eq!(polls.load(SeqCst), 1);
}

#[test]
fn a_drop_after_shutdown_disposes_nothing_more() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    let finished = Counts::default();
    finished.finish.store(true, SeqCst);
    host.supervise(finished.job()).unwrap();
    let (job, polls, dropped) = always_panics();
    host.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    assert_eq!(host.shutdown_within(Duration::from_secs(60)), ONE_PENDING);
    assert!(watch.wait_ended(Duration::from_secs(10)));
    assert_eq!((finished.drops(), dropped.load(SeqCst)), (1, true));
    // What shutdown disposed is gone, so the drop finds nothing to dispose.
    let started = Instant::now();
    drop(host);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it does not wait"
    );
    assert_eq!((finished.drops(), finished.polls()), (1, 1));
    assert!(watch.wait_ended(Duration::ZERO) && !watch.running());
}

#[test]
fn after_shutdown_no_job_is_supervised_and_no_session_opens() {
    let host = HostContext::new();
    let watch = host.supervisor_watch();
    assert_eq!(host.shutdown_within(BOUND), NOTHING_PENDING);
    let refused = Counts::default();
    assert!(host.clone().supervise(refused.job()).is_err());
    assert_eq!(
        (refused.polls(), refused.drops()),
        (0, 1),
        "dropped unpolled, so no work starts that nothing cleans up"
    );
    assert!(!watch.running(), "and no supervisor thread starts");
    let error = open(SessionOptions::new(host.clone(), no_environment())).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(host.shutdown_within(BOUND), NOTHING_PENDING);
}

#[test]
fn a_session_reads_transport_off_from_its_options_and_never_from_its_snapshot() {
    // TR1.5 has not named the switch's variable, so the snapshot holds
    // plausible names, each with values that read as on and as off.
    let names = [
        "GWZ_TRANSPORT_OFF",
        "GWZ_TRANSPORT",
        "GWZ_NO_TRANSPORT",
        "GWZ_DISABLE_TRANSPORT",
        "GWZ_TRANSPORT_DISABLED",
        "GWZ_NATIVE_TRANSPORT",
        "GWZ_OFF",
        "TRANSPORT_OFF",
        "transport_off",
        "gwz_transport_off",
    ];
    let host = HostContext::new();
    for value in ["1", "0", "true", "false", "on", "off", "yes", "no", ""] {
        let options = |transport_off| {
            let pairs = names.map(|name| (name.as_bytes().to_vec(), value.as_bytes().to_vec()));
            let environment = EnvironmentSnapshot::from_byte_pairs(pairs).unwrap();
            let mut options = SessionOptions::new(host.clone(), environment);
            assert!(!options.transport_off, "false from SessionOptions::new");
            options.transport_off = transport_off;
            options
        };
        // Two sessions of one host context, each with its own value.
        let on = open(options(true)).unwrap();
        let off = open(options(false)).unwrap();
        for (channel, expected) in [(&on, true), (&off, false)] {
            let session = channel.session();
            assert!(session.environment().get("GWZ_TRANSPORT_OFF").is_some());
            assert_eq!(session.transport_off(), expected, "{value:?}");
        }
    }
}

#[test]
fn a_session_drops_its_snapshot_when_it_ends() {
    // The session context owns the snapshot, which cannot be cloned
    // (`environment::tests`), so the context's drop is the snapshot's, and
    // each of its names and values overwrites its allocation as it drops.
    let host = HostContext::new();
    let environment =
        EnvironmentSnapshot::from_byte_pairs(vec![(b"GH_TOKEN".to_vec(), b"s3cr3t".to_vec())])
            .unwrap();
    let channel = open(SessionOptions::new(host.clone(), environment)).unwrap();
    let session = Arc::downgrade(channel.session());
    assert!(
        session
            .upgrade()
            .is_some_and(|session| session.environment().len() == 1)
    );
    drop(channel);
    assert!(
        session.upgrade().is_none(),
        "the session context and its snapshot dropped as the session ended"
    );
    assert_eq!(host.shutdown_within(BOUND), NOTHING_PENDING);
}
