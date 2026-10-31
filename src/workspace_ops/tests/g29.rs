//! The member lock around each fetch and push member step (step 2.3c; core session design §5.1): the handlers take
//! the port's guard around a member's exchange and ref update, so two steps of one member in one host context
//! never overlap, different members still do, and a cancelled call's wait fails that member's row.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gwz_session_host::{CallControls, CancellationToken};

use crate::operation::NullSink;
use crate::operation::{BoundMemberLocks, MemberLockManager};

use super::*;

use super::g01::tracking_backend::{ConfiguredRemote, TEST_COMMIT, TrackingBackend};

const LONG: Duration = Duration::from_secs(30);

const APP: (&str, &str, &str, &str) = (
    "mem_app",
    "repos/app",
    "ssh://one.invalid/app.git",
    TEST_COMMIT,
);
const LIB: (&str, &str, &str, &str) = (
    "mem_lib",
    "repos/lib",
    "ssh://two.invalid/lib.git",
    TEST_COMMIT,
);

fn token() -> (CallControls<()>, CancellationToken) {
    let session = Arc::new(());
    let (controls, gate) = CallControls::new(&session);
    let token = gate.token().clone();
    (controls, token)
}

/// A workspace of `members` over a double. With `attached`, each member is on `main` with an `origin` remote, which
/// is what a fetch of it needs; a push needs the double's defaults, so that its plan is not already on origin.
fn workspace(
    name: &str,
    expected_overlap: usize,
    members: Vec<(&str, &str, &str, &str)>,
    attached: bool,
) -> (TempDir, TrackingBackend) {
    let temp = TempDir::new(name);
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let backend = TrackingBackend::new(expected_overlap);
    for (_, path, url, _) in &members {
        if attached {
            let repository = temp.path().join(path);
            backend.set_head(&repository, "main", TEST_COMMIT);
            backend.set_materialized(&repository, true);
            backend.add_remote_config(&repository, ConfiguredRemote::new("origin", url));
            backend.set_remote_tracking(&repository, "refs/remotes/origin/main", TEST_COMMIT);
        }
    }
    write_pull_fixture(temp.path(), members);
    (temp, backend)
}

