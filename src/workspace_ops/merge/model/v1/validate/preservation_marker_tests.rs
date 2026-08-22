//! Durable preservation-cursor marker validation.
//!
//! Pins `GwzM5-8DurableCursorAmendment.md` §2.1 (field delta), §2.2 (validated
//! derivations, the two structural rules, and the exhaustive sixteen-shape
//! legality table), and the §2.2 pending-action cross-checks.

use super::super::super::{OperationState, ParticipantState, PreservationEvidence};
use super::super::{
    MergeOperationRecordV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1,
};
use super::tests::{oid, record, sha};
use super::validate_v1_preservation;
use crate::model::ErrorCode;

const OWNER: &str = "mem_a";

/// The owner anchor: `mem_a`'s resulting commit.
fn anchor() -> String {
    oid('c')
}

/// A recorded backup target deliberately distinct from the anchor, so the two
/// §2.2 value equations (`noop_commit` tracks `backup_commit` when the backup
/// pair is present; `reset_commit` always tracks the anchor) are separable.
fn backup_target() -> String {
    oid('d')
}

fn canonical_ref() -> String {
    format!("refs/gwz/merge/merge_1/{OWNER}/head")
}

fn owner() -> PreservationOwnerV1 {
    PreservationOwnerV1::Participant {
        member_id: OWNER.to_owned(),
    }
}

/// Build a `Preserving` record whose single `mem_a` evidence row carries the
/// requested combination. `B`/`S` are the pre-amendment artifact pairs; `N`/`R`
/// are this amendment's markers, valued per the §2.2 equations.
fn case(b: bool, s: bool, n: bool, r: bool) -> MergeOperationRecordV1 {
    let mut case = record();
    case.state = OperationState::Preserving;
    case.preservation_publication_handoff =
        Some(super::super::PreservationPublicationHandoffV1::NoCandidate);
    let participant = case.participants.get_mut(OWNER).unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(anchor());
    participant.preservation = vec![PreservationEvidence {
        backup_ref: b.then(canonical_ref),
        backup_commit: b.then(backup_target),
        stash_id: s.then(|| "stash_merge_1".to_owned()),
        stash_object_id: s.then(|| oid('b')),
        // §2.2: `noop_commit` equals the recorded `backup_commit` when the
        // backup pair is present, and otherwise the immutable owner anchor.
        noop_commit: n.then(|| if b { backup_target() } else { anchor() }),
        // §2.2: `reset_commit` always equals the immutable owner anchor.
        reset_commit: r.then(anchor),
    }];
    case
}

fn is_legal(b: bool, s: bool, n: bool, r: bool) -> bool {
    validate_v1_preservation(&case(b, s, n, r)).is_ok()
}

