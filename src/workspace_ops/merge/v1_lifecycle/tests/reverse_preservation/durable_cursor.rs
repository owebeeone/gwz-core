//! Acceptance suite for the durable preservation cursor.
//!
//! `GwzM5-8DurableCursorAmendment.md` §8.2(a)/(b) (restart equivalence, the
//! durable path proven image-capture-free, crash-injected marker idempotence),
//! §8.5 (post-`reset_commit` preflight-only interference) and §8.7 (bundle
//! identity invariance). These are the RecordContract §9 exit rows that the
//! round-1 implementation package left undelivered.

use std::fs;

use super::*;
use crate::model::{ModelError, ModelResult};
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::PreservationEvidence;
use crate::workspace_ops::merge::preserve::{
    V1_PRESERVATION_IMAGE_CAPTURES, v1_preservation_owners, v1_write_bundle_checked,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, V1ServiceResponse, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

/// Two integrated members, both parked clean at their immutable anchors — the
/// shape whose artifact and reset positions are all no-ops, so the durable
/// cursor can retire every position with markers alone.
fn two_clean_owners(name: &str) -> PreservationFixture {
    let mut fixture = integrated_fixture(name);
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.protected, &fixture.result)
        .unwrap();
    let (later, _, later_result, later_protected) =
        add_integrated_member(&mut fixture, "mem_b", "members/b");
    fixture
        .backend
        .set_branch_target_checked(&later, "main", &later_protected, &later_result)
        .unwrap();
    fixture
}

/// The `N+R` row a fully-retired no-op owner carries: both markers valued at
/// the immutable owner anchor per §2.2, and no artifact evidence at all.
fn retired_marker_row(anchor: &str) -> PreservationEvidence {
    PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: Some(anchor.to_owned()),
        reset_commit: Some(anchor.to_owned()),
    }
}

fn install_retired_markers(fixture: &mut PreservationFixture) {
    for member_id in ["mem_a", "mem_b"] {
        let row = fixture.model.participants.get_mut(member_id).unwrap();
        let anchor = row.resulting_commit.clone().unwrap();
        row.preservation = vec![retired_marker_row(&anchor)];
    }
}

fn captures<T>(body: impl FnOnce() -> T) -> (T, usize) {
    V1_PRESERVATION_IMAGE_CAPTURES.with(|count| count.set(0));
    let value = body();
    (
        value,
        V1_PRESERVATION_IMAGE_CAPTURES.with(std::cell::Cell::get),
    )
}

fn preserve(fixture: &PreservationFixture) -> ModelResult<V1ServiceResponse> {
    let context = fixture.context();
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
}

/// §8.2(a)/(b). The same fixture is driven to the same cursor position twice:
/// (a) with durable markers, and (b) with the markers stripped to the
/// pre-amendment shape — where, per §8.2, "stripping an `N`-only row deletes
/// the row entirely, producing the legitimate absent-earlier-row shape, which
/// must classify identically via the live fallback (never reject at decode)".
///
/// The classification verdict must be identical, and the (a) path must be
/// image-capture-free: §3.2 makes every position decode-terminal, so the whole
/// live-image surface `stash_complete`/`reset_complete` used to pay per
/// dispatch disappears. That is the amendment's headline cost claim, and it is
/// measured here at the real capture seam (`v1_preservation_image`).
#[test]
fn restart_with_and_without_durable_markers_classifies_identically() {
    // (a) durable markers: every position decode-terminal.
    let mut marked = two_clean_owners("v1-durable-cursor-equivalence-marked");
    install_retired_markers(&mut marked);
    marked.seed_open();
    let (marked_result, marked_captures) = captures(|| preserve(&marked));
    let marked_disposition = marked_result.unwrap().disposition();

    // (b) the same state stripped to the pre-amendment shape: an `N`-only row
    // strips to no row at all, which is the legitimate absent-earlier-row
    // shape — it must classify identically through the live fallback and must
    // never reject at decode.
    let mut stripped = two_clean_owners("v1-durable-cursor-equivalence-stripped");
    for member_id in ["mem_a", "mem_b"] {
        stripped
            .model
            .participants
            .get_mut(member_id)
            .unwrap()
            .preservation
            .clear();
    }
    stripped.seed_open();
    let (stripped_result, stripped_captures) = captures(|| preserve(&stripped));
    let stripped_disposition = stripped_result
        .expect("the stripped pre-amendment shape must never reject at decode")
        .disposition();

    assert_eq!(
        marked_disposition, stripped_disposition,
        "durable and degraded records must classify identically"
    );
    assert_eq!(
        marked_disposition,
        V1ResponseDisposition::Terminal(OperationState::Aborted)
    );

    // §3.2 / §8.2(a): the durable path pays no live preservation image at all.
    assert_eq!(
        marked_captures, 0,
        "the durable-marker path must be image-capture-free"
    );
    // The degraded path is the control: it still pays today's live re-proof
    // verbatim (§4 item 2), which is exactly what the markers retire.
    assert!(
        stripped_captures > 0,
        "the degraded live-fallback control must still capture images"
    );
}

