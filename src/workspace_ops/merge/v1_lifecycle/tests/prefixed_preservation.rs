use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{PreservationTransition, V1Transition, prepare};
use super::fixtures::{
    apply_preservation, evidence_rollback_record, oid, preservation_evidence, preservation_payload,
    preservation_prefix, preserving_record, reset_action as plain_reset,
    stash_action as plain_stash,
};
use crate::workspace_ops::merge::PreservationEvidence;
use crate::workspace_ops::merge::model::v1::{
    GitObjectAlgorithmV1, GitObjectIdV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationPublicationHandoffV1, PreservationRefResetPhaseV1 as R,
    PreservationStashPhaseV1 as S, PublicationIndexFormV1, PublicationPrefixV1,
};
use crate::workspace_ops::tests::TempDir;

const PHASES: [S; 11] = [
    S::NormalizeParent,
    S::NormalizeMarker,
    S::NormalizeLock,
    S::NormalizeIndex,
    S::CreateStash,
    S::RestoreIndex,
    S::RestoreLock,
    S::RestoreParent,
    S::RestoreMarker,
    S::WriteBundle,
    S::Complete,
];
const RESET_PHASES: [R; 10] = [
    R::PrepareParent,
    R::PrepareMarker,
    R::PrepareLock,
    R::PrepareIndex,
    R::ResetRef,
    R::RestoreIndex,
    R::RestoreLock,
    R::RestoreParent,
    R::RestoreMarker,
    R::Complete,
];

fn rejects(
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: PreservationTransition,
) -> bool {
    prepare(
        lease,
        current,
        V1Transition::Preservation(Box::new(transition)),
    )
    .is_err()
}

#[test]
fn prefixed_stash_accepts_only_each_exact_successor_and_owner_prefix() {
    let root = TempDir::new_git("merge-v1-prefixed-preservation");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = evidence_rollback_record(&root);
    model.state = crate::workspace_ops::merge::OperationState::Preserving;
    model.preservation_publication_handoff = Some(candidate_handoff());
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let wrong_position = PreservationCursorPosition::Stash(S::NormalizeLock);
    let wrong_intent = PreparedStashIntent::for_test(
        &current,
        "@publication-root",
        "begin_stash",
        "cursor_checked",
        payload(wrong_position, Some(action(S::NormalizeLock)), None),
        prefix(&current, wrong_position),
    )
    .unwrap();
    assert!(rejects(
        &lease,
        &current,
        PreservationTransition::BeginStash(Box::new(wrong_intent)),
    ));
    let position = PreservationCursorPosition::Stash(S::NormalizeParent);
    let intent = PreparedStashIntent::for_test(
        &current,
        "@publication-root",
        "begin_stash",
        "cursor_checked",
        payload(position, Some(action(S::NormalizeParent)), None),
        prefix(&current, position),
    )
    .unwrap();
    let mut current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginStash(Box::new(intent)),
    );
    let position = PreservationCursorPosition::Stash(S::NormalizeParent);
    let mut changed = action(S::NormalizeMarker);
    if let PendingPreservationActionV1::Stash {
        preimage_sha256, ..
    } = &mut changed
    {
        *preimage_sha256 = "2".repeat(64);
    }
    let changed = VerifiedStashPhase::for_test(
        &current,
        "@publication-root",
        "advance_stash",
        "completed",
        payload(position, Some(changed), None),
        prefix(&current, position),
    )
    .unwrap();
    assert!(rejects(
        &lease,
        &current,
        PreservationTransition::AdvanceStash(Box::new(changed)),
    ));

    for (old, next) in [
        (S::NormalizeParent, S::NormalizeMarker),
        (S::NormalizeMarker, S::NormalizeLock),
        (S::NormalizeLock, S::NormalizeIndex),
        (S::NormalizeIndex, S::CreateStash),
        (S::CreateStash, S::RestoreIndex),
        (S::RestoreIndex, S::RestoreLock),
        (S::RestoreLock, S::RestoreParent),
        (S::RestoreParent, S::RestoreMarker),
        (S::RestoreMarker, S::WriteBundle),
        (S::WriteBundle, S::Complete),
    ] {
        let position = PreservationCursorPosition::Stash(old);
        for invalid in PHASES.into_iter().filter(|phase| *phase != next) {
            let proof = phase_proof(&current, position, invalid);
            assert!(rejects(
                &lease,
                &current,
                PreservationTransition::AdvanceStash(Box::new(proof)),
            ));
        }
        let proof = phase_proof(&current, position, next);
        current = apply_preservation(
            &root,
            &lease,
            &current,
            PreservationTransition::AdvanceStash(Box::new(proof)),
        );
    }
    let position = PreservationCursorPosition::Stash(S::Complete);
    assert!(rejects(
        &lease,
        &current,
        PreservationTransition::AdvanceStash(Box::new(phase_proof(
            &current,
            position,
            S::Complete,
        ))),
    ));
    let proof = VerifiedStashCompletion::for_test(
        &current,
        "@publication-root",
        "finish_stash",
        "completed",
        payload(position, None, None),
        prefix(&current, position),
    )
    .unwrap();
    let finished = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::FinishStash(Box::new(proof)),
    );
    assert!(finished.record().pending_preservation.is_none());
    assert_eq!(
        finished.record().preservation_publication_handoff,
        Some(candidate_handoff())
    );
}

