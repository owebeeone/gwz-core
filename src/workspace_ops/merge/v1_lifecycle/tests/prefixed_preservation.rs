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
    PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S, PublicationPrefixV1,
};
use crate::workspace_ops::tests::TempDir;

const PHASES: [S; 5] = [
    S::NormalizeRoot,
    S::CreateStash,
    S::RestoreRoot,
    S::WriteBundle,
    S::Complete,
];
const RESET_PHASES: [R; 3] = [R::ResetRef, R::RestoreRoot, R::Complete];

#[test]
fn prefixed_stash_accepts_only_each_exact_successor_and_owner_prefix() {
    let root = TempDir::new("merge-v1-prefixed-preservation");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = evidence_rollback_record(&root);
    model.state = crate::workspace_ops::merge::OperationState::Preserving;
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let position = PreservationCursorPosition::Stash(S::NormalizeRoot);
    let intent = PreparedStashIntent::for_test(
        &current,
        "@publication-root",
        "begin_stash",
        "cursor_checked",
        payload(position, Some(action(S::NormalizeRoot)), None),
        prefix(&current, position),
    )
    .unwrap();
    let mut current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginStash(Box::new(intent)),
    );

    for (old, next) in [
        (S::NormalizeRoot, S::CreateStash),
        (S::CreateStash, S::RestoreRoot),
        (S::RestoreRoot, S::WriteBundle),
        (S::WriteBundle, S::Complete),
    ] {
        let position = PreservationCursorPosition::Stash(old);
        for invalid in PHASES.into_iter().filter(|phase| *phase != next) {
            let proof = phase_proof(&current, position, invalid);
            assert!(
                prepare(
                    &lease,
                    &current,
                    V1Transition::Preservation(Box::new(PreservationTransition::AdvanceStash(
                        Box::new(proof),
                    ))),
                )
                .is_err()
            );
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
        finished
            .record()
            .publication
            .as_ref()
            .unwrap()
            .preservation_prefix
            .as_deref(),
        Some("baseline")
    );
}

#[test]
fn prefixed_reset_accepts_only_restore_root_then_complete() {
    let root = TempDir::new("merge-v1-prefixed-reset");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = evidence_rollback_record(&root);
    model.state = crate::workspace_ops::merge::OperationState::Preserving;
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
        });
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let position = PreservationCursorPosition::ResetAttachedRef(R::ResetRef);
    let intent = PreparedRefResetIntent::for_test(
        &current,
        "@publication-root",
        "begin_reset_attached_ref",
        "cursor_checked",
        payload(position, Some(reset_action(R::ResetRef)), None),
        prefix(&current, position),
    )
    .unwrap();
    let mut current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginResetAttachedRef(Box::new(intent)),
    );

    for (old, next) in [(R::ResetRef, R::RestoreRoot), (R::RestoreRoot, R::Complete)] {
        let position = PreservationCursorPosition::ResetAttachedRef(old);
        for invalid in RESET_PHASES.into_iter().filter(|phase| *phase != next) {
            let proof = reset_phase_proof(&current, position, invalid);
            assert!(
                prepare(
                    &lease,
                    &current,
                    V1Transition::Preservation(Box::new(
                        PreservationTransition::AdvanceResetAttachedRef(Box::new(proof)),
                    )),
                )
                .is_err()
            );
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
    let proof = VerifiedRefResetCompletion::for_test(
        &current,
        "@publication-root",
        "finish_reset_attached_ref",
        "completed",
        payload(position, None, None),
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
    let root = TempDir::new("merge-v1-no-prefix-preservation-negative-matrix");
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
                prepare(
                    &lease,
                    &current,
                    V1Transition::Preservation(Box::new(PreservationTransition::AdvanceStash(
                        Box::new(proof),
                    ))),
                )
                .is_err(),
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
            prepare(
                &lease,
                &current,
                V1Transition::Preservation(Box::new(
                    PreservationTransition::AdvanceResetAttachedRef(Box::new(proof)),
                )),
            )
            .is_err(),
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

fn action(phase: S) -> PendingPreservationActionV1 {
    let ids = !matches!(phase, S::NormalizeRoot | S::CreateStash);
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
        root_publication_prefix: Some(PublicationPrefixV1::Baseline),
    }
}

fn reset_action(phase: R) -> PendingPreservationActionV1 {
    PendingPreservationActionV1::ResetAttachedRef {
        owner: owner(),
        branch: "main".into(),
        expected_commit: oid('d'),
        restore_commit: oid('e'),
        phase,
        root_publication_prefix: Some(PublicationPrefixV1::Baseline),
    }
}

fn evidence() -> PreservationEvidence {
    PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: Some("stash_merge_1".into()),
        stash_object_id: Some(oid('b')),
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
