//! `local_clone::tests::owner_wait`: the lane-made-by-a-tool surface
//! (gwz-core `dev-docs/GwzLaneCleanFixes.md` §3.6, R20/R21/R22).
//!
//! These are real creates over a real workspace, because what R22 asks to be
//! proved is a race between two whole invocations and not a property of a
//! pure function: one lane exists afterwards, the create that waited is
//! refused by the row the other one left, and that refusal names the owner
//! the winner recorded.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gwz_family_model::{INDEX_RELATIVE_PATH, INDEX_SCHEMA, MemberState};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilyStore};

use super::fixture::{FamilyFixture, family_workspace, meta};
use crate::git::Git2Backend;
use crate::model::{ErrorCode, ModelResult};
use crate::operation::NullSink;
use crate::workspace_ops::{handle_clone_local_workspace, handle_local_family};

fn clone_request(
    name: &str,
    owner: Option<&str>,
    wait_seconds: Option<i64>,
) -> crate::CloneLocalWorkspaceRequest {
    crate::CloneLocalWorkspaceRequest {
        meta: meta("req-clone-local"),
        name: name.to_owned(),
        dest: None,
        mode: crate::LocalCloneMode::Verbatim,
        branch: None,
        copy_source: None,
        owner: owner.map(ToOwned::to_owned),
        wait_seconds,
    }
}

fn list_request() -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        meta: meta("req-local-family"),
        op: crate::LocalFamilyOp::List,
        name: None,
        keep: None,
        force_hazards: Vec::new(),
        wait_seconds: None,
    }
}

fn view(root: &Path) -> gwz_family_model::FamilyView {
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(root))
        .expect("the family reads")
    {
        FamilyObservation::Family { view, .. } => view,
        FamilyObservation::NoFamily => panic!("{} is in no family", root.display()),
    }
}

fn index_bytes(root: &Path) -> Vec<u8> {
    std::fs::read(root.join(INDEX_RELATIVE_PATH)).expect("the index reads")
}

fn create(
    fixture: &FamilyFixture,
    request: crate::CloneLocalWorkspaceRequest,
) -> ModelResult<String> {
    let backend = Git2Backend::without_credential_helpers();
    handle_clone_local_workspace(&backend, &fixture.root, request, "op-clone", &NullSink)
        .map(|response| response.response.meta.message.unwrap_or_default())
}

/// R20: the token is recorded by the write that reserves the row, reported
/// by the listing in both renderings, and left alone by every later write.
#[test]
fn an_owner_token_is_recorded_once_and_reported_by_the_listing() {
    let fixture = family_workspace("owner-recorded");
    create(
        &fixture,
        clone_request("A", Some("claude-code:lane_1"), None),
    )
    .expect("the owned create");
    create(&fixture, clone_request("B", None, None)).expect("the unowned create");

    let family = view(&fixture.root);
    let (_, a) = family.member("A").expect("row A");
    let (_, b) = family.member("B").expect("row B");
    assert_eq!(
        a.owner.as_ref().map(|owner| owner.as_str()),
        Some("claude-code:lane_1"),
        "the reserving write recorded the token"
    );
    assert_eq!(b.owner, None, "a create without --owner records none");

    // The index that now stands is format 2, and `owner` is the only key it
    // gained: `B`'s row carries no `owner` at all.
    let text = String::from_utf8(index_bytes(&fixture.root)).expect("utf-8");
    assert!(text.contains(INDEX_SCHEMA), "{text}");
    assert_eq!(
        text.matches("owner:").count(),
        1,
        "only the owned row carries the key: {text}"
    );

    // `B`'s create rewrote the index several times after `A` was reserved,
    // and never touched `A`'s token: nothing sets, changes or clears it.
    let backend = Git2Backend::without_credential_helpers();
    let listing = handle_local_family(
        &backend,
        &fixture.root,
        list_request(),
        "op-list",
        &NullSink,
    )
    .expect("the listing");
    let owners: Vec<Option<&str>> = listing
        .members
        .iter()
        .map(|entry| entry.owner.as_deref())
        .collect();
    assert_eq!(
        owners,
        vec![None, Some("claude-code:lane_1"), None],
        "the root first, then A and B in name order"
    );
}

/// R20: the refusal an older gwz gives for a format-2 index must name the
/// minimum gwz version that reads it, and that version must be one this
/// build could actually be released as. The model states it as a literal
/// because the release bump comes after the work, so this is the guard: a
/// gwz-core bumped *past* `INDEX_MIN_GWZ_VERSION` without moving it would
/// be promising operators a gwz that never read a v2 index.
#[test]
fn the_minimum_gwz_version_for_the_index_is_not_behind_this_build() {
    fn triple(version: &str) -> (u64, u64, u64) {
        let mut parts = version
            .split(['.', '-'])
            .filter_map(|part| part.parse::<u64>().ok());
        (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        )
    }
    let minimum = triple(gwz_family_model::INDEX_MIN_GWZ_VERSION);
    let building = triple(env!("CARGO_PKG_VERSION"));
    assert!(
        minimum >= building,
        "INDEX_MIN_GWZ_VERSION {:?} is behind gwz-core {:?}: move it to the version this ships in",
        gwz_family_model::INDEX_MIN_GWZ_VERSION,
        env!("CARGO_PKG_VERSION"),
    );
}

