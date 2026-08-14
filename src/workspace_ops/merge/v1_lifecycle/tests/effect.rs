use super::super::transition::*;
use super::super::{authority::*, checked::*};
use super::fixtures::{
    accepted_workspace, align_baseline_lock, candidate_payload, evidence_payload,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationOwnerV1, PreservationPublicationCandidateV1,
    PreservationPublicationHandoffV1, PreservationStashPhaseV1, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PreservationEvidence, PublicationProgress,
    PublicationStep,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn effect_vocabulary_is_one_to_one_with_transition_vocabulary() {
    let kinds = all_effect_kinds();
    assert_eq!(kinds.len(), EFFECT_VARIANT_COUNT);
    assert_eq!(kinds.len(), TRANSITION_VARIANT_COUNT);
}

pub(super) fn all_effect_kinds() -> [EffectKind; EFFECT_VARIANT_COUNT] {
    use EffectKind::*;
    [
        BeginExecution,
        AwaitResolution,
        Halt,
        EnterFinalizing,
        BeginPreservation,
        BeginRollback,
        CompleteOperation,
        AbortOperation,
        PrepareParticipant,
        RecordParticipantOutcome,
        RecordHaltedOutcomeAndResumeExecution,
        RecordHaltedOutcomeAndBeginRollback,
        RecordHaltedOutcomeAndBeginPreservation,
        AbandonNotStartedAndBeginRollback,
        AbandonNotStartedAndBeginPreservation,
        RecordPreparationFailureAndHalt,
        RecordOwnedRetryFailureAndHalt,
        RecordOwnedResolutionFailureAndHalt,
        RecordNoMutationAbort,
        FreezeAcceptance,
        ClassifyPublicationRequired,
        ClassifyNoPublication,
        BeginMigratedValidation,
        ClassifyMigratedPublicationRequired,
        ClassifyMigratedNoPublication,
        RecordCandidate,
        BeginEvidence,
        RecordEvidence,
        BeginCandidatePublication,
        RecordCandidatePublished,
        RecordPublicationVerified,
        EnterRecovery,
        ResumeRecovery,
        BeginBackupRef,
        FinishBackupRef,
        BeginStash,
        AdvanceStash,
        FinishStash,
        BeginResetAttachedRef,
        AdvanceResetAttachedRef,
        FinishResetAttachedRef,
        BeginParticipantRollback,
        FinishParticipantRollback,
        BeginEvidenceRollback,
        AdvanceEvidenceRollback,
        FinishEvidenceRollback,
        BeginSelectedRootRollback,
        AdvanceSelectedRootRollback,
        FinishSelectedRootRollback,
        RecordParticipantDrift,
        ClearParticipantDrift,
        RecordOperationDrift,
        ClearOperationDrift,
    ]
}

#[test]
fn exact_effect_verifier_accepts_owned_change_and_rejects_immutable_change() {
    let old = record();
    let mut next = old.clone();
    next.state = OperationState::Halted;
    let effect = TransitionEffect::operation_for_test(EffectKind::Halt);
    effect.verify_known_diff(&old, &next).unwrap();
    next.source_ref = "another-source".into();
    assert!(effect.verify_known_diff(&old, &next).is_err());
    let mut wrong_writer = old.clone();
    wrong_writer.state = OperationState::Halted;
    wrong_writer.writer_version = "unexpected".into();
    assert!(effect.verify_known_diff(&old, &wrong_writer).is_err());
}

#[test]
fn preservation_entry_is_the_only_effect_that_installs_the_durable_handoff() {
    let old = record();
    let mut preserving = old.clone();
    preserving.state = OperationState::Preserving;
    preserving.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::NoCandidate);
    TransitionEffect::operation_for_test(EffectKind::BeginPreservation)
        .verify_known_diff(&old, &preserving)
        .unwrap();

    let mut missing = preserving.clone();
    missing.preservation_publication_handoff = None;
    assert!(
        TransitionEffect::operation_for_test(EffectKind::BeginPreservation)
            .verify_known_diff(&old, &missing)
            .is_err()
    );

    let mut direct = old.clone();
    direct.state = OperationState::RollingBack;
    direct.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
    assert!(
        TransitionEffect::operation_for_test(EffectKind::BeginRollback)
            .verify_known_diff(&old, &direct)
            .is_err()
    );
}