#[test]
fn prefixed_reset_accepts_only_each_exact_successor() {
    let root = TempDir::new_git("merge-v1-prefixed-reset");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = evidence_rollback_record(&root);
    model.state = crate::workspace_ops::merge::OperationState::Preserving;
    model.preservation_publication_handoff = Some(candidate_handoff());
    model
        .publication
        .as_mut()
        .unwrap()
        .root_preservation
        .push(PreservationEvidence {
            backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()),
            backup_commit: Some(oid('d')),
            stash_id: None,
            stash_object_id: None,
            noop_commit: None,
            reset_commit: None,
        });
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let wrong_position = PreservationCursorPosition::ResetAttachedRef(R::PrepareLock);
    let wrong_intent = PreparedRefResetIntent::for_test(
        &current,
        "@publication-root",
        "begin_reset_attached_ref",
        "cursor_checked",
        payload(wrong_position, Some(reset_action(R::PrepareLock)), None),
        prefix(&current, wrong_position),
    )
    .unwrap();
    assert!(rejects(
        &lease,
        &current,
        PreservationTransition::BeginResetAttachedRef(Box::new(wrong_intent)),
    ));
    let position = PreservationCursorPosition::ResetAttachedRef(R::PrepareParent);
    let intent = PreparedRefResetIntent::for_test(
        &current,
        "@publication-root",
        "begin_reset_attached_ref",
        "cursor_checked",
        payload(position, Some(reset_action(R::PrepareParent)), None),
        prefix(&current, position),
    )
    .unwrap();
    let mut current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginResetAttachedRef(Box::new(intent)),
    );

    for (old, next) in [
        (R::PrepareParent, R::PrepareMarker),
        (R::PrepareMarker, R::PrepareLock),
        (R::PrepareLock, R::PrepareIndex),
        (R::PrepareIndex, R::ResetRef),
        (R::ResetRef, R::RestoreIndex),
        (R::RestoreIndex, R::RestoreLock),
        (R::RestoreLock, R::RestoreParent),
        (R::RestoreParent, R::RestoreMarker),
        (R::RestoreMarker, R::Complete),
    ] {
        let position = PreservationCursorPosition::ResetAttachedRef(old);
        for invalid in RESET_PHASES.into_iter().filter(|phase| *phase != next) {
            let proof = reset_phase_proof(&current, position, invalid);
            assert!(rejects(
                &lease,
                &current,
                PreservationTransition::AdvanceResetAttachedRef(Box::new(proof)),
            ));
        }
        current = apply_preservation(
            &root,
            &lease,
            &current,
            PreservationTransition::AdvanceResetAttachedRef(Box::new(reset_phase_proof(
                &current, position, next,
            ))),
        );
    }
    let position = PreservationCursorPosition::ResetAttachedRef(R::Complete);
    assert!(rejects(
        &lease,
        &current,
        PreservationTransition::AdvanceResetAttachedRef(Box::new(reset_phase_proof(
            &current,
            position,
            R::Complete,
        ))),
    ));
    // `GwzM5-8DurableCursorAmendment.md` §3.1 edge 1 + the marker backfill.
    // This owner's row is the marker-less `B` shape a pre-amendment record
    // presents, so the same atomic rewrite that writes `reset_commit` also
    // backfills `noop_commit` — valued per §2.2 at the recorded
    // `backup_commit`, since the backup pair is present. The result is the
    // §2.2-legal `B+N+R`, never `B+R`.
    let proof = VerifiedRefResetCompletion::for_test(
        &current,
        "@publication-root",
        "finish_reset_attached_ref",
        "completed",
        payload(
            position,
            None,
            Some(PreservationEvidence {
                backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()),
                backup_commit: Some(oid('d')),
                stash_id: None,
                stash_object_id: None,
                noop_commit: Some(oid('d')),
                reset_commit: Some(oid('e')),
            }),
        ),
        prefix(&current, position),
    )
    .unwrap();
    let finished = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::FinishResetAttachedRef(Box::new(proof)),
    );
    assert!(finished.record().pending_preservation.is_none());
}