/// A fetch of exactly `members` (the root is not fetched).
fn fetch_request(members: &[&str]) -> crate::FetchRequest {
    crate::FetchRequest {
        meta: crate::RequestMeta {
            selection: Some(crate::Selection {
                targets: members.iter().map(|member| (*member).to_owned()).collect(),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
    }
}

fn fetch(
    backend: &TrackingBackend,
    root: &Path,
    members: &[&str],
    op: &str,
    steps: &BoundMemberLocks,
) -> crate::FetchResponse {
    handle_fetch_with_events_in(backend, root, fetch_request(members), op, &NullSink, steps)
        .unwrap()
}

fn push(
    backend: &TrackingBackend,
    root: &Path,
    op: &str,
    steps: &BoundMemberLocks,
) -> crate::PushResponse {
    let world = crate::operation_context::TestWorld::physical();
    handle_push_with_events_in(
        &world.context(),
        backend,
        root,
        push_request(None, None),
        op,
        &NullSink,
        steps,
    )
    .unwrap()
}

fn statuses(members: &[crate::MemberResponse]) -> Vec<(String, crate::MemberStatus)> {
    members
        .iter()
        .map(|row| (row.member_id.clone(), row.status))
        .collect()
}

/// Two fetches of one member in one host context: the second waits for the first, and both succeed. The double
/// holds each fetch until two have entered (or two seconds pass), so overlap is what a missing lock shows.
#[test]
fn a_fetch_beside_a_fetch_of_one_member_waits_then_both_succeed() {
    let (temp, backend) = workspace("lock-fetch-fetch", 2, vec![APP], true);
    let manager = MemberLockManager::new();
    let (_controls, cancel) = token();
    let steps = Arc::new(BoundMemberLocks::new(manager.clone(), cancel));
    let responses: Vec<_> = (0..2)
        .map(|n| {
            let (backend, steps) = (backend.clone(), Arc::clone(&steps));
            let root = temp.path().to_path_buf();
            thread::spawn(move || {
                fetch(
                    &backend,
                    &root,
                    &["mem_app"],
                    &format!("op_fetch_{n}"),
                    &steps,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    for response in &responses {
        assert_eq!(
            statuses(&response.response.members),
            vec![("mem_app".to_owned(), crate::MemberStatus::Noop)],
            "{:?}",
            response.response.members
        );
    }
    assert_eq!(
        backend.fetch_peak(),
        1,
        "two fetches of one member ran at once"
    );
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn fetches_of_different_members_still_overlap() {
    let (temp, backend) = workspace("lock-fetch-members", 2, vec![APP, LIB], true);
    let manager = MemberLockManager::new();
    let (_controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let response = fetch(
        &backend,
        temp.path(),
        &["mem_app", "mem_lib"],
        "op_fetch",
        &steps,
    );
    assert_eq!(response.response.members.len(), 2);
    assert_eq!(
        backend.fetch_peak(),
        2,
        "the member lock serialised two different members"
    );
    assert_eq!(manager.held_count(), 0);
}

/// The workspace root as the handlers resolve it, which is what keys their member locks.
fn resolved_root(temp: &TempDir) -> std::path::PathBuf {
    resolve_request_workspace_root(temp.path(), &request_meta_with_workspace()).unwrap()
}

/// Runs `step` on a thread and returns its result channel.
fn on_a_thread<R: Send + 'static>(
    step: impl FnOnce() -> R + Send + 'static,
) -> (mpsc::Receiver<R>, thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        sender.send(step()).unwrap();
    });
    (receiver, handle)
}

/// A fetch of a member whose lock another step holds is not entered until that step lets go, and then runs.
#[test]
fn a_fetch_waits_for_the_member_another_step_holds() {
    let (temp, backend) = workspace("lock-fetch-held", 1, vec![APP], true);
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(&resolved_root(&temp), "mem_app", None)
        .expect("the member is free");
    let (_controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let (result, thread) = on_a_thread({
        let (backend, root) = (backend.clone(), temp.path().to_path_buf());
        move || fetch(&backend, &root, &["mem_app"], "op_fetch", &steps)
    });
    manager.assert_waiting(&result, "the fetch did not wait for the member's holder");
    assert_eq!(backend.fetch_peak(), 0, "the fetch ran while it was held");
    drop(holder);
    let response = result.recv_timeout(LONG).expect("the fetch ended");
    thread.join().unwrap();
    assert_eq!(
        statuses(&response.response.members),
        vec![("mem_app".to_owned(), crate::MemberStatus::Noop)]
    );
    assert_eq!(backend.fetch_peak(), 1);
}

/// A push takes the member's guard around its exchange: it does not run while a fetch (or another session's step)
/// holds the member, and runs when it lets go.
#[test]
fn a_push_waits_for_the_member_another_step_holds() {
    let (temp, backend) = workspace("lock-push-held", 1, vec![APP], false);
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(&resolved_root(&temp), "mem_app", None)
        .expect("the member is free");
    let (_controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let (result, thread) = on_a_thread({
        let (backend, root) = (backend.clone(), temp.path().to_path_buf());
        move || push(&backend, &root, "op_push", &steps)
    });
    manager.assert_waiting(&result, "the push did not wait for the member's holder");
    assert_eq!(backend.push_peak(), 0, "the push ran while it was held");
    drop(holder);
    let response = result.recv_timeout(LONG).expect("the push ended");
    thread.join().unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(backend.push_peak(), 1);
    assert_eq!(manager.held_count(), 0);
}

/// A cancelled call stops waiting for its member: that member's row fails with `cancelled`, the exchange never
/// runs, and the holder keeps the member.
#[test]
fn a_cancelled_fetch_wait_fails_its_members_row_with_cancelled() {
    let (temp, backend) = workspace("lock-fetch-cancel", 1, vec![APP], true);
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(&resolved_root(&temp), "mem_app", None)
        .expect("the member is free");
    let (controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let (result, thread) = on_a_thread({
        let (backend, root) = (backend.clone(), temp.path().to_path_buf());
        move || fetch(&backend, &root, &["mem_app"], "op_fetch", &steps)
    });
    manager.assert_waiting(&result, "it should be waiting");
    controls.cancel();
    let response = result
        .recv_timeout(LONG)
        .expect("the wait did not wake on the cancel");
    thread.join().unwrap();
    let row = &response.response.members[0];
    assert_eq!(row.status, crate::MemberStatus::Failed);
    assert_eq!(
        row.error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::Cancelled)
    );
    assert_eq!(backend.fetch_peak(), 0, "a cancelled wait still fetched");
    assert_eq!(manager.held_count(), 1, "the holder lost its member");
    drop(holder);
}

#[test]
fn a_cancelled_push_wait_fails_its_members_row_with_cancelled() {
    let (temp, backend) = workspace("lock-push-cancel", 1, vec![APP], false);
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(&resolved_root(&temp), "mem_app", None)
        .expect("the member is free");
    let (controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let (result, thread) = on_a_thread({
        let (backend, root) = (backend.clone(), temp.path().to_path_buf());
        move || push(&backend, &root, "op_push", &steps)
    });
    manager.assert_waiting(&result, "it should be waiting");
    controls.cancel();
    let response = result
        .recv_timeout(LONG)
        .expect("the wait did not wake on the cancel");
    thread.join().unwrap();
    let row = response
        .response
        .members
        .iter()
        .find(|row| row.member_id == "mem_app")
        .expect("the member's row");
    assert_eq!(row.status, crate::MemberStatus::Failed);
    assert_eq!(
        row.error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::Cancelled)
    );
    assert_eq!(backend.push_peak(), 0, "a cancelled wait still pushed");
    drop(holder);
}

fn pull_request(members: &[&str]) -> crate::PullHeadRequest {
    crate::PullHeadRequest {
        meta: crate::RequestMeta {
            selection: Some(crate::Selection {
                targets: members.iter().map(|member| (*member).to_owned()).collect(),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
    }
}

fn pull(
    backend: &TrackingBackend,
    root: &Path,
    members: &[&str],
    op: &str,
    steps: &BoundMemberLocks,
) -> crate::PullHeadResponse {
    let world = crate::operation_context::TestWorld::physical();
    handle_pull_head_with_events_in(
        &world.context(),
        backend,
        root,
        pull_request(members),
        op,
        &NullSink,
        steps,
    )
    .unwrap()
}

/// A pull's preflight fetch of a member beside a fetch of the same member: the second waits for the first, and both
/// succeed. The double holds each fetch until two have entered (or two seconds pass), so overlap is what a missing
/// lock shows.
#[test]
fn a_pulls_preflight_fetch_beside_a_fetch_of_one_member_waits_then_both_succeed() {
    let (temp, backend) = workspace("lock-pull-fetch", 2, vec![APP], true);
    let manager = MemberLockManager::new();
    let (_controls, cancel) = token();
    let steps = Arc::new(BoundMemberLocks::new(manager.clone(), cancel));
    let pulling = {
        let (backend, steps, root) = (
            backend.clone(),
            Arc::clone(&steps),
            temp.path().to_path_buf(),
        );
        thread::spawn(move || pull(&backend, &root, &["mem_app"], "op_pull", &steps))
    };
    let fetching = {
        let (backend, steps, root) = (
            backend.clone(),
            Arc::clone(&steps),
            temp.path().to_path_buf(),
        );
        thread::spawn(move || fetch(&backend, &root, &["mem_app"], "op_fetch", &steps))
    };
    let pulled = pulling.join().unwrap();
    let fetched = fetching.join().unwrap();
    assert_eq!(
        pulled.response.meta.aggregate_status,
        crate::AggregateStatus::Noop,
        "{:?}",
        pulled.response
    );
    assert_eq!(
        statuses(&fetched.response.members),
        vec![("mem_app".to_owned(), crate::MemberStatus::Noop)]
    );
    assert_eq!(
        backend.fetch_peak(),
        1,
        "a pull's fetch and a fetch of one member ran at once"
    );
    assert_eq!(manager.held_count(), 0);
}

/// A pull's preflight fetch waits for a member another step holds, and a cancel ends the wait with `cancelled`
/// before anything was fetched.
#[test]
fn a_pulls_preflight_fetch_waits_for_the_member_another_step_holds_and_wakes_on_cancel() {
    let (temp, backend) = workspace("lock-pull-held", 1, vec![APP], true);
    let manager = MemberLockManager::new();
    let holder = manager
        .lock(&resolved_root(&temp), "mem_app", None)
        .expect("the member is free");
    let (controls, cancel) = token();
    let steps = BoundMemberLocks::new(manager.clone(), cancel);
    let (result, thread) = on_a_thread({
        let (backend, root) = (backend.clone(), temp.path().to_path_buf());
        move || {
            let world = crate::operation_context::TestWorld::physical();
            handle_pull_head_with_events_in(
                &world.context(),
                &backend,
                &root,
                pull_request(&["mem_app"]),
                "op_pull",
                &NullSink,
                &steps,
            )
        }
    });
    manager.assert_waiting(&result, "it should be waiting");
    assert_eq!(
        backend.fetch_peak(),
        0,
        "the pull fetched while it was held"
    );
    controls.cancel();
    let error = result
        .recv_timeout(LONG)
        .expect("the wait did not wake on the cancel")
        .unwrap_err();
    thread.join().unwrap();
    assert_eq!(error.code, crate::model::ErrorCode::Cancelled);
    assert_eq!(backend.fetch_peak(), 0);
    assert_eq!(manager.held_count(), 1, "the holder lost its member");
    drop(holder);
}