/// R20: the token is opaque, and a shape the model does not admit is
/// refused before anything is observed, reserved or copied.
#[test]
fn an_owner_token_outside_the_admitted_shape_refuses_before_anything_is_reserved() {
    let fixture = family_workspace("owner-refused");
    let error = create(&fixture, clone_request("A", Some("lane one"), None))
        .expect_err("a space is not in the alphabet");
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("--owner"), "{}", error.message);
    assert!(
        !fixture.root.join(INDEX_RELATIVE_PATH).exists(),
        "a shape refusal founds no family"
    );
    assert!(
        !fixture.sibling("A").exists(),
        "and allocates no destination"
    );
}

/// Poll the index until member `name` has been reserved. The reservation is
/// written under the family lock and the lock is held for the whole of the
/// copy that follows, so a create observed here is a create that is holding
/// the lock right now. Nothing here takes the lock: an attempt of our own
/// could win the race we are trying to observe.
fn wait_until_reserved(root: &Path, name: &str, limit: Duration) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Ok(bytes) = std::fs::read(root.join(INDEX_RELATIVE_PATH))
            && let Ok(text) = String::from_utf8(bytes)
            && text.contains(&format!("{name}:"))
            && text.contains(MemberState::Creating.as_str())
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("`{name}` was never reserved within {limit:?}");
}

/// R22, first test: two creates of one name, concurrently, the second with
/// `--wait`.
///
/// The second create is started only once the first has reserved its row,
/// which it does under the family lock and holds through the whole copy, so
/// the waiting create really does meet a busy lock and really does wait. It
/// then wins the lock, rereads, and is answered by the row the first create
/// left -- the point of R21's "a wait that succeeds MUST reread the index
/// before acting".
///
/// What is proved afterwards: exactly one lane exists; the waiting create
/// reports the name as held and names the first create's owner; and the
/// index carries exactly one reservation, the winner's, so the loser's
/// refusal wrote nothing -- one index reservation per create that got one,
/// and none for the create that was refused.
#[test]
fn two_concurrent_creates_of_one_name_leave_one_lane_and_name_the_holder() {
    let fixture = family_workspace("concurrent-create");
    let root = fixture.root.clone();
    let destination: PathBuf = fixture.sibling("A");

    let (winner, loser) = std::thread::scope(|scope| {
        let first = scope.spawn(|| create(&fixture, clone_request("A", Some("owner-one"), None)));
        // Deliberately *not* a sleep: the wait is only exercised if the
        // first create is inside its locked section when the second starts.
        wait_until_reserved(&root, "A", Duration::from_secs(120));
        let second =
            scope.spawn(|| create(&fixture, clone_request("A", Some("owner-two"), Some(120))));
        (
            first.join().expect("the first create finishes"),
            second.join().expect("the waiting create finishes"),
        )
    });

    winner.expect("the create that took the lock first makes the lane");
    let error = loser.expect_err("the create that waited finds the name held");
    assert_eq!(error.code, ErrorCode::PathCollision);
    assert!(
        error.message.contains("name `A` already holds"),
        "the refusal reports the name as held: {}",
        error.message
    );
    assert!(
        error.message.contains("owner `owner-one`"),
        "and names the first create's owner: {}",
        error.message
    );
    assert!(
        !error.message.contains("owner-two"),
        "a refused create records nothing of its own: {}",
        error.message
    );

    // Exactly one lane, and it is the winner's.
    let family = view(&root);
    assert_eq!(family.members.len(), 1, "exactly one row: {family:?}");
    let (_, row) = family.member("A").expect("row A");
    assert_eq!(row.state, MemberState::Ready);
    assert_eq!(
        row.owner.as_ref().map(|owner| owner.as_str()),
        Some("owner-one")
    );
    assert!(destination.join(".gwz/family-root").is_file());
    assert_eq!(
        String::from_utf8(index_bytes(&root))
            .expect("utf-8")
            .matches("owner:")
            .count(),
        1,
        "one reservation stands, carrying one owner"
    );
}

/// R21: without `--wait`, a busy family lock still refuses immediately.
/// Held here by an ordinary session, so the refusal is the store's own
/// `Busy` and not a create losing a race.
#[test]
fn a_busy_family_lock_still_refuses_at_once_without_wait() {
    let fixture = family_workspace("busy-no-wait");
    create(&fixture, clone_request("A", None, None)).expect("found the family");
    let store = gwz_family_store::YamlFamilyStore::new();
    let held = store
        .try_lock(&FamilyLocation::new(&fixture.root))
        .expect("the test takes the family lock");
    let started = Instant::now();
    let error = create(&fixture, clone_request("B", None, None)).expect_err("a busy lock refuses");
    let elapsed = started.elapsed();
    assert!(
        error.message.contains("held by another operation"),
        "{}",
        error.message
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "no wait means no waiting, took {elapsed:?}"
    );
    drop(held);
}
