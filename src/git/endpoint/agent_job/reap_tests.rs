//! The reaper's hand-off of a finished job. A job's thread stores its result,
//! wakes the reaper and exits, and the reaper can look at it before the exit is
//! done. It used to leave such a job for its next tick, 20 ms later, which was
//! the whole fixed cost of a Linux HTTPS command whenever the race went the
//! wrong way (gwz-core-evidence, `2026-10-13-https-fixed-cost-pi`). The thread
//! is a stand-in here that has not exited, so the outcome does not depend on
//! how fast a real one does.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A job thread that has or has not exited, and counts the waits for it.
struct Thread {
    exited: bool,
    joins: Arc<AtomicUsize>,
}
impl Exit for Thread {
    fn exited(&self) -> bool {
        self.exited
    }
    fn join(self) -> bool {
        self.joins.fetch_add(1, Ordering::SeqCst);
        false
    }
}

/// An entry whose job has (`stored`) or has not stored its result, and whose
/// thread has (`exited`) or has not exited.
struct Reaping {
    entry: Entry<(), Thread>,
    control: Arc<Control>,
    joins: Arc<AtomicUsize>,
}
fn reaping(stored: bool, exited: bool) -> Reaping {
    let clock = ManualClock::new();
    let control = Control::scripted(None, Duration::ZERO, Duration::from_secs(5), clock.clock());
    let joins = Arc::new(AtomicUsize::new(0));
    let cell = Arc::new(Cell {
        control: control.clone(),
        result: Mutex::new(stored.then_some(Ok(()))),
        permit: Mutex::new(None),
    });
    let thread = Thread {
        exited,
        joins: joins.clone(),
    };
    Reaping {
        entry: Entry {
            cell,
            join: Some(thread),
        },
        control,
        joins,
    }
}
fn joined(control: &Control) -> bool {
    control
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .joined
}

#[test]
fn a_job_with_its_result_stored_is_joined_while_its_thread_is_still_exiting() {
    let mut job = reaping(true, false);
    assert!(!job.entry.reap(), "the result is not yet taken");
    assert_eq!(
        job.joins.load(Ordering::SeqCst),
        1,
        "the thread is waited for"
    );
    assert!(
        joined(&job.control),
        "so the job is joined in this pass, not the next tick"
    );
}

#[test]
fn a_job_whose_thread_has_exited_is_joined() {
    let mut job = reaping(true, true);
    assert!(!job.entry.reap());
    assert_eq!(job.joins.load(Ordering::SeqCst), 1);
    assert!(joined(&job.control));
}

#[test]
fn a_job_still_working_is_never_waited_for() {
    let mut job = reaping(false, false);
    assert!(!job.entry.reap());
    assert_eq!(
        job.joins.load(Ordering::SeqCst),
        0,
        "the reaper does not block on running work"
    );
    assert!(!joined(&job.control));
}