/// §8.2. A crash between a live pass and its marker write re-proves live at
/// restart and re-writes the identical marker — "idempotent, and never worse
/// than the status quo". Injected by discarding the durable write once.
#[test]
fn a_crash_between_the_live_pass_and_its_marker_write_reproves_and_converges() {
    let fixture = two_clean_owners("v1-durable-cursor-crash-idempotence");
    fixture.seed_open();
    let record_path = fixture
        .root
        .path
        .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id));
    let before = fs::read(&record_path).unwrap();

    // One dispatch: the artifact pass live-proves the first owner and persists
    // its skip marker as its one durable step.
    let context = fixture.context();
    let mut stopping = StopAfterFirstDurableWrite {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        writes: 0,
    };
    let _ = run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut stopping,
    );
    let after_first = fs::read(&record_path).unwrap();
    let marker = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap()
        .record()
        .participants["mem_a"]
        .preservation
        .first()
        .and_then(|row| row.noop_commit.clone());

    // Inject the crash: the live pass happened, the marker write did not
    // survive. Restart must re-prove live and re-derive the same value.
    fs::write(&record_path, &before).unwrap();
    let resumed = preserve(&fixture).unwrap();
    assert_eq!(
        resumed.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted),
        "the re-proved pass must converge, not wedge"
    );
    if let Some(marker) = marker {
        // The re-derived value is record-derived (anchor / recorded
        // backup_commit), so it is necessarily the same one.
        assert_eq!(
            resumed.current().record().participants["mem_a"]
                .preservation
                .first()
                .and_then(|row| row.noop_commit.clone()),
            Some(marker),
            "the marker re-write must be idempotent"
        );
    }
    assert_ne!(
        after_first, before,
        "the first dispatch must have made a durable step to crash out of"
    );
}

/// §8.5 — the highest-latency legal detection case, with the Code review's
/// branch-move interference rather than worktree dirt.
///
/// Owner `mem_a`'s no-op reset is proven and `reset_commit` written; the branch
/// then moves to a descendant before restart. The reset position is
/// decode-skipped (§3.2) and the owner has no own next action left, so
/// exhaustion proceeds and rollback-entry preflight is the sole remaining
/// catcher: it must refuse fail-closed with no mutation.
#[test]
fn interference_after_reset_commit_is_caught_only_by_rollback_entry_preflight() {
    let mut fixture = integrated_fixture("v1-durable-cursor-preflight-only");
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.protected, &fixture.result)
        .unwrap();
    let anchor = fixture.model.participants["mem_a"]
        .resulting_commit
        .clone()
        .unwrap();
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation = vec![retired_marker_row(&anchor)];
    fixture.seed_open();

    // The interference: the attached ref moves off the anchor to a descendant
    // AFTER `reset_commit` was written. Plan derivation's ancestor gate passes
    // for a descendant, so the cursor is genuinely reached.
    fixture
        .backend
        .set_branch_target_checked(&fixture.member, "main", &fixture.result, &fixture.protected)
        .unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();

    let outcome = preserve(&fixture);

    // Fail-closed: either a typed refusal or a stop into recovery — never a
    // completed abort over the moved branch.
    let refused = match &outcome {
        Err(_) => true,
        Ok(response) => {
            response.disposition() != V1ResponseDisposition::Terminal(OperationState::Aborted)
        }
    };
    assert!(
        refused,
        "interference after reset_commit must refuse fail-closed, got {:?}",
        outcome.as_ref().map(|response| response.disposition())
    );
    // No mutation: the branch is untouched and no preservation artifact was
    // fabricated over it.
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert!(
        fixture
            .backend
            .read_ref(
                &fixture.member,
                &format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
            )
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .backend
            .preservation_stashes(&fixture.member, &fixture.model.merge_id)
            .unwrap()
            .is_empty()
    );
}