#[test]
fn no_prefix_stash_and_reset_reject_every_non_successor_phase() {
    let root = TempDir::new_git("merge-v1-no-prefix-preservation-negative-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();

    for (current_phase, next_phase, with_evidence) in [
        (S::CreateStash, S::WriteBundle, true),
        (S::WriteBundle, S::Complete, false),
    ] {
        let mut model = preserving_record();
        model.pending_preservation = Some(plain_stash(current_phase));
        if current_phase == S::WriteBundle {
            model
                .participants
                .get_mut("mem_a")
                .unwrap()
                .preservation
                .push(preservation_evidence(true));
        }
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        let position = PreservationCursorPosition::Stash(current_phase);
        for invalid in PHASES.into_iter().filter(|phase| *phase != next_phase) {
            let proof = VerifiedStashPhase::for_test(
                &current,
                "mem_a",
                "advance_stash",
                "completed",
                preservation_payload(
                    position,
                    Some(plain_stash(invalid)),
                    with_evidence.then(|| preservation_evidence(true)),
                ),
                preservation_prefix(&current, position),
            )
            .unwrap();
            assert!(
                rejects(
                    &lease,
                    &current,
                    PreservationTransition::AdvanceStash(Box::new(proof)),
                ),
                "stash {current_phase:?} accepted {invalid:?}"
            );
        }
    }

    let mut model = preserving_record();
    model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation
        .push(preservation_evidence(false));
    model.pending_preservation = Some(plain_reset(R::ResetRef));
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let position = PreservationCursorPosition::ResetAttachedRef(R::ResetRef);
    for invalid in RESET_PHASES
        .into_iter()
        .filter(|phase| *phase != R::Complete)
    {
        let proof = VerifiedRefResetPhase::for_test(
            &current,
            "mem_a",
            "advance_reset_attached_ref",
            "completed",
            preservation_payload(position, Some(plain_reset(invalid)), None),
            preservation_prefix(&current, position),
        )
        .unwrap();
        assert!(
            rejects(
                &lease,
                &current,
                PreservationTransition::AdvanceResetAttachedRef(Box::new(proof)),
            ),
            "reset accepted {invalid:?}"
        );
    }
}

fn phase_proof(
    current: &StoredV1Record,
    position: PreservationCursorPosition,
    next: S,
) -> VerifiedStashPhase {
    VerifiedStashPhase::for_test(
        current,
        "@publication-root",
        "advance_stash",
        "completed",
        payload(
            position,
            Some(action(next)),
            (position == PreservationCursorPosition::Stash(S::CreateStash)).then(evidence),
        ),
        prefix(current, position),
    )
    .unwrap()
}

fn reset_phase_proof(
    current: &StoredV1Record,
    position: PreservationCursorPosition,
    next: R,
) -> VerifiedRefResetPhase {
    VerifiedRefResetPhase::for_test(
        current,
        "@publication-root",
        "advance_reset_attached_ref",
        "completed",
        payload(position, Some(reset_action(next)), None),
        prefix(current, position),
    )
    .unwrap()
}

fn owner() -> PreservationOwnerV1 {
    PreservationOwnerV1::PublicationRoot
}

fn candidate_handoff() -> PreservationPublicationHandoffV1 {
    PreservationPublicationHandoffV1::Candidate {
        prefix: PublicationPrefixV1::Baseline,
        index: PublicationIndexFormV1::Pre,
    }
}

fn action(phase: S) -> PendingPreservationActionV1 {
    let ids = !matches!(
        phase,
        S::NormalizeParent
            | S::NormalizeMarker
            | S::NormalizeLock
            | S::NormalizeIndex
            | S::CreateStash
    );
    PendingPreservationActionV1::Stash {
        owner: owner(),
        phase,
        stash_id: ids.then(|| "stash_merge_1".into()),
        stash_object_id: ids.then(|| GitObjectIdV1 {
            algorithm: GitObjectAlgorithmV1::Sha1,
            digest_hex: oid('b'),
        }),
        message: "gwz:stash_merge_1: merge preservation".into(),
        head_commit: oid('e'),
        preimage_sha256: "1".repeat(64),
        root_publication_handoff: candidate_handoff().candidate(),
    }
}

fn reset_action(phase: R) -> PendingPreservationActionV1 {
    PendingPreservationActionV1::ResetAttachedRef {
        owner: owner(),
        branch: "main".into(),
        expected_commit: oid('d'),
        restore_commit: oid('e'),
        phase,
        root_publication_handoff: candidate_handoff().candidate(),
    }
}

fn evidence() -> PreservationEvidence {
    PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: Some("stash_merge_1".into()),
        stash_object_id: Some(oid('b')),
        noop_commit: None,
        reset_commit: None,
    }
}