#[test]
fn resolution_failure_cannot_mutate_the_authoritative_conflict_outcome() {
    let old = record();
    let effect = TransitionEffect::failure_for_test(
        EffectKind::RecordOwnedResolutionFailureAndHalt,
        "mem_a",
        &[],
    );
    let mut next = old.clone();
    next.state = OperationState::Halted;
    next.participants.get_mut("mem_a").unwrap().error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "resolution failed".into(),
        detail: None,
    });
    effect.verify_known_diff(&old, &next).unwrap();

    next.participants.get_mut("mem_a").unwrap().state = ParticipantState::Failed;
    assert!(effect.verify_known_diff(&old, &next).is_err());
}

#[test]
fn preservation_effects_require_exact_phase_owned_evidence() {
    let owner = PreservationOwnerV1::Participant {
        member_id: "mem_a".into(),
    };
    let mut backup = record();
    backup.pending_preservation = Some(PendingPreservationActionV1::BackupRef {
        owner: owner.clone(),
        name: "refs/gwz/merge/merge_1/mem_a/head".into(),
        target_commit: "a".repeat(40),
    });
    let mut finished = backup.clone();
    finished.pending_preservation = None;
    let finish_backup =
        TransitionEffect::preservation_for_test(EffectKind::FinishBackupRef, owner.clone());
    assert!(finish_backup.verify_known_diff(&backup, &finished).is_err());
    finished
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation
        .push(PreservationEvidence {
            backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".into()),
            backup_commit: Some("a".repeat(40)),
            stash_id: None,
            stash_object_id: None,
        });
    finish_backup.verify_known_diff(&backup, &finished).unwrap();

    let mut stash = record();
    stash.pending_preservation = Some(stash_action(
        owner.clone(),
        PreservationStashPhaseV1::Complete,
        None,
    ));
    let mut finished_stash = stash.clone();
    finished_stash.pending_preservation = None;
    finished_stash
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation
        .push(PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some("stash_merge_1".into()),
            stash_object_id: Some("b".repeat(40)),
        });
    let finish_stash = TransitionEffect::preservation_for_test(EffectKind::FinishStash, owner);
    assert!(
        finish_stash
            .verify_known_diff(&stash, &finished_stash)
            .is_err()
    );
}

#[test]
fn stash_effect_requires_evidence_only_after_create_stash() {
    let owner = PreservationOwnerV1::Participant {
        member_id: "mem_a".into(),
    };
    let mut old = record();
    old.pending_preservation = Some(stash_action(
        owner.clone(),
        PreservationStashPhaseV1::CreateStash,
        None,
    ));
    let mut next = old.clone();
    next.pending_preservation = Some(stash_action(
        owner.clone(),
        PreservationStashPhaseV1::WriteBundle,
        None,
    ));
    let effect = TransitionEffect::preservation_for_test(EffectKind::AdvanceStash, owner.clone());
    assert!(effect.verify_known_diff(&old, &next).is_err());
    next.participants
        .get_mut("mem_a")
        .unwrap()
        .preservation
        .push(PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some("stash_merge_1".into()),
            stash_object_id: Some("b".repeat(40)),
        });
    effect.verify_known_diff(&old, &next).unwrap();

    let mut polluted = record();
    polluted.pending_preservation = Some(stash_action(
        owner.clone(),
        PreservationStashPhaseV1::CreateStash,
        None,
    ));
    polluted.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
    let begin = TransitionEffect::preservation_for_test(EffectKind::BeginStash, owner);
    assert!(begin.verify_known_diff(&record(), &polluted).is_err());
}

