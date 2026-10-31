//! The member lock manager's rows (step 2.3c; core session design §5.1): a second step of one member waits and then
//! proceeds, different members never wait, a wait wakes on cancellation, and a panic fails no later operation.

use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use gwz_session_host::{CallControls, CancellationToken};

use super::{BoundMemberLocks, MemberLockManager};
use crate::model::ErrorCode;
use crate::operation::MemberSteps;

/// Long enough that a thread which was going to get a lock has got it.
const SETTLE: Duration = Duration::from_millis(150);
/// A bound on waits that must end.
const LONG: Duration = Duration::from_secs(20);

const WORKSPACE: &str = "/ws";

/// A call's token and the authority to cancel it.
fn token() -> (CallControls<()>, CancellationToken) {
    let session = Arc::new(());
    let (controls, gate) = CallControls::new(&session);
    // The gate holds its session weakly; the token does not need it.
    let token = gate.token().clone();
    (controls, token)
}

/// Locks `member` on a new thread, which reports when it holds the lock and then releases it at once.
fn lock_on_a_thread(
    manager: &MemberLockManager,
    member: &'static str,
    cancel: Option<CancellationToken>,
) -> (
    mpsc::Receiver<Result<(), ErrorCode>>,
    thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let manager = manager.clone();
    let handle = thread::spawn(move || {
        let outcome = manager
            .lock(Path::new(WORKSPACE), member, cancel.as_ref())
            .map(drop)
            .map_err(|error| error.code);
        sender.send(outcome).unwrap();
    });
    (receiver, handle)
}

/// The row the old try-lock test was: the first lock of a member holds it, the next waits, and once the first
/// drops the member can be taken again. (The old manager refused the second; this one blocks it.)
#[test]
fn a_second_lock_of_one_member_waits_for_the_first_then_succeeds() {
    let manager = MemberLockManager::new();
    let first = manager
        .lock(Path::new(WORKSPACE), "mem_01", None)
        .expect("the first lock");
    let (second, thread) = lock_on_a_thread(&manager, "mem_01", None);
    assert!(
        second.recv_timeout(SETTLE).is_err(),
        "the second lock did not wait for the first"
    );
    assert_eq!(manager.held_count(), 1);
    drop(first);
    assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.held_count(), 0);
    assert!(
        manager
            .lock(Path::new(WORKSPACE), "mem_01", None)
            .is_ok_and(|guard| {
                drop(guard);
                true
            })
    );
}

#[test]
fn different_members_do_not_wait_for_each_other() {
    let manager = MemberLockManager::new();
    let held = manager
        .lock(Path::new(WORKSPACE), "mem_01", None)
        .expect("the first lock");
    let (other_member, a) = lock_on_a_thread(&manager, "mem_02", None);
    assert_eq!(other_member.recv_timeout(LONG), Ok(Ok(())));
    a.join().unwrap();
    // The same member id of another workspace is another member.
    let manager_clone = manager.clone();
    let (sender, receiver) = mpsc::channel();
    let b = thread::spawn(move || {
        let guard = manager_clone.lock(Path::new("/elsewhere"), "mem_01", None);
        sender.send(guard.is_ok()).unwrap();
    });
    assert_eq!(receiver.recv_timeout(LONG), Ok(true));
    b.join().unwrap();
    drop(held);
}