/// §8.7 — bundle identity invariance. Canonical bundle derivation ignores
/// evidence rows without stash ids (`checked_bundle.rs`), so skip/reset markers
/// never enter bundle bytes: expected bundle output is byte-identical for the
/// same artifact set with and without markers.
#[test]
fn expected_bundle_bytes_are_identical_with_and_without_markers() {
    // `mem_a` stays dirty at its protected commit so preservation genuinely
    // stashes it; `mem_b` is parked clean at its anchor so it can carry markers.
    let mut fixture = dirty_integrated_fixture("v1-durable-cursor-bundle-invariance");
    let (later, _, later_result, later_protected) =
        add_integrated_member(&mut fixture, "mem_b", "members/b");
    fixture
        .backend
        .set_branch_target_checked(&later, "main", &later_protected, &later_result)
        .unwrap();
    fixture.seed_open();

    // Drive a real preservation pass far enough to create a genuine stash, but
    // stop before the reset executes so the live world still matches the
    // record and owner-plan derivation stays well defined.
    let context = fixture.context();
    let mut stopping = StopBeforeReset {
        inner: ReverseRuntime::new(&fixture.backend, &context),
    };
    let _ = run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut stopping,
    );
    let stored = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    // Graft the durable evidence the pass produced onto the pre-run model: the
    // run may have advanced state past `Preserving`, and owner-plan derivation
    // is only defined over the preserving shape. Only the evidence rows matter
    // to bundle derivation.
    let mut record = fixture.model.clone();
    for (member_id, row) in stored.record().participants.iter() {
        if let Some(target) = record.participants.get_mut(member_id) {
            target.preservation = row.preservation.clone();
        }
    }
    let stash_owner = record
        .participants
        .iter()
        .find(|(_, row)| row.preservation.iter().any(|row| row.stash_id.is_some()))
        .map(|(member_id, _)| member_id.clone())
        .expect("the dirty owner must have produced a stash pair");

    let plans = v1_preservation_owners(&fixture.backend, &fixture.root.path, &record).unwrap();
    let owner = plans
        .iter()
        .find(|plan| plan.target_id == stash_owner)
        .map(|plan| plan.owner.clone())
        .expect("the stash-bearing owner must have a preservation plan");
    let bundle =
        crate::stash::bundle_path(&fixture.root.path, &format!("stash_{}", record.merge_id));

    let write = |record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1| {
        if bundle.exists() {
            fs::remove_file(&bundle).unwrap();
        }
        let plans = v1_preservation_owners(&fixture.backend, &fixture.root.path, record).unwrap();
        v1_write_bundle_checked(&fixture.root.path, record, &plans, &owner).unwrap();
        fs::read(&bundle).unwrap()
    };

    let without_markers = write(&record);

    // Now add markers everywhere they are legal: a `reset_commit` on the
    // stash-bearing row (`S+R`/`B+S+R`) and full `N+R` retirement on the other
    // owner. Neither may reach bundle bytes.
    for (member_id, row) in record.participants.iter_mut() {
        let anchor = row.resulting_commit.clone().unwrap();
        if member_id == &stash_owner {
            if let Some(evidence) = row.preservation.first_mut() {
                evidence.reset_commit = Some(anchor);
            }
        } else {
            row.preservation = vec![retired_marker_row(&anchor)];
        }
    }
    let with_markers = write(&record);

    assert_eq!(
        without_markers, with_markers,
        "skip/reset markers must never enter bundle bytes"
    );
}

/// Stops the operation immediately after its first durable record write, so a
/// crash can be injected exactly at the marker-write boundary.
struct StopAfterFirstDurableWrite<'a> {
    inner: ReverseRuntime<'a, crate::git::Git2Backend>,
    writes: usize,
}

impl ExactObserver for StopAfterFirstDurableWrite<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if self.writes > 0 {
            return Err(ModelError::new(
                crate::model::ErrorCode::GitCommandFailed,
                "injected crash after the first durable marker write".to_owned(),
            ));
        }
        self.writes += 1;
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopAfterFirstDurableWrite<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}

/// Stops the operation at the first attached-ref reset, leaving the stash
/// created and every branch still where the record says it is.
struct StopBeforeReset<'a> {
    inner: ReverseRuntime<'a, crate::git::Git2Backend>,
}

impl ExactObserver for StopBeforeReset<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopBeforeReset<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if matches!(
            action,
            PhysicalActionKind::Preservation(
                crate::workspace_ops::merge::model::v1::PendingPreservationActionV1::ResetAttachedRef { .. }
            )
        ) {
            return ExecutionDiagnostic::Failed {
                code: crate::model::ErrorCode::GitCommandFailed,
                message: "stop before the attached-ref reset".into(),
                detail: None,
            };
        }
        self.inner.execute(lease, current, action)
    }
}
