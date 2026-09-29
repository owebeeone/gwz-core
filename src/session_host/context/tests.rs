//! Tests of the host context, the session context and `open` (session plan
//! CS1.4, extended by CS1.9). The supervisor's own tests moved with it to
//! gwz-session-host (crate map §6 step 4); these keep what the host context
//! composes: its report, its drop and `open`'s refusal after `shutdown`.

use super::*;
use crate::model::ErrorCode;
use gwz_session_contract::Tag;
use std::sync::Barrier;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::thread;
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
fn each_limits_refusal_keeps_its_text() {
    // The limits' validation lives in gwz-session-host, with its own error
    // (crate map §6 step 4); `open` still refuses with `invalid_request`, in
    // the words it used before the move.
    type Case = (fn(&mut Limits), &'static str);
    let cases: [Case; 5] = [
        (
            |limits| limits.event_log = 1,
            "session limit event_log must be at least 2",
        ),
        (
            |limits| limits.operation_table = 1,
            "session limit operation_table must hold at least running_operations plus queued_operations",
        ),
        (
            |limits| limits.read_bytes = crate::session_host::MAX_FRAME_BYTES,
            "session limit read_bytes must be at most half the 64 MiB frame size",
        ),
        (
            |limits| limits.outstanding_calls = usize::MAX,
            "session limits outstanding_calls plus control_reserve overflow a channel queue",
        ),
        (
            |limits| limits.close_wait = Duration::from_secs(60 * 60 + 1),
            "session limit close_wait must be at most one hour",
        ),
    ];
    for (change, text) in cases {
        let mut options = SessionOptions::new(HostContext::new(), no_environment());
        change(&mut options.limits);
        let error = open(options).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{text}");
        assert_eq!(error.message, text);
    }
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
    let session = Arc::downgrade(&channel.session());
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

#[test]
fn open_returns_the_client_end_of_an_in_process_channel() {
    // CS1.2's core side: `open` wires gwz-session-channel's pair, whose host
    // end waits for CS2.2's host. A call reaches the host end, and a reply
    // comes back, through the methods and through the traits alike.
    let channel = test_session();
    let call = Frame::new(Tag::SessionCall, vec![0xa0]);
    channel.send(call.clone(), Lane::Call).unwrap();
    assert_eq!(channel.with_host_end(|host| host.recv()), Some(Ok(call)));
    let reply = Frame::new(Tag::SessionReply, vec![0xa1]);
    let sent = channel.with_host_end(|host| host.send(reply.clone(), Lane::Call));
    assert!(matches!(sent, Some(Ok(()))), "{sent:?}");
    assert_eq!(FrameSource::recv(&channel), Ok(reply));
    let cancel = Frame::new(Tag::SessionCall, vec![0xa2]);
    FrameSink::send(&channel, cancel.clone(), Lane::Control).unwrap();
    assert_eq!(channel.with_host_end(|host| host.recv()), Some(Ok(cancel)));
}

#[test]
fn a_full_lane_refuses_at_the_client_end() {
    // Each queue holds the outstanding calls on the call lane and the
    // control reserve on the control lane (§3), as `open`'s limits set them.
    // `Limits` is gwz-session-host's non-exhaustive type, set field by field.
    let mut options = SessionOptions::new(HostContext::new(), no_environment());
    options.limits.outstanding_calls = 1;
    options.limits.control_reserve = 1;
    let channel = open(options).unwrap();
    let call = |marker: u8| Frame::new(Tag::SessionCall, vec![marker]);
    channel.send(call(1), Lane::Call).unwrap();
    match channel.send(call(2), Lane::Call) {
        Err(SendError::Full(frame)) => assert_eq!(frame, call(2), "the frame comes back"),
        result => panic!("expected a full call lane, got {result:?}"),
    }
    channel
        .send(call(3), Lane::Control)
        .expect("the control reserve takes a control frame");
    assert!(matches!(
        channel.send(call(4), Lane::Control),
        Err(SendError::Full(_))
    ));
    assert_eq!(channel.with_host_end(|host| host.recv()), Some(Ok(call(1))));
    assert_eq!(channel.with_host_end(|host| host.recv()), Some(Ok(call(3))));
}

#[test]
fn closing_the_client_channel_ends_the_session_at_both_ends() {
    let channel = test_session();
    channel.close();
    let frame = Frame::new(Tag::SessionCall, vec![0xa0]);
    match channel.send(frame.clone(), Lane::Call) {
        Err(SendError::Closed(returned, Closed::Local)) => assert_eq!(returned, frame),
        result => panic!("expected the closure, got {result:?}"),
    }
    assert_eq!(channel.recv(), Err(Closed::Local));
    // The host's end went with the session: it closed as it dropped.
    assert!(channel.with_host_end(|_| ()).is_none());
}

#[test]
fn closing_the_client_channel_drops_its_snapshot() {
    // The snapshot is zeroized when its session ends (§5.6 as amended), and
    // `close()` ends the session as a drop does: with the handle still alive,
    // the session context, and the snapshot it owns, are gone at the close.
    let environment =
        EnvironmentSnapshot::from_byte_pairs(vec![(b"GH_TOKEN".to_vec(), b"s3cr3t".to_vec())])
            .unwrap();
    let channel = open(SessionOptions::new(HostContext::new(), environment)).unwrap();
    let session = Arc::downgrade(&channel.session());
    assert!(session.upgrade().is_some());
    channel.close();
    assert!(
        session.upgrade().is_none(),
        "the session context and its snapshot dropped at the close"
    );
    // The handle lives on, closed, and closing again changes nothing.
    assert_eq!(channel.recv(), Err(Closed::Local));
    FrameSink::close(&channel);
    assert_eq!(channel.recv(), Err(Closed::Local));
}
