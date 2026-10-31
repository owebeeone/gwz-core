//! The member lock manager's rows (step 2.3c; core session design §5.1): a second step of one member waits and then
//! proceeds, different members never wait, a wait wakes on cancellation, a panic fails no later operation, and every
//! spelling of one workspace is one key (remediation round 1, S-1).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use gwz_session_host::{CallControls, CancellationToken};

use super::{BoundMemberLocks, MemberLockManager};
use crate::model::ErrorCode;
use crate::operation::MemberSteps;
use crate::test_support::TempDir;

/// A bound on waits that must end.
const LONG: Duration = Duration::from_secs(20);
/// A bound on a thread reaching its wait. A row that is red because nothing waits pays it once.
const PARKS: Duration = Duration::from_secs(10);

/// A workspace root that exists: the lock canonicalizes it.
fn workspace(stem: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new(stem);
    let root = temp.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    (temp, root)
}

/// A call's token and the authority to cancel it.
fn token() -> (CallControls<()>, CancellationToken) {
    let session = Arc::new(());
    let (controls, gate) = CallControls::new(&session);
    // The gate holds its session weakly; the token does not need it.
    let token = gate.token().clone();
    (controls, token)
}

/// Locks `member` of `workspace` on a new thread, which reports when it holds the lock and then releases it at once.
fn lock_on_a_thread(
    manager: &MemberLockManager,
    workspace: &Path,
    member: &'static str,
    cancel: Option<CancellationToken>,
) -> (
    mpsc::Receiver<Result<(), ErrorCode>>,
    thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let manager = manager.clone();
    let workspace = workspace.to_path_buf();
    let handle = thread::spawn(move || {
        let outcome = manager
            .lock(&workspace, member, cancel.as_ref())
            .map(drop)
            .map_err(|error| error.code);
        sender.send(outcome).unwrap();
    });
    (receiver, handle)
}

/// The row proving "it is waiting": `count` threads are parked in the manager (its own state, not a time window),
/// and none of them has answered.
fn assert_waiting(
    manager: &MemberLockManager,
    count: usize,
    answers: &[&mpsc::Receiver<Result<(), ErrorCode>>],
) {
    assert!(
        manager.wait_for_waiters(count, PARKS),
        "{count} lock call(s) should be waiting; {} are",
        manager.waiting_count()
    );
    for answer in answers {
        assert!(answer.try_recv().is_err(), "a waiting lock call answered");
    }
}

/// The row the old try-lock test was: the first lock of a member holds it, the next waits, and once the first
/// drops the member can be taken again. (The old manager refused the second; this one blocks it.)
#[test]
fn a_second_lock_of_one_member_waits_for_the_first_then_succeeds() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let first = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (second, thread) = lock_on_a_thread(&manager, &root, "mem_01", None);
    assert_waiting(&manager, 1, &[&second]);
    assert_eq!(manager.held_count(), 1);
    drop(first);
    assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.held_count(), 0);
    assert_eq!(manager.waiting_count(), 0);
    assert!(manager.lock(&root, "mem_01", None).is_ok_and(|guard| {
        drop(guard);
        true
    }));
}

#[test]
fn different_members_do_not_wait_for_each_other() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let held = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (other_member, a) = lock_on_a_thread(&manager, &root, "mem_02", None);
    assert_eq!(other_member.recv_timeout(LONG), Ok(Ok(())));
    a.join().unwrap();
    drop(held);
}

