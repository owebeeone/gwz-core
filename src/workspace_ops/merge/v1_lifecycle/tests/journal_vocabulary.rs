use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::*;
use super::fixtures::{
    apply_preservation, backup_action, evidence_rollback_record, preservation_evidence,
    preservation_payload, preservation_prefix, preserving_record, reset_action,
    selected_root_rollback_record, stash_action,
};
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1, ParticipantRollbackKindV1, PendingRollbackActionV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1, RootMetadataRollbackStepV1,
    test_record as record,
};
use crate::workspace_ops::merge::{OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

macro_rules! preserve {
    ($ty:ident, $current:expr, $action:literal, $phase:literal, $position:expr, $pending:expr, $evidence:expr) => {
        $ty::for_test(
            $current,
            "mem_a",
            $action,
            $phase,
            preservation_payload($position, $pending, $evidence),
            preservation_prefix($current, $position),
        )
        .unwrap()
    };
}

#[test]
pub(super) fn preservation_reducers_enforce_the_exact_no_prefix_phase_graph() {
    let root = TempDir::new_git("merge-v1-preservation-phase-reducers");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = StoredV1Record::for_test(&root.path, preserving_record()).unwrap();

    let intent = preserve!(
        PreparedBackupRefIntent,
        &current,
        "begin_backup_ref",
        "cursor_checked",
        PreservationCursorPosition::BackupRef,
        Some(backup_action()),
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginBackupRef(Box::new(intent)),
    );
    let proof = preserve!(
        VerifiedBackupRef,
        &current,
        "finish_backup_ref",
        "completed",
        PreservationCursorPosition::BackupRef,
        None,
        Some(preservation_evidence(false))
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::FinishBackupRef(Box::new(proof)),
    );

    let create = PreservationCursorPosition::Stash(PreservationStashPhaseV1::CreateStash);
    let intent = preserve!(
        PreparedStashIntent,
        &current,
        "begin_stash",
        "cursor_checked",
        create,
        Some(stash_action(PreservationStashPhaseV1::CreateStash)),
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginStash(Box::new(intent)),
    );
    let proof = preserve!(
        VerifiedStashPhase,
        &current,
        "advance_stash",
        "completed",
        create,
        Some(stash_action(PreservationStashPhaseV1::WriteBundle)),
        Some(preservation_evidence(true))
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::AdvanceStash(Box::new(proof)),
    );
    let write = PreservationCursorPosition::Stash(PreservationStashPhaseV1::WriteBundle);
    let proof = preserve!(
        VerifiedStashPhase,
        &current,
        "advance_stash",
        "completed",
        write,
        Some(stash_action(PreservationStashPhaseV1::Complete)),
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::AdvanceStash(Box::new(proof)),
    );
    let complete = PreservationCursorPosition::Stash(PreservationStashPhaseV1::Complete);
    let proof = preserve!(
        VerifiedStashCompletion,
        &current,
        "finish_stash",
        "completed",
        complete,
        None,
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::FinishStash(Box::new(proof)),
    );

    let reset = PreservationCursorPosition::ResetAttachedRef(PreservationRefResetPhaseV1::ResetRef);
    let intent = preserve!(
        PreparedRefResetIntent,
        &current,
        "begin_reset_attached_ref",
        "cursor_checked",
        reset,
        Some(reset_action(PreservationRefResetPhaseV1::ResetRef)),
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::BeginResetAttachedRef(Box::new(intent)),
    );
    let proof = preserve!(
        VerifiedRefResetPhase,
        &current,
        "advance_reset_attached_ref",
        "completed",
        reset,
        Some(reset_action(PreservationRefResetPhaseV1::Complete)),
        None
    );
    let current = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::AdvanceResetAttachedRef(Box::new(proof)),
    );
    let complete =
        PreservationCursorPosition::ResetAttachedRef(PreservationRefResetPhaseV1::Complete);
    let proof = preserve!(
        VerifiedRefResetCompletion,
        &current,
        "finish_reset_attached_ref",
        "completed",
        complete,
        None,
        None
    );
    let next = apply_preservation(
        &root,
        &lease,
        &current,
        PreservationTransition::FinishResetAttachedRef(Box::new(proof)),
    );
    assert!(next.record().pending_preservation.is_none());
}

