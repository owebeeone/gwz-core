#![allow(dead_code)]
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[test]
fn retained_cleanup_slots_are_bounded_and_progress_without_live_helpers() {
    let ready = Arc::new(AtomicBool::new(false));
    let complete = Arc::new(AtomicUsize::new(0));
    let slots: Vec<_> = (0..64)
        .map(|_| agent_job::Cleanup::reserve().unwrap())
        .collect();
    assert!(
        matches!(agent_job::Cleanup::reserve(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
    );
    for slot in slots {
        let ready = ready.clone();
        let complete = complete.clone();
        slot.retain(move || {
            if ready.load(Ordering::SeqCst) {
                complete.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        });
    }
    assert!(
        matches!(agent_job::Cleanup::reserve(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
    );
    std::thread::sleep(Duration::from_millis(60));
    ready.store(true, Ordering::SeqCst); // no helper and no unpark
    let deadline = Instant::now() + Duration::from_secs(3);
    while complete.load(Ordering::SeqCst) != 64 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    // The last callback can publish completion immediately before its permit
    // is dropped. Wait for that destruction rather than racing the callback.
    loop {
        match agent_job::Cleanup::reserve() {
            Ok(_next) => break,
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