fn label(b: bool, s: bool, n: bool, r: bool) -> String {
    let mut parts = Vec::new();
    for (flag, name) in [(b, "B"), (s, "S"), (n, "N"), (r, "R")] {
        if flag {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        "all absent".to_owned()
    } else {
        parts.join("+")
    }
}

/// §2.2's legality table, enumerated structurally over all sixteen shapes from
/// the two normative rules: (1) a stash pair and `noop_commit` may never
/// coexist; (2) `reset_commit` requires artifact-pass completion evidence on
/// the same row (`noop_commit` or a stash pair). An empty row stays invalid.
fn expected_legal(b: bool, s: bool, n: bool, r: bool) -> bool {
    if !b && !s && !n && !r {
        return false; // unchanged: an empty row is invalid
    }
    if s && n {
        return false; // rule 1
    }
    if r && !(n || s) {
        return false; // rule 2
    }
    true
}

#[test]
fn marker_legality_table_is_exhaustive_over_all_sixteen_row_shapes() {
    let mut checked = 0;
    for bits in 0..16u8 {
        let (b, s, n, r) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
        let expected = expected_legal(b, s, n, r);
        assert_eq!(
            is_legal(b, s, n, r),
            expected,
            "row shape '{}' should be {}",
            label(b, s, n, r),
            if expected { "legal" } else { "rejected" }
        );
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn pre_amendment_row_shapes_remain_valid_for_graceful_degradation() {
    // §2.2 table row 2 / §4 item 2: B, S and B+S must keep validating exactly
    // as before, or degraded (pre-amendment) records stop being readable.
    for (b, s) in [(true, false), (false, true), (true, true)] {
        assert!(
            is_legal(b, s, false, false),
            "pre-amendment shape '{}' must stay valid",
            label(b, s, false, false)
        );
    }
}

#[test]
fn noop_commit_must_equal_the_recorded_backup_commit_when_the_backup_pair_is_present() {
    let mut wrong = case(true, false, true, false);
    row(&mut wrong).noop_commit = Some(anchor()); // the anchor, not the backup target
    assert_eq!(
        validate_v1_preservation(&wrong).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn noop_commit_must_equal_the_owner_anchor_when_no_backup_pair_is_present() {
    let mut wrong = case(false, false, true, false);
    row(&mut wrong).noop_commit = Some(backup_target());
    assert_eq!(
        validate_v1_preservation(&wrong).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn reset_commit_must_equal_the_owner_anchor() {
    let mut wrong = case(true, false, true, true);
    row(&mut wrong).reset_commit = Some(backup_target());
    assert_eq!(
        validate_v1_preservation(&wrong).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn fabricated_marker_values_are_rejected_without_any_repository_read() {
    // §3.3: what hardens at decode is exactly the marker-bearing contradiction
    // surface. A non-oid marker is a typed evidence mismatch.
    for value in ["not-a-commit", "", &"z".repeat(40)] {
        let mut wrong = case(false, false, true, false);
        row(&mut wrong).noop_commit = Some(value.to_owned());
        assert_eq!(
            validate_v1_preservation(&wrong).unwrap_err().code,
            ErrorCode::PreservationEvidenceMismatch,
            "fabricated noop_commit '{value}' must reject"
        );
    }
}

#[test]
fn pending_backup_ref_contradicts_a_noop_marker_on_the_same_owner_row() {
    // §2.2: a pending `BackupRef` for an owner whose row carries `noop_commit`
    // is an owner/phase/pass contradiction and rejects at decode.
    let mut case = case(false, false, true, false);
    case.pending_preservation = Some(PendingPreservationActionV1::BackupRef {
        owner: owner(),
        name: canonical_ref(),
        target_commit: backup_target(),
    });
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn pending_stash_contradicts_a_noop_marker_on_the_same_owner_row() {
    let mut case = case(true, false, true, false);
    case.pending_preservation = Some(PendingPreservationActionV1::Stash {
        owner: owner(),
        phase: PreservationStashPhaseV1::CreateStash,
        stash_id: None,
        stash_object_id: None,
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: backup_target(),
        preimage_sha256: sha('1'),
        root_publication_handoff: None,
    });
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn pending_reset_contradicts_a_reset_marker_on_the_same_owner_row() {
    // §2.2: a pending `ResetAttachedRef` for an owner whose row already
    // carries `reset_commit` is a contradiction — the position is retired.
    let mut case = case(true, false, true, true);
    case.pending_preservation = Some(PendingPreservationActionV1::ResetAttachedRef {
        owner: owner(),
        branch: "main".to_owned(),
        expected_commit: backup_target(),
        restore_commit: anchor(),
        phase: PreservationRefResetPhaseV1::ResetRef,
        root_publication_handoff: None,
    });
    assert_eq!(
        validate_v1_preservation(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );
}

#[test]
fn a_pending_reset_is_still_legal_beside_an_artifact_pass_marker() {
    // The backfill shape of §3.1: a marker-less-at-the-reset-position row that
    // already records the artifact pass as a no-op must still accept its
    // pending reset, or degraded records could never retire.
    let mut case = case(true, false, true, false);
    case.pending_preservation = Some(PendingPreservationActionV1::ResetAttachedRef {
        owner: owner(),
        branch: "main".to_owned(),
        expected_commit: backup_target(),
        restore_commit: anchor(),
        phase: PreservationRefResetPhaseV1::ResetRef,
        root_publication_handoff: None,
    });
    validate_v1_preservation(&case).unwrap();
}

#[test]
fn markers_are_absent_by_default_on_the_wire() {
    // §2.1: both fields are absent when unset, never `null`, so every byte
    // stream produced by a writer that does not set them is identical to
    // today's.
    let row = PreservationEvidence {
        backup_ref: Some(canonical_ref()),
        backup_commit: Some(backup_target()),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    };
    let yaml = serde_yaml::to_string(&row).unwrap();
    assert!(!yaml.contains("noop_commit"), "unset marker leaked: {yaml}");
    assert!(
        !yaml.contains("reset_commit"),
        "unset marker leaked: {yaml}"
    );
}

#[test]
fn markers_emit_in_declaration_order_after_stash_object_id() {
    // §2.1: appended after `stash_object_id`; YAML emission order is
    // declaration order, and the spellings are exactly these.
    let row = PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: Some(anchor()),
        reset_commit: Some(anchor()),
    };
    let yaml = serde_yaml::to_string(&row).unwrap();
    let noop = yaml.find("noop_commit").expect("noop_commit is emitted");
    let reset = yaml.find("reset_commit").expect("reset_commit is emitted");
    assert!(noop < reset, "declaration order not preserved: {yaml}");
    assert!(
        yaml.find("stash_object_id").unwrap() < noop,
        "markers must trail the inherited fields: {yaml}"
    );
}

#[test]
fn absent_markers_round_trip_from_a_row_that_never_mentions_them() {
    // A pre-amendment byte stream still decodes, with both markers absent.
    let yaml = "backup_ref: refs/gwz/merge/merge_1/mem_a/head\nbackup_commit: \
                dddddddddddddddddddddddddddddddddddddddd\nstash_id: null\nstash_object_id: null\n";
    let row: PreservationEvidence = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(row.noop_commit, None);
    assert_eq!(row.reset_commit, None);
    assert_eq!(row.backup_commit.as_deref(), Some(backup_target().as_str()));
}

fn row(case: &mut MergeOperationRecordV1) -> &mut PreservationEvidence {
    &mut case.participants.get_mut(OWNER).unwrap().preservation[0]
}
