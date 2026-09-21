#![allow(dead_code)]
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
use agent_job::Job;
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Owned(Arc<AtomicUsize>);
impl Drop for Owned {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn global_capacity_counts_abandoned_helpers_until_join_and_disposal() {
    // The first supervisor creation fails; concurrent retries publish exactly one hub.
    let failed = Job::start_with(
        None,
        Duration::from_secs(1),
        |_| Ok(()),
        |name, _| {
            assert_eq!(name, "gwz-setup-reaper");
            Err(io::ErrorKind::Other.into())
        },
    );
    assert!(matches!(failed, Err(e) if e.kind() == io::ErrorKind::Other));
    let supervisors = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut callers = Vec::new();
    for _ in 0..8 {
        let supervisors = supervisors.clone();
        let barrier = barrier.clone();
        callers.push(std::thread::spawn(move || {
            barrier.wait();
            let mut job = Job::start_with(
                None,
                Duration::from_secs(1),
                |_| Ok(()),
                |name, body| {
                    if name == "gwz-setup-reaper" {
                        supervisors.fetch_add(1, Ordering::SeqCst);
                    }
                    std::thread::Builder::new().name(name.into()).spawn(body)
                },
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let std::task::Poll::Ready(result) =
                    job.poll_result(&mut std::task::Context::from_waker(std::task::Waker::noop()))
                {
                    result.unwrap();
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }));
    }
    for caller in callers {
        caller.join().unwrap();
    }
    assert_eq!(supervisors.load(Ordering::SeqCst), 1);
    let invoked = Arc::new(AtomicUsize::new(0));
    for _ in 0..65 {
        let invoked = invoked.clone();
        let failed = Job::start_with(
            None,
            Duration::from_secs(1),
            move |_| {
                invoked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_, _| Err(io::ErrorKind::Other.into()),
        );
        assert!(matches!(failed, Err(e) if e.kind() == io::ErrorKind::Other));
    }
    assert_eq!(invoked.load(Ordering::SeqCst), 0);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let disposed = Arc::new(AtomicUsize::new(0));
    let mut jobs = Vec::new();
    let (started, running) = std::sync::mpsc::channel();
    for _ in 0..64 {
        let gate = gate.clone();
        let disposed = disposed.clone();
        let started = started.clone();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let gate = gate.clone();
            let disposed = disposed.clone();
            let started = started.clone();
            match Job::start(None, Duration::from_millis(10), move |_| {
                started.send(()).unwrap();
                let (lock, ready) = &*gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = ready.wait(open).unwrap();
                }
                Ok(Owned(disposed))
            }) {
                Ok(job) => {
                    jobs.push(job);
                    break;
                }
                Err(e) => {
                    assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
            }
        }
    }
    for _ in 0..64 {
        running.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    let full = || {
        assert!(
            matches!(Job::start(None, Duration::from_secs(1), |_| Ok(())), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
        )
    };
    let checks = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        full();
        jobs.clear(); // caller/endpoint recreation cannot bypass live helper permits
        std::thread::sleep(Duration::from_millis(50));
        full();
        assert_eq!(disposed.load(Ordering::SeqCst), 0);
    }));
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let deadline = Instant::now() + Duration::from_secs(5);
    while disposed.load(Ordering::SeqCst) != 64 {
        assert!(
            Instant::now() < deadline,
            "abandoned results were not disposed"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Fresh facade access can allocate after the old jobs have been reaped.
    let mut next = loop {
        match Job::start(None, Duration::from_secs(1), |_| Ok(())) {
            Ok(job) => break job,
            Err(e) => {
                assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    loop {
        if let std::task::Poll::Ready(result) =
            next.poll_result(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        {
            result.unwrap();
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    if let Err(panic) = checks {
        std::panic::resume_unwind(panic);
    }
}
