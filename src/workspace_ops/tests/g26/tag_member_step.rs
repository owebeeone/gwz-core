//! Step 2.3c: a tag push and a tag fetch take the member lock around their transfer (core session design 5.1). A tag
//! operation and a push never meet in one host (both take the workspace mutator lock), so the other step is a holder
//! of the member, as a fetch of another session would be.
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gwz_session_host::CallControls;

use super::*;
use crate::session_host::members::{BoundMemberLocks, MemberLockManager};

const SETTLE: Duration = Duration::from_millis(250);
const LONG: Duration = Duration::from_secs(30);

fn root_tag_request(op: crate::TagOp) -> crate::TagRequest {
    crate::TagRequest {
        meta: crate::RequestMeta {
            selection: root_only(),
            ..request_meta_with_workspace()
        },
        op,
        name: Some("v1".to_owned()),
        ..Default::default()
    }
}

/// Runs a root tag operation on a thread, under `steps`, and returns its result channel.
fn tag_on_a_thread(
    fixture: &Arc<PublicationFixture>,
    op: crate::TagOp,
    steps: BoundMemberLocks,
) -> (
    mpsc::Receiver<crate::model::ModelResult<crate::TagResponse>>,
    thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let fixture = Arc::clone(fixture);
    let handle = thread::spawn(move || {
        let world = crate::operation_context::TestWorld::physical();
        let result = handle_tag_in(
            &world.context(),
            &fixture.backend,
            &fixture.root,
            root_tag_request(op),
            "op_tag",
            &steps,
        );
        sender.send(result).unwrap();
    });
    (receiver, handle)
}

fn tag_fixture() -> Arc<PublicationFixture> {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    fixture.backend.set_tag(&fixture.root, "v1", ROOT_HEAD);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    Arc::new(fixture)
}

fn pushes(fixture: &PublicationFixture) -> usize {
    fixture
        .backend
        .remote_calls()
        .iter()
        .filter(|call| matches!(call, RemoteCall::Push { .. }))
        .count()
}

#[test]
fn a_tag_push_waits_for_the_member_another_step_holds_then_succeeds() {
    let fixture = tag_fixture();
    let manager = MemberLockManager::new();
    let holder = manager.lock(&fixture.root, "@root", None).unwrap();
    let session = Arc::new(());
    let (_controls, gate) = CallControls::new(&session);
    let steps = BoundMemberLocks::new(manager.clone(), gate.token().clone());
    let (result, thread) = tag_on_a_thread(&fixture, crate::TagOp::Push, steps);
    assert!(
        result.recv_timeout(SETTLE).is_err(),
        "the tag push did not wait for the member's holder"
    );
    assert_eq!(pushes(&fixture), 0, "the tag was pushed while it was held");
    drop(holder);
    let response = result
        .recv_timeout(LONG)
        .expect("the tag push ended")
        .unwrap();
    thread.join().unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(pushes(&fixture), 1);
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_cancelled_tag_push_wait_ends_with_cancelled_and_pushes_nothing() {
    let fixture = tag_fixture();
    let manager = MemberLockManager::new();
    let holder = manager.lock(&fixture.root, "@root", None).unwrap();
    let session = Arc::new(());
    let (controls, gate) = CallControls::new(&session);
    let steps = BoundMemberLocks::new(manager.clone(), gate.token().clone());
    let (result, thread) = tag_on_a_thread(&fixture, crate::TagOp::Push, steps);
    assert!(result.recv_timeout(SETTLE).is_err(), "it should be waiting");
    controls.cancel();
    let error = result
        .recv_timeout(LONG)
        .expect("the wait did not wake on the cancel")
        .unwrap_err();
    thread.join().unwrap();
    assert_eq!(error.code, crate::model::ErrorCode::Cancelled);
    assert_eq!(pushes(&fixture), 0);
    assert_eq!(manager.held_count(), 1);
    drop(holder);
}

fn tag_fetches(fixture: &PublicationFixture) -> usize {
    fixture.backend.tag_fetches().len()
}

#[test]
fn a_tag_fetch_waits_for_the_member_another_step_holds_then_succeeds() {
    let fixture = tag_fixture();
    let manager = MemberLockManager::new();
    let holder = manager.lock(&fixture.root, "@root", None).unwrap();
    let session = Arc::new(());
    let (_controls, gate) = CallControls::new(&session);
    let steps = BoundMemberLocks::new(manager.clone(), gate.token().clone());
    let (result, thread) = tag_on_a_thread(&fixture, crate::TagOp::Fetch, steps);
    assert!(
        result.recv_timeout(SETTLE).is_err(),
        "the tag fetch did not wait for the member's holder"
    );
    assert_eq!(
        tag_fetches(&fixture),
        0,
        "the tags were fetched while it was held"
    );
    drop(holder);
    let response = result
        .recv_timeout(LONG)
        .expect("the tag fetch ended")
        .unwrap();
    thread.join().unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(tag_fetches(&fixture), 1);
    assert_eq!(manager.held_count(), 0);
}

#[test]
fn a_cancelled_tag_fetch_wait_ends_with_cancelled_and_fetches_nothing() {
    let fixture = tag_fixture();
    let manager = MemberLockManager::new();
    let holder = manager.lock(&fixture.root, "@root", None).unwrap();
    let session = Arc::new(());
    let (controls, gate) = CallControls::new(&session);
    let steps = BoundMemberLocks::new(manager.clone(), gate.token().clone());
    let (result, thread) = tag_on_a_thread(&fixture, crate::TagOp::Fetch, steps);
    assert!(result.recv_timeout(SETTLE).is_err(), "it should be waiting");
    controls.cancel();
    let error = result
        .recv_timeout(LONG)
        .expect("the wait did not wake on the cancel")
        .unwrap_err();
    thread.join().unwrap();
    assert_eq!(error.code, crate::model::ErrorCode::Cancelled);
    assert_eq!(tag_fetches(&fixture), 0);
    assert_eq!(manager.held_count(), 1);
    drop(holder);
}
