//! TR2.17: a supervised job's permit returns as soon as its result is taken,
//! not at the reaper's next sweep, and the reaper returns the permit of a
//! result nobody takes.
//!
//! The permits and the reaper are the supervisor's, so the test has one of
//! its own and no other job takes a permit from it.

use super::*;
use std::{
    sync::Condvar,
    task::{Context, Poll, Waker},
};

/// Opens the gate it holds when dropped, however the test ends.
struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        let (open, opened) = &*self.0;
        *open.lock().unwrap_or_else(|e| e.into_inner()) = true;
        opened.notify_all();
    }
}

fn wait_result<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
    let mut cx = Context::from_waker(Waker::noop());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut cx) {
            return result;
        }
        assert!(Instant::now() < deadline, "the job's result never came");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_taken_result_frees_its_permit_at_once() {
    let supervisor = Supervisor::new();
    let permits = || supervisor.taken();
    // A retained cleanup holds the reaper, from the sweep that joins the
    // job's thread, until the gate opens: that sweep cannot also dispose of
    // the job, so only taking its result can free its permit.
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let mut job = Job::start(&supervisor, None, Duration::from_secs(5), |_| Ok(7)).unwrap();
    assert_eq!(permits(), 1);
    let cell = job.cell.clone();
    Cleanup::reserve(&supervisor).unwrap().retain(move || {
        let state = cell.control.state.lock();
        if !state.unwrap_or_else(|e| e.into_inner()).joined {
            return false;
        }
        let (open, opened) = &*gate;
        let mut open = open.lock().unwrap_or_else(|e| e.into_inner());
        while !*open {
            open = opened.wait(open).unwrap_or_else(|e| e.into_inner());
        }
        true
    });
    assert_eq!(wait_result(&mut job).unwrap(), 7);
    assert_eq!(permits(), 0, "the taken result's permit is still held");
    drop(release);

    // A result nobody takes: the reaper disposes of it, and returns its
    // permit then.
    let job = Job::start(&supervisor, None, Duration::from_secs(5), |_| Ok(8)).unwrap();
    assert_eq!(permits(), 1);
    drop(job);
    let deadline = Instant::now() + Duration::from_secs(5);
    while permits() != 0 {
        assert!(
            Instant::now() < deadline,
            "the reaper never returned the abandoned job's permit"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// The budget belongs to the host that made the supervisor (OQ9): a full
/// budget in one host leaves another's untouched, and every clone of a
/// supervisor, the handle its endpoints hold, shares one budget.
#[test]
fn a_budget_is_per_supervisor_and_clones_share_it() {
    let first = Supervisor::new();
    let second = Supervisor::new();
    let shared = first.clone();
    let waker = Waker::noop();
    let held: Vec<_> = (0..LIMIT)
        .map(|_| first.reserve(waker).expect("a place"))
        .collect();
    assert_eq!(first.taken(), LIMIT);
    assert!(matches!(
        shared.reserve(waker),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));
    assert_eq!(second.taken(), 0);
    let other = second
        .reserve(waker)
        .expect("another host's budget is free");
    assert_eq!(second.taken(), 1);
    drop(other);
    drop(held);
    assert_eq!(first.taken(), 0);
    assert!(shared.reserve(waker).is_ok());
}

/// A setup waiting for a place is woken by the release that frees one, and
/// only by a release in its own host.
#[test]
fn a_released_place_wakes_only_its_own_supervisors_waiters() {
    struct Count(std::sync::atomic::AtomicUsize);
    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let first = Supervisor::new();
    let second = Supervisor::new();
    let held: Vec<_> = (0..LIMIT)
        .map(|_| first.reserve(Waker::noop()).unwrap())
        .collect();
    let woken = Arc::new(Count(std::sync::atomic::AtomicUsize::new(0)));
    let waker = Waker::from(woken.clone());
    assert!(first.reserve(&waker).is_err());
    drop(second.reserve(Waker::noop()).unwrap());
    assert_eq!(woken.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    drop(held);
    assert_eq!(woken.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}