#[test]
pub(super) fn rollback_reducers_follow_only_exact_cursor_successors() {
    let root = TempDir::new_git("merge-v1-rollback-phase-reducers");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut current =
        StoredV1Record::for_test(&root.path, evidence_rollback_record(&root)).unwrap();
    let intent = PreparedEvidenceRollback::for_test(
        &current,
        "@publication",
        "begin_evidence_rollback",
        "prepared",
        evidence_action(EvidenceRollbackStepV1::EvidenceCommit),
    )
    .unwrap();
    current = apply_rollback(
        &root,
        &lease,
        &current,
        RollbackTransition::BeginEvidence(Box::new(intent)),
    );
    for (old, next, phase) in [
        (
            EvidenceRollbackStepV1::EvidenceCommit,
            EvidenceRollbackStepV1::Boundary,
            "evidence_commit",
        ),
        (
            EvidenceRollbackStepV1::Boundary,
            EvidenceRollbackStepV1::Lock,
            "boundary",
        ),
        (
            EvidenceRollbackStepV1::Lock,
            EvidenceRollbackStepV1::Marker,
            "lock",
        ),
        (
            EvidenceRollbackStepV1::Marker,
            EvidenceRollbackStepV1::Index,
            "marker",
        ),
        (
            EvidenceRollbackStepV1::Index,
            EvidenceRollbackStepV1::Complete,
            "index",
        ),
    ] {
        assert_eq!(
            current.record().pending_rollback,
            Some(evidence_action(old))
        );
        for invalid in [
            EvidenceRollbackStepV1::EvidenceCommit,
            EvidenceRollbackStepV1::Boundary,
            EvidenceRollbackStepV1::Lock,
            EvidenceRollbackStepV1::Marker,
            EvidenceRollbackStepV1::Index,
            EvidenceRollbackStepV1::Complete,
        ] {
            if invalid == next {
                continue;
            }
            let invalid = VerifiedEvidenceRollbackStep::for_test(
                &current,
                "@publication",
                "advance_evidence_rollback",
                phase,
                evidence_action(invalid),
            )
            .unwrap();
            rejects(
                &lease,
                &current,
                RollbackTransition::AdvanceEvidence(Box::new(invalid)),
            );
        }
        let proof = VerifiedEvidenceRollbackStep::for_test(
            &current,
            "@publication",
            "advance_evidence_rollback",
            phase,
            evidence_action(next),
        )
        .unwrap();
        current = apply_rollback(
            &root,
            &lease,
            &current,
            RollbackTransition::AdvanceEvidence(Box::new(proof)),
        );
    }
    let proof = VerifiedEvidenceRollbackCompletion::for_test(
        &current,
        "@publication",
        "finish_evidence_rollback",
        "complete",
        (),
    )
    .unwrap();
    assert_eq!(proof.value(), &());
    let next = apply_rollback(
        &root,
        &lease,
        &current,
        RollbackTransition::FinishEvidence(proof),
    );
    assert!(
        next.record()
            .publication
            .as_ref()
            .unwrap()
            .evidence_rolled_back
    );

    let mut integrated = record();
    integrated.state = OperationState::RollingBack;
    let row = integrated.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some("d".repeat(40));
    let current = StoredV1Record::for_test(&root.path, integrated).unwrap();
    let action = PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    };
    let intent = PreparedParticipantRollback::for_test(
        &current,
        "mem_a",
        "begin_participant_rollback",
        "prepared",
        action,
    )
    .unwrap();
    let current = apply_rollback(
        &root,
        &lease,
        &current,
        RollbackTransition::BeginParticipant(Box::new(intent)),
    );
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::RolledBack;
    let proof = VerifiedParticipantRollback::for_test(
        &current,
        "mem_a",
        "finish_participant_rollback",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    assert_eq!(
        apply_rollback(
            &root,
            &lease,
            &current,
            RollbackTransition::FinishParticipant(Box::new(proof)),
        )
        .record()
        .participants["mem_a"]
            .state,
        ParticipantState::RolledBack
    );

    let mut current =
        StoredV1Record::for_test(&root.path, selected_root_rollback_record()).unwrap();
    let intent = PreparedRootMetadataRollback::for_test(
        &current,
        "@root",
        "begin_root_metadata_rollback",
        "prepared",
        root_action(RootMetadataRollbackStepV1::Manifest),
    )
    .unwrap();
    current = apply_rollback(
        &root,
        &lease,
        &current,
        RollbackTransition::BeginSelectedRoot(Box::new(intent)),
    );
    for (next, phase) in [
        (RootMetadataRollbackStepV1::Lock, "manifest"),
        (RootMetadataRollbackStepV1::Complete, "lock"),
    ] {
        for invalid in [
            RootMetadataRollbackStepV1::Manifest,
            RootMetadataRollbackStepV1::Lock,
            RootMetadataRollbackStepV1::Complete,
        ] {
            if invalid == next {
                continue;
            }
            let invalid = VerifiedRootMetadataRollbackStep::for_test(
                &current,
                "@root",
                "advance_root_metadata_rollback",
                phase,
                root_action(invalid),
            )
            .unwrap();
            rejects(
                &lease,
                &current,
                RollbackTransition::AdvanceSelectedRoot(Box::new(invalid)),
            );
        }
        let proof = VerifiedRootMetadataRollbackStep::for_test(
            &current,
            "@root",
            "advance_root_metadata_rollback",
            phase,
            root_action(next),
        )
        .unwrap();
        current = apply_rollback(
            &root,
            &lease,
            &current,
            RollbackTransition::AdvanceSelectedRoot(Box::new(proof)),
        );
    }
    let proof = VerifiedRootMetadataRollbackCompletion::for_test(
        &current,
        "@root",
        "finish_root_metadata_rollback",
        "complete",
        (),
    )
    .unwrap();
    assert_eq!(proof.value(), &());
    assert!(
        apply_rollback(
            &root,
            &lease,
            &current,
            RollbackTransition::FinishSelectedRoot(proof),
        )
        .record()
        .pending_rollback
        .is_none()
    );
}

fn evidence_action(next_step: EvidenceRollbackStepV1) -> PendingRollbackActionV1 {
    PendingRollbackActionV1::PublicationEvidence { next_step }
}

fn root_action(next_step: RootMetadataRollbackStepV1) -> PendingRollbackActionV1 {
    PendingRollbackActionV1::SelectedRootMetadata { next_step }
}

fn rejects(lease: &V1MutationLease, current: &StoredV1Record, transition: RollbackTransition) {
    assert!(prepare(lease, current, V1Transition::Rollback(Box::new(transition))).is_err());
}

fn apply_rollback(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: RollbackTransition,
) -> StoredV1Record {
    let rewrite = prepare(lease, current, V1Transition::Rollback(Box::new(transition))).unwrap();
    StoredV1Record::for_test(&root.path, rewrite.next().clone()).unwrap()
}
