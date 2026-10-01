//! TR2.17: a supervised job's permit returns as soon as its result is taken,
//! not at the reaper's next sweep, and the reaper returns the permit of a
//! result nobody takes.
//!
//! The permits and the reaper are the process's, so the test runs in a child
//! process of its own, where no other job takes a permit.

use super::*;
use std::{
    process::Command,
    sync::Condvar,
    task::{Context, Poll, Waker},
};

const CHILD: &str = "GWZ_TEST_PROMPT_PERMIT";

#[test]
fn a_taken_result_frees_its_permit_at_once() {
    let path = module_path!();
    let test = format!(
        "{}::taken_result_child",
        path.split_once("::").map_or(path, |(_, rest)| rest)
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .env(CHILD, "1")
        .args([
            "--ignored",
            "--exact",
            &test,
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains(&format!("test {test} ... ok")), "{stdout}");
}

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

fn permits() -> usize {
    COUNT.load(Ordering::Acquire)
}

#[test]
#[ignore = "the child process of the prompt permit test, which holds the process's reaper"]
fn taken_result_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    // A retained cleanup holds the reaper, from the sweep that joins the
    // job's thread, until the gate opens: that sweep cannot also dispose of
    // the job, so only taking its result can free its permit.
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let mut job = Job::start(None, Duration::from_secs(5), |_| Ok(7)).unwrap();
    assert_eq!(permits(), 1);
    let cell = job.cell.clone();
    Cleanup::reserve().unwrap().retain(move || {
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
    let job = Job::start(None, Duration::from_secs(5), |_| Ok(8)).unwrap();
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
