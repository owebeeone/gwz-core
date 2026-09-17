//! `gwz merge --remote <name> --wait <secs>` (GwzOpenDecisions D1).
//!
//! The family merge takes the family lock at its step 5 and holds it across
//! the import and the delegation, so two unattended family operations fired
//! for one request refused each other outright. `--wait <secs>` carries
//! GwzLaneCleanFixes R21's polling helper (`local_clone::wait::lock_family`)
//! to this last family verb: only `Busy` is retried, at the same fixed
//! interval, until the deadline.
//!
//! The two tests here are the two ends of that deadline, against a stub that
//! holds the real family lock through the store contract rather than a fake:
//! a wait that outlives the holder merges, and a wait the holder outlives
//! reports `Busy` (`ErrorCode::OpenOperation`) after spending the whole
//! deadline and importing nothing.

use std::thread::sleep;
use std::time::{Duration, Instant};

use gwz_family_store_contract::{FamilyLocation, FamilyStore};

use super::{commit, family, family_merge_request, head, import_refs};
use crate::local_clone::family_merge::family_store;
use crate::local_clone::wait::POLL_INTERVAL;
use crate::model::ErrorCode;
use crate::operation::NullSink;
use crate::workspace_ops::handle_merge_with_local_family;

/// A family merge that waits behind another holder of the family lock still
/// merges, once that holder leaves and before the deadline. The helper
/// rereads the family index under the lock it wins, so the source is
/// resolved against the family that holder left.
#[test]
fn a_family_merge_waits_out_a_busy_family_lock_and_then_integrates() {
    let family = family("family-merge-wait-wins", &["app"]);
    let backend = &family.backend;
    let in_clone = commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );

    let store = family_store();
    let held = store
        .try_lock(&FamilyLocation::new(&family.root))
        .expect("the stub takes the family lock first");
    let mut request = family_merge_request("A", None);
    request.wait_seconds = Some(30);

    let response = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep(POLL_INTERVAL * 4);
            drop(held);
        });
        handle_merge_with_local_family(
            backend,
            &family.root,
            request,
            "op_family_merge_wait",
            &NullSink,
        )
    })
    .expect("the merge waits out the stub and integrates");

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(head(backend, &family.member("app")), in_clone);
}

/// A wait the holder outlives is the unchanged refusal, only later: the
/// whole deadline is spent, `Busy` is reported as `OpenOperation`, and
/// nothing was imported -- the lock is taken before the transfer id is
/// minted, so no import ref exists to retain.
#[test]
fn a_family_merge_whose_wait_expires_reports_busy_and_imported_nothing() {
    let family = family("family-merge-wait-expires", &["app"]);
    let backend = &family.backend;
    commit(
        &family.clone_member("app"),
        "feature.txt",
        "from A\n",
        "work in A",
        "HEAD",
    );
    let before = head(backend, &family.member("app"));

    let store = family_store();
    let held = store
        .try_lock(&FamilyLocation::new(&family.root))
        .expect("the stub holds the family lock for the whole wait");
    let mut request = family_merge_request("A", None);
    let wait = Duration::from_secs(1);
    request.wait_seconds = Some(1);

    let started = Instant::now();
    let error = handle_merge_with_local_family(
        backend,
        &family.root,
        request,
        "op_family_merge_wait_expires",
        &NullSink,
    )
    .expect_err("the stub never leaves, so the wait expires");
    let elapsed = started.elapsed();
    drop(held);

    assert_eq!(error.code, ErrorCode::OpenOperation, "{error:?}");
    assert!(
        elapsed >= wait,
        "the whole deadline is spent before Busy is reported: {elapsed:?}"
    );
    assert_eq!(head(backend, &family.member("app")), before);
    assert!(
        import_refs(&family.member("app")).is_empty(),
        "the lock is taken before any import, so nothing was fetched"
    );
}