fn payload(
    observed_position: PreservationCursorPosition,
    pending: Option<PendingPreservationActionV1>,
    evidence: Option<PreservationEvidence>,
) -> PreservationPayload {
    PreservationPayload {
        owner: owner(),
        observed_position,
        pending,
        evidence,
        publication_prefix: Some("baseline".into()),
    }
}

fn prefix(
    current: &StoredV1Record,
    position: PreservationCursorPosition,
) -> VerifiedPreservationCursorPrefix {
    VerifiedPreservationCursorPrefix::for_test(
        current,
        "@publication-root",
        "preservation_cursor",
        "prefix_verified",
        PreservationCursorPrefix {
            owner: owner(),
            position,
        },
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Durable preservation-cursor marker write edges.
// `GwzM5-8DurableCursorAmendment.md` §3.1 (write edges + backfill), §8.1
// (immutability across the new rewrite edges), §8.2 (degraded-record
// pending-reset retirement).
// ---------------------------------------------------------------------------

/// A `Preserving` record with no pending action and the given root evidence
/// row, ready for a marker write.
fn marker_record(root: &TempDir, row: Option<PreservationEvidence>) -> StoredV1Record {
    let mut model = evidence_rollback_record(root);
    model.state = crate::workspace_ops::merge::OperationState::Preserving;
    model.preservation_publication_handoff = Some(candidate_handoff());
    let rows = &mut model.publication.as_mut().unwrap().root_preservation;
    rows.clear();
    if let Some(row) = row {
        rows.push(row);
    }
    StoredV1Record::for_test(&root.path, model).unwrap()
}

fn backup_row() -> PreservationEvidence {
    PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/root/head".into()),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }
}

fn artifact_noop(
    current: &StoredV1Record,
    evidence: PreservationEvidence,
) -> PreservationTransition {
    let position = PreservationCursorPosition::Stash(S::Complete);
    PreservationTransition::RecordArtifactNoop(Box::new(
        PreparedArtifactNoop::for_test(
            current,
            "@publication-root",
            "record_artifact_noop",
            "recorded",
            payload(position, None, Some(evidence)),
            prefix(current, position),
        )
        .unwrap(),
    ))
}

fn reset_noop(current: &StoredV1Record, evidence: PreservationEvidence) -> PreservationTransition {
    let position = PreservationCursorPosition::ResetAttachedRef(R::Complete);
    PreservationTransition::RecordResetNoop(Box::new(
        PreparedResetNoop::for_test(
            current,
            "@publication-root",
            "record_reset_noop",
            "recorded",
            payload(position, None, Some(evidence)),
            prefix(current, position),
        )
        .unwrap(),
    ))
}

/// §3.1: the marker write is an evidence-only rewrite — no pending action is
/// journaled for it, and the four inherited fields stay byte-constant (§8.1).
#[test]
fn marker_writes_are_action_free_and_leave_every_inherited_field_byte_constant() {
    let root = TempDir::new_git("merge-v1-marker-write-footprint");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = marker_record(&root, Some(backup_row()));

    // The artifact pass proved the stash position unnecessary: `B` -> `B+N`,
    // with `noop_commit` equal to the recorded `backup_commit` per §2.2.
    let next = apply_preservation(
        &root,
        &lease,
        &current,
        artifact_noop(
            &current,
            PreservationEvidence {
                noop_commit: Some(oid('d')),
                ..backup_row()
            },
        ),
    );
    let row = &next
        .record()
        .publication
        .as_ref()
        .unwrap()
        .root_preservation[0];
    assert_eq!(
        row.backup_ref.as_deref(),
        backup_row().backup_ref.as_deref()
    );
    assert_eq!(row.backup_commit.as_deref(), Some(oid('d').as_str()));
    assert_eq!(row.stash_id, None);
    assert_eq!(row.stash_object_id, None);
    assert_eq!(row.noop_commit.as_deref(), Some(oid('d').as_str()));
    assert_eq!(row.reset_commit, None);
    assert!(next.record().pending_preservation.is_none());

    // The reset pass then retires the reset position: `B+N` -> `B+N+R`, with
    // `reset_commit` equal to the immutable owner anchor.
    let after = apply_preservation(
        &root,
        &lease,
        &next,
        reset_noop(
            &next,
            PreservationEvidence {
                noop_commit: Some(oid('d')),
                reset_commit: Some(oid('e')),
                ..backup_row()
            },
        ),
    );
    let row = &after
        .record()
        .publication
        .as_ref()
        .unwrap()
        .root_preservation[0];
    assert_eq!(row.reset_commit.as_deref(), Some(oid('e').as_str()));
    assert!(after.record().pending_preservation.is_none());
}

/// §3.1: a clean owner that never wrote a row gets an `N`-only row, and the
/// write may not fabricate artifact evidence.
#[test]
fn an_absent_row_accepts_only_a_marker_only_successor() {
    let root = TempDir::new_git("merge-v1-marker-write-absent-row");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = marker_record(&root, None);

    assert!(rejects(
        &lease,
        &current,
        artifact_noop(
            &current,
            PreservationEvidence {
                noop_commit: Some(oid('e')),
                ..backup_row()
            },
        ),
    ));

    let next = apply_preservation(
        &root,
        &lease,
        &current,
        artifact_noop(
            &current,
            PreservationEvidence {
                backup_ref: None,
                backup_commit: None,
                stash_id: None,
                stash_object_id: None,
                noop_commit: Some(oid('e')),
                reset_commit: None,
            },
        ),
    );
    let row = &next
        .record()
        .publication
        .as_ref()
        .unwrap()
        .root_preservation[0];
    assert_eq!(row.noop_commit.as_deref(), Some(oid('e').as_str()));
    assert!(row.backup_ref.is_none() && row.stash_id.is_none());
}

/// §2.2/§8.1: both markers are immutable once written, and a marker write may
/// never disturb an inherited field. The whole-row `set_evidence` replacement
/// makes this discipline-enforced, so the write edge checks it.
#[test]
fn marker_writes_reject_every_disturbance_of_a_pinned_field() {
    let root = TempDir::new_git("merge-v1-marker-write-immutability");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let seeded = PreservationEvidence {
        noop_commit: Some(oid('d')),
        ..backup_row()
    };
    let current = marker_record(&root, Some(seeded.clone()));

    for (label, evidence) in [
        (
            "backup_commit revalued",
            PreservationEvidence {
                backup_commit: Some(oid('c')),
                ..seeded.clone()
            },
        ),
        (
            "backup pair dropped",
            PreservationEvidence {
                backup_ref: None,
                backup_commit: None,
                ..seeded.clone()
            },
        ),
        (
            "stash pair fabricated",
            PreservationEvidence {
                stash_id: Some("stash_merge_1".into()),
                stash_object_id: Some(oid('b')),
                ..seeded.clone()
            },
        ),
        (
            "existing marker revalued",
            PreservationEvidence {
                noop_commit: Some(oid('e')),
                reset_commit: Some(oid('e')),
                ..backup_row()
            },
        ),
        (
            "existing marker dropped",
            PreservationEvidence {
                reset_commit: Some(oid('e')),
                ..backup_row()
            },
        ),
    ] {
        assert!(
            rejects(&lease, &current, reset_noop(&current, evidence)),
            "{label} must be rejected by the marker write edge"
        );
    }
}

/// §8.2(c): degraded-record pending-reset retirement. A marker-less `B+S` row
/// retires to `B+S+R` with NO backfill — the stash pair is already the
/// artifact-pass evidence (§3.1), so `noop_commit` must stay absent or rule 1
/// would reject the row.
#[test]
fn a_marker_less_backup_and_stash_row_retires_without_a_noop_backfill() {
    let root = TempDir::new_git("merge-v1-degraded-stash-retirement");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let seeded = PreservationEvidence {
        stash_id: Some("stash_merge_1".into()),
        stash_object_id: Some(oid('b')),
        ..backup_row()
    };
    let current = marker_record(&root, Some(seeded.clone()));

    // Rule 1: a stash pair and `noop_commit` may never coexist.
    assert!(rejects(
        &lease,
        &current,
        reset_noop(
            &current,
            PreservationEvidence {
                noop_commit: Some(oid('d')),
                reset_commit: Some(oid('e')),
                ..seeded.clone()
            },
        ),
    ));

    let next = apply_preservation(
        &root,
        &lease,
        &current,
        reset_noop(
            &current,
            PreservationEvidence {
                reset_commit: Some(oid('e')),
                ..seeded
            },
        ),
    );
    let row = &next
        .record()
        .publication
        .as_ref()
        .unwrap()
        .root_preservation[0];
    assert_eq!(row.noop_commit, None, "no backfill beside a stash pair");
    assert_eq!(row.reset_commit.as_deref(), Some(oid('e').as_str()));
    assert!(row.stash_id.is_some() && row.backup_ref.is_some());
}