/// The control of the spelling rows: the same member id of two genuinely different workspaces is two members.
#[test]
fn two_different_workspaces_do_not_wait_for_each_other() {
    let (_temp, root) = workspace("memberlock-a");
    let (_other_temp, other) = workspace("memberlock-b");
    let manager = MemberLockManager::new();
    let held = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (elsewhere, thread) = lock_on_a_thread(&manager, &other, "mem_01", None);
    assert_eq!(elsewhere.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.waiting_count(), 0);
    drop(held);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Red row (S-1, remediation round 1): a symlink to the workspace root is the same workspace, so its lock of
        /// the same member waits. (Windows has no unprivileged symlink; its spelling row is the case variant below.)
        #[test]
        fn a_symlink_spelling_of_one_workspace_is_one_key_and_waits() {
            let (temp, root) = workspace("memberlock");
            let link = temp.path().join("ws-link");
            crate::test_support::symlink_spelling(&root, &link);
            let manager = MemberLockManager::new();
            let holder = manager.lock(&root, "mem_01", None).expect("the first lock");
            let (second, thread) = lock_on_a_thread(&manager, &link, "mem_01", None);
            assert_waiting(&manager, 1, &[&second]);
            drop(holder);
            assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
            thread.join().unwrap();
            assert_eq!(manager.held_count(), 0);
        }
    }
}

/// Red row (S-1, remediation round 1): a case variant of the root is the same workspace on a volume that folds case
/// (Windows, macOS APFS), so its lock of the same member waits. On a case-sensitive volume the variant is another
/// directory, and the row says so and does not run.
#[test]
fn a_case_variant_spelling_of_one_workspace_is_one_key_and_waits() {
    let (_temp, root) = workspace("memberlock");
    let variant = match crate::test_support::case_variant(&root) {
        Ok(variant) => variant,
        Err(reason) => {
            eprintln!("SKIPPED ROW: {reason}");
            return;
        }
    };
    let manager = MemberLockManager::new();
    let holder = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (second, thread) = lock_on_a_thread(&manager, &variant, "mem_01", None);
    assert_waiting(&manager, 1, &[&second]);
    drop(holder);
    assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.held_count(), 0);
}

/// A root that does not resolve refuses the step with the I/O error. It is never keyed by its spelling.
#[test]
fn a_workspace_that_does_not_resolve_refuses_the_step_and_holds_nothing() {
    let (temp, _root) = workspace("memberlock");
    let missing = temp.path().join("never-created");
    let manager = MemberLockManager::new();
    let error = manager
        .lock(&missing, "mem_01", None)
        .map(drop)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::IoError);
    assert!(
        error.message.contains("never-created"),
        "the refusal should name the root: {}",
        error.message
    );
    assert_eq!(manager.held_count(), 0);
    assert_eq!(manager.waiting_count(), 0);
}

#[test]
fn a_wait_wakes_on_cancel_and_refuses_with_cancelled() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let holder = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (controls, cancel) = token();
    let (waiting, thread) = lock_on_a_thread(&manager, &root, "mem_01", Some(cancel));
    assert_waiting(&manager, 1, &[&waiting]);
    controls.cancel();
    assert_eq!(
        waiting.recv_timeout(LONG),
        Ok(Err(ErrorCode::Cancelled)),
        "the wait did not wake on the cancel"
    );
    thread.join().unwrap();
    // The cancelled wait took nothing: the holder still holds, and the member frees when it drops.
    assert_eq!(manager.held_count(), 1);
    assert_eq!(manager.waiting_count(), 0);
    drop(holder);
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_cancel_wakes_only_the_wait_of_its_own_token() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let holder = manager.lock(&root, "mem_01", None).expect("the first lock");
    let (controls, cancel) = token();
    let (cancelled, a) = lock_on_a_thread(&manager, &root, "mem_01", Some(cancel));
    let (other, b) = lock_on_a_thread(&manager, &root, "mem_01", None);
    assert_waiting(&manager, 2, &[&cancelled, &other]);
    controls.cancel();
    assert_eq!(cancelled.recv_timeout(LONG), Ok(Err(ErrorCode::Cancelled)));
    // The cancelled wait has left the count before it answered, so the other is the one still parked.
    assert_waiting(&manager, 1, &[&other]);
    assert_eq!(
        manager.waiting_count(),
        1,
        "the other wait ended with the cancel"
    );
    drop(holder);
    assert_eq!(other.recv_timeout(LONG), Ok(Ok(())));
    a.join().unwrap();
    b.join().unwrap();
}

