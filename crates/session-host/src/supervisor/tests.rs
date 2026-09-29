#![cfg(test)]
//! Tests of the supervisor (session plan CS1.4, extended by CS1.9), moved
//! from gwz-core's `session_host/context/tests.rs`. A test whose subject was
//! the supervisor keeps its name. One whose subject was the host context
//! holding it is renamed for the supervisor here, and gwz-core keeps its own
//! version, which drives the supervisor through the host context:
//! - `dropping_a_host_context_ends_its_supervisor_thread_once_its_jobs_finish`
//!   is `dropping_a_supervisor_ends_its_thread_once_its_jobs_finish`;
//! - `a_host_context_that_never_supervised_ends_at_once` is
//!   `a_supervisor_that_never_supervised_ends_at_once`;
//! - `after_shutdown_no_job_is_supervised_and_no_session_opens` keeps its
//!   first half here, as `after_shutdown_no_job_is_supervised`.

use super::*;
use crate::test_support::watch;
use std::sync::Barrier;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::Instant;

/// The bound the shutdown tests use in place of the host context's 5-second
/// cleanup bound.
const BOUND: Duration = Duration::from_millis(300);

/// How late a shutdown may return past its bound on a loaded machine.
const SLACK: Duration = Duration::from_millis(700);

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

/// Shuts `supervisor` down within `bound` from `count` threads at once, and
/// returns each count and how long it took.
fn shut_down(
    supervisor: &Arc<Supervisor>,
    bound: Duration,
    count: usize,
) -> Vec<(usize, Duration)> {
    let start = Arc::new(Barrier::new(count));
    let callers = (0..count)
        .map(|_| {
            let (supervisor, start) = (supervisor.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                let began = Instant::now();
                (supervisor.shutdown(bound), began.elapsed())
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
fn the_supervisor_crosses_threads() {
    // gwz-core's host context, which drivers share across threads, holds it.
    fn shareable<T: Send + Sync>() {}
    shareable::<Supervisor>();
    shareable::<SuperviseError>();
}

#[test]
fn dropping_a_supervisor_ends_its_thread_once_its_jobs_finish() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    assert!(!watch.running(), "no thread before the first job");
    let finished = Arc::new(AtomicBool::new(false));
    supervisor
        .supervise(Box::new(Latch(finished.clone())))
        .unwrap();
    assert!(watch.running());
    drop(supervisor);
    assert!(
        !watch.wait_ended(Duration::from_millis(100)),
        "the thread outlives its supervisor while a job runs"
    );
    finished.store(true, SeqCst);
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "the thread ends once its jobs have finished"
    );
}

#[test]
fn a_supervisor_that_never_supervised_ends_at_once() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    drop(supervisor);
    assert!(watch.wait_ended(Duration::ZERO));
    assert!(!watch.running());
}

#[test]
fn a_job_that_panics_is_quarantined_and_never_polled_again() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    let (job, polls, dropped) = always_panics();
    supervisor.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    // With only a quarantined job, the supervisor waits as with none.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(polls.load(SeqCst), 1, "polled exactly once");
    assert!(!dropped.load(SeqCst), "kept while the supervisor lives");
    drop(supervisor);
    assert!(
        watch.wait_ended(Duration::from_secs(1)),
        "only quarantined jobs remain, so the thread ends"
    );
    assert!(dropped.load(SeqCst), "the quarantined job drops as it ends");
}

#[test]
fn a_quarantined_job_does_not_stop_the_others() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    let (job, polls, _dropped) = always_panics();
    supervisor.supervise(Box::new(job)).unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    supervisor
        .supervise(Box::new(Latch(finished.clone())))
        .unwrap();
    await_first_poll(&polls);
    drop(supervisor);
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
    let supervisor = Arc::new(Supervisor::new());
    let watch = watch(&supervisor);
    let job = Counts::default();
    supervisor.supervise(job.job()).unwrap();
    let [(pending, took)] = shut_down(&supervisor, BOUND, 1).try_into().unwrap();
    assert!(took >= BOUND, "it waited for the job: {took:?}");
    assert!(took < BOUND + SLACK, "and returned at its bound: {took:?}");
    assert_eq!(pending, 1);
    // The unfinished job stays with the thread, which polls it to its end
    // and then stops, while the supervisor lives.
    job.finish.store(true, SeqCst);
    assert!(watch.wait_ended(Duration::from_secs(10)));
    assert_eq!(job.drops(), 1);
    // A second call now finds nothing left, and still returns the first
    // call's count: it disposes nothing again.
    assert_eq!(supervisor.shutdown(BOUND), 1);
    assert_eq!(job.drops(), 1);
}