#[test]
pub(super) fn publication_reducers_follow_every_exact_forward_phase() {
    let root = TempDir::new_git("merge-v1-publication-reducer");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = frozen(&root, &lease, false);

    let no_publication = BoundPublicationDecision::for_test(
        &current,
        "@publication",
        "classify_publication",
        "none",
        false,
    )
    .unwrap();
    let no_publication = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::ClassifyNone(no_publication),
    );
    assert_eq!(
        no_publication.record().publication.as_ref().unwrap().step,
        PublicationStep::Complete
    );

    let current = frozen(&root, &lease, true);
    let required = BoundPublicationDecision::for_test(
        &current,
        "@publication",
        "classify_publication",
        "required",
        true,
    )
    .unwrap();
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::ClassifyRequired(required),
    );
    let candidate = PreparedCandidate::for_test(
        &current,
        "@publication",
        "prepare_candidate",
        "prepared",
        candidate_payload(&current),
    )
    .unwrap();
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::RecordCandidate(Box::new(candidate)),
    );
    let intent = PreparedEvidenceIntent::for_test(
        &current,
        "@publication",
        "begin_evidence",
        "preflight",
        (),
    )
    .unwrap();
    assert_eq!(intent.value(), &());
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::BeginEvidence(intent),
    );
    let evidence = VerifiedEvidenceResult::for_test(
        &current,
        "@publication",
        "record_evidence",
        "completed",
        evidence_payload(&current),
    )
    .unwrap();
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::RecordEvidence(Box::new(evidence)),
    );
    let intent = PreparedPublicationIntent::for_test(
        &current,
        "@publication",
        "begin_candidate_publication",
        "preflight",
        (),
    )
    .unwrap();
    assert_eq!(intent.value(), &());
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::BeginCandidatePublication(intent),
    );
    let published = VerifiedCandidatePublicationCompletion::for_test(
        &current,
        "@publication",
        "candidate_publication",
        "completed",
        (),
    )
    .unwrap();
    assert_eq!(published.value(), &());
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::RecordCandidatePublished(published),
    );
    let verified = VerifiedPublicationCompletion::for_test(
        &current,
        "@publication",
        "publication",
        "verified",
        (),
    )
    .unwrap();
    let current = publish(
        &root,
        &lease,
        &current,
        PublicationTransition::RecordPublicationVerified(verified),
    );
    assert_eq!(
        current.record().publication.as_ref().unwrap().step,
        PublicationStep::Complete
    );
}

#[test]
pub(super) fn migrated_publication_compatibility_has_only_the_two_named_successors() {
    let root = TempDir::new_git("merge-v1-migrated-publication-reducer");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for (required, phase, expected) in [
        (
            true,
            "publication_required",
            PublicationStep::PreparingCandidate,
        ),
        (false, "no_publication", PublicationStep::Complete),
    ] {
        let seed = frozen(&root, &lease, required);
        let mut model = seed.record().clone();
        model.publication = Some(empty_publication(PublicationStep::NotStarted));
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        let validating = publish(
            &root,
            &lease,
            &current,
            PublicationTransition::BeginMigratedValidation,
        );
        let proof = VerifiedResults::for_test(
            &validating,
            "@publication",
            "validate_migrated_results",
            phase,
            required,
        )
        .unwrap();
        let transition = if required {
            PublicationTransition::ClassifyMigratedRequired(proof)
        } else {
            PublicationTransition::ClassifyMigratedNone(proof)
        };
        let next = publish(&root, &lease, &validating, transition);
        assert_eq!(next.record().publication.as_ref().unwrap().step, expected);
    }
}

fn frozen(root: &TempDir, lease: &V1MutationLease, changed: bool) -> StoredV1Record {
    let mut model = record();
    model.state = OperationState::Finalizing;
    let participant = model.participants.get_mut("mem_a").unwrap();
    participant.state = if changed {
        ParticipantState::FastForwarded
    } else {
        ParticipantState::UpToDate
    };
    participant.resulting_commit = Some(if changed {
        "d".repeat(40)
    } else {
        participant.before_commit.clone()
    });
    align_baseline_lock(&mut model);
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let accepted = PreparedAcceptedWorkspace::for_test(
        &current,
        "@operation",
        "freeze_acceptance",
        "prepared",
        accepted_workspace(&current),
    )
    .unwrap();
    apply(
        root,
        lease,
        &current,
        V1Transition::Acceptance(Box::new(AcceptanceTransition::Freeze(Box::new(accepted)))),
    )
}

fn apply(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: V1Transition,
) -> StoredV1Record {
    let rewrite = super::super::transition::prepare(lease, current, transition).unwrap();
    StoredV1Record::for_test(&root.path, rewrite.next().clone()).unwrap()
}

fn publish(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: PublicationTransition,
) -> StoredV1Record {
    apply(
        root,
        lease,
        current,
        V1Transition::Publication(Box::new(transition)),
    )
}

fn empty_publication(step: PublicationStep) -> PublicationProgress {
    PublicationProgress {
        step,
        candidate_lock_sha256: None,
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}

fn stash_action(
    owner: PreservationOwnerV1,
    phase: PreservationStashPhaseV1,
    root_publication_handoff: Option<PreservationPublicationCandidateV1>,
) -> PendingPreservationActionV1 {
    PendingPreservationActionV1::Stash {
        owner,
        phase,
        stash_id: None,
        stash_object_id: None,
        message: "preserve merge_1".into(),
        head_commit: "a".repeat(40),
        preimage_sha256: "b".repeat(64),
        root_publication_handoff,
    }
}