#[test]
fn a_wait_wakes_on_cancel_and_refuses_with_cancelled() {
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(Path::new(WORKSPACE), "mem_01", None)
        .expect("the first lock");
    let (controls, cancel) = token();
    let (waiting, thread) = lock_on_a_thread(&manager, "mem_01", Some(cancel));
    assert!(
        waiting.recv_timeout(SETTLE).is_err(),
        "it should be waiting"
    );
    controls.cancel();
    assert_eq!(
        waiting.recv_timeout(LONG),
        Ok(Err(ErrorCode::Cancelled)),
        "the wait did not wake on the cancel"
    );
    thread.join().unwrap();
    // The cancelled wait took nothing: the holder still holds, and the member frees when it drops.
    assert_eq!(manager.held_count(), 1);
    drop(holder);
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_cancel_wakes_only_the_wait_of_its_own_token() {
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(Path::new(WORKSPACE), "mem_01", None)
        .expect("the first lock");
    let (controls, cancel) = token();
    let (cancelled, a) = lock_on_a_thread(&manager, "mem_01", Some(cancel));
    let (other, b) = lock_on_a_thread(&manager, "mem_01", None);
    assert!(cancelled.recv_timeout(SETTLE).is_err());
    controls.cancel();
    assert_eq!(cancelled.recv_timeout(LONG), Ok(Err(ErrorCode::Cancelled)));
    assert!(
        other.recv_timeout(SETTLE).is_err(),
        "the other wait ended with the cancel"
    );
    drop(holder);
    assert_eq!(other.recv_timeout(LONG), Ok(Ok(())));
    a.join().unwrap();
    b.join().unwrap();
}

#[test]
fn a_token_already_cancelled_refuses_before_any_wait_even_for_a_free_member() {
    let manager = MemberLockManager::new();
    let (controls, cancel) = token();
    controls.cancel();
    let error = manager
        .lock(Path::new(WORKSPACE), "mem_01", Some(&cancel))
        .map(drop)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(manager.held_count(), 0, "a refused lock holds nothing");
}

#[test]
fn a_cancel_that_arrives_as_the_member_frees_leaves_nothing_held() {
    // The holder drops while the waiter's token is cancelled: either outcome is allowed, a held member is not.
    for _ in 0..50 {
        let manager = MemberLockManager::new();
        let holder = manager
            .lock(Path::new(WORKSPACE), "mem_01", None)
            .expect("the first lock");
        let (controls, cancel) = token();
        let (waiting, thread) = lock_on_a_thread(&manager, "mem_01", Some(cancel));
        let barrier = Arc::new(Barrier::new(2));
        let at = Arc::clone(&barrier);
        let canceller = thread::spawn(move || {
            at.wait();
            controls.cancel();
        });
        barrier.wait();
        drop(holder);
        let outcome = waiting.recv_timeout(LONG).expect("the wait ended");
        assert!(outcome == Ok(()) || outcome == Err(ErrorCode::Cancelled));
        canceller.join().unwrap();
        thread.join().unwrap();
        assert_eq!(manager.held_count(), 0);
    }
}

#[test]
fn a_panic_while_holding_a_guard_does_not_fail_later_operations() {
    let manager = MemberLockManager::new();
    let panicking = manager.clone();
    let result = thread::spawn(move || {
        let _guard = panicking
            .lock(Path::new(WORKSPACE), "mem_01", None)
            .expect("the lock");
        panic!("a fetch step panicked while holding its member");
    })
    .join();
    assert!(result.is_err(), "the thread should have panicked");
    assert_eq!(manager.held_count(), 0, "the unwinding guard released");
    let again = manager.lock(Path::new(WORKSPACE), "mem_01", None);
    assert!(again.is_ok(), "a later operation failed after the panic");
}

#[test]
fn a_manager_whose_lock_was_poisoned_still_locks_and_releases() {
    let manager = MemberLockManager::new();
    manager.poison();
    let guard = manager
        .lock(Path::new(WORKSPACE), "mem_01", None)
        .expect("a poisoned manager still locks");
    assert_eq!(manager.held_count(), 1);
    let (second, thread) = lock_on_a_thread(&manager, "mem_01", None);
    assert!(second.recv_timeout(SETTLE).is_err());
    drop(guard);
    assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_handler_port_without_a_token_waits_and_with_one_is_cancellable() {
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(Path::new(WORKSPACE), "@root", None)
        .expect("the first lock");
    let (controls, cancel) = token();
    let bound = Arc::new(BoundMemberLocks::new(manager.clone(), cancel));
    let waiting = Arc::clone(&bound);
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let outcome = waiting
            .enter(Path::new(WORKSPACE), "@root")
            .map(drop)
            .map_err(|error| error.code);
        sender.send(outcome).unwrap();
    });
    assert!(receiver.recv_timeout(SETTLE).is_err());
    controls.cancel();
    assert_eq!(receiver.recv_timeout(LONG), Ok(Err(ErrorCode::Cancelled)));
    thread.join().unwrap();
    drop(holder);

    // A direct caller's port has a manager of its own call and no token: it never refuses and never waits on others.
    let per_call = BoundMemberLocks::per_call();
    let first = per_call.enter(Path::new(WORKSPACE), "@root").unwrap();
    let other = per_call.enter(Path::new(WORKSPACE), "mem_01").unwrap();
    drop((first, other));
}