#[test]
fn shutdown_returns_once_its_jobs_finish_and_releases_the_supervisor() {
    let supervisor = Arc::new(Supervisor::new());
    let watch = watch(&supervisor);
    let job = Counts::default();
    supervisor.supervise(job.job()).unwrap();
    let finish = job.finish.clone();
    let finisher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        finish.store(true, SeqCst);
    });
    let [(pending, took)] = shut_down(&supervisor, Duration::from_secs(60), 1)
        .try_into()
        .unwrap();
    finisher.join().unwrap();
    // The thread reported the finish while shutdown waited, so the wait
    // held no lock the thread needs.
    assert_eq!(pending, 0);
    assert!(took < Duration::from_secs(10), "not at its bound: {took:?}");
    assert_eq!(job.drops(), 1, "the finished job was dropped");
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "the thread stops without waiting for the supervisor's drop"
    );
}

#[test]
fn concurrent_shutdowns_wait_for_the_first_and_return_its_report() {
    let supervisor = Arc::new(Supervisor::new());
    let job = Counts::default();
    supervisor.supervise(job.job()).unwrap();
    // Six calls that each waited a bound in turn would take six bounds.
    for (pending, took) in shut_down(&supervisor, BOUND, 6) {
        assert_eq!(pending, 1);
        assert!(took < BOUND + SLACK, "{took:?}");
    }
    assert_eq!(job.drops(), 0);
    job.finish.store(true, SeqCst);
}

#[test]
fn a_quarantined_job_appears_in_the_pending_report() {
    let supervisor = Arc::new(Supervisor::new());
    let watch = watch(&supervisor);
    let (job, polls, dropped) = always_panics();
    supervisor.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    let [(pending, took)] = shut_down(&supervisor, Duration::from_secs(60), 1)
        .try_into()
        .unwrap();
    assert_eq!(pending, 1, "it never finishes, so it is pending");
    assert!(
        took < Duration::from_secs(10),
        "and not waited for: {took:?}"
    );
    assert!(
        watch.wait_ended(Duration::from_secs(10)),
        "then the thread stops, dropping it"
    );
    assert!(dropped.load(SeqCst));
    assert_eq!(polls.load(SeqCst), 1);
}

#[test]
fn a_drop_after_shutdown_disposes_nothing_more() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    let finished = Counts::default();
    finished.finish.store(true, SeqCst);
    supervisor.supervise(finished.job()).unwrap();
    let (job, polls, dropped) = always_panics();
    supervisor.supervise(Box::new(job)).unwrap();
    await_first_poll(&polls);
    assert_eq!(supervisor.shutdown(Duration::from_secs(60)), 1);
    assert!(watch.wait_ended(Duration::from_secs(10)));
    assert_eq!((finished.drops(), dropped.load(SeqCst)), (1, true));
    // What shutdown disposed is gone, so the drop finds nothing to dispose.
    let started = Instant::now();
    drop(supervisor);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it does not wait"
    );
    assert_eq!((finished.drops(), finished.polls()), (1, 1));
    assert!(watch.wait_ended(Duration::ZERO) && !watch.running());
}

#[test]
fn after_shutdown_no_job_is_supervised() {
    let supervisor = Supervisor::new();
    let watch = watch(&supervisor);
    assert_eq!(supervisor.shutdown(BOUND), 0);
    let refused = Counts::default();
    assert!(matches!(
        supervisor.supervise(refused.job()),
        Err(SuperviseError::ShutDown)
    ));
    assert_eq!(
        (refused.polls(), refused.drops()),
        (0, 1),
        "dropped unpolled, so no work starts that nothing cleans up"
    );
    assert!(!watch.running(), "and no thread starts");
    assert_eq!(supervisor.shutdown(BOUND), 0);
}