#[test]
fn a_token_already_cancelled_refuses_before_any_wait_even_for_a_free_member() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let (controls, cancel) = token();
    controls.cancel();
    let error = manager
        .lock(&root, "mem_01", Some(&cancel))
        .map(drop)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(manager.held_count(), 0, "a refused lock holds nothing");
}

/// A call already cancelled answers `cancelled` even when its root would not resolve: the token is looked at first.
#[test]
fn a_token_already_cancelled_answers_cancelled_before_the_root_is_looked_at() {
    let (temp, _root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let (controls, cancel) = token();
    controls.cancel();
    let error = manager
        .lock(&temp.path().join("never-created"), "mem_01", Some(&cancel))
        .map(drop)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
}

#[test]
fn a_cancel_that_arrives_as_the_member_frees_leaves_nothing_held() {
    // The holder drops while the waiter's token is cancelled: either outcome is allowed, a held member is not.
    let (_temp, root) = workspace("memberlock");
    for _ in 0..50 {
        let manager = MemberLockManager::new();
        let holder = manager.lock(&root, "mem_01", None).expect("the first lock");
        let (controls, cancel) = token();
        let (waiting, thread) = lock_on_a_thread(&manager, &root, "mem_01", Some(cancel));
        assert_waiting(&manager, 1, &[&waiting]);
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
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let panicking = manager.clone();
    let panicking_root = root.clone();
    let result = thread::spawn(move || {
        let _guard = panicking
            .lock(&panicking_root, "mem_01", None)
            .expect("the lock");
        panic!("a fetch step panicked while holding its member");
    })
    .join();
    assert!(result.is_err(), "the thread should have panicked");
    assert_eq!(manager.held_count(), 0, "the unwinding guard released");
    let again = manager.lock(&root, "mem_01", None);
    assert!(again.is_ok(), "a later operation failed after the panic");
}

#[test]
fn a_manager_whose_lock_was_poisoned_still_locks_and_releases() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    manager.poison();
    let guard = manager
        .lock(&root, "mem_01", None)
        .expect("a poisoned manager still locks");
    assert_eq!(manager.held_count(), 1);
    let (second, thread) = lock_on_a_thread(&manager, &root, "mem_01", None);
    assert_waiting(&manager, 1, &[&second]);
    drop(guard);
    assert_eq!(second.recv_timeout(LONG), Ok(Ok(())));
    thread.join().unwrap();
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_handler_port_without_a_token_waits_and_with_one_is_cancellable() {
    let (_temp, root) = workspace("memberlock");
    let manager = MemberLockManager::new();
    let holder = manager.lock(&root, "@root", None).expect("the first lock");
    let (controls, cancel) = token();
    let bound = Arc::new(BoundMemberLocks::new(manager.clone(), cancel));
    let waiting = Arc::clone(&bound);
    let (sender, receiver) = mpsc::channel();
    let waiting_root = root.clone();
    let thread = thread::spawn(move || {
        let outcome = waiting
            .enter(&waiting_root, "@root")
            .map(drop)
            .map_err(|error| error.code);
        sender.send(outcome).unwrap();
    });
    assert!(
        manager.wait_for_waiters(1, PARKS),
        "the port's enter should be waiting"
    );
    assert!(receiver.try_recv().is_err());
    controls.cancel();
    assert_eq!(receiver.recv_timeout(LONG), Ok(Err(ErrorCode::Cancelled)));
    thread.join().unwrap();
    drop(holder);

    // A direct caller's port has a manager of its own call and no token: it never refuses and never waits on others.
    // The two members are taken in sequence, so a manager that confuses them fails this row in bounded time instead
    // of waiting on itself.
    let per_call = BoundMemberLocks::per_call();
    drop(per_call.enter(&root, "@root").unwrap());
    drop(per_call.enter(&root, "mem_01").unwrap());
}
