use sha2::{Digest, Sha256};

use super::super::super::{
    MergeRecordError, MergeTargetKind, OperationState, ParticipantState, PendingMergeActionKind,
    PendingMergeExpectedResult, PublicationCandidateHash, PublicationStep,
};
use super::super::{
    AcceptedMetadataSourceV1, EvidenceRollbackStepV1, ParticipantRollbackKindV1,
    PendingPreservationActionV1, PendingRollbackActionV1, PreservationOwnerV1, RecoveryContextV1,
    RecoveryOriginStateV1, RootMetadataRollbackStepV1,
};
use super::tests::{oid, record};
use super::validate_v1_journal;
use crate::model::ErrorCode;

#[test]
fn recovery_origin_is_derived_from_one_exact_base_and_resume_action() {
    let mut cases = Vec::new();

    cases.push((record(), RecoveryOriginStateV1::Executing));

    let mut awaiting = record();
    awaiting.participants.get_mut("mem_a").unwrap().state = ParticipantState::Conflicted;
    cases.push((awaiting, RecoveryOriginStateV1::AwaitingResolution));

    let mut halted = record();
    halted.participants.get_mut("mem_a").unwrap().state = ParticipantState::Failed;
    cases.push((halted, RecoveryOriginStateV1::Halted));

    let mut finalizing = record();
    let participant = finalizing.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::UpToDate;
    participant.resulting_commit = Some(oid('a'));
    cases.push((finalizing, RecoveryOriginStateV1::Finalizing));

    let mut preserving = record();
    preserving.preservation_publication_handoff =
        Some(super::super::PreservationPublicationHandoffV1::NoCandidate);
    let participant = preserving.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::UpToDate;
    participant.resulting_commit = Some(oid('a'));
    preserving.pending_preservation = Some(PendingPreservationActionV1::BackupRef {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        name: "refs/gwz/merge/merge_1/mem_a/head".to_owned(),
        target_commit: oid('a'),
    });
    cases.push((preserving, RecoveryOriginStateV1::Preserving));

    let mut rolling_back = record();
    let participant = rolling_back.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('c'));
    rolling_back.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".to_owned(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    });
    cases.push((rolling_back, RecoveryOriginStateV1::RollingBack));

    for (mut case, origin_state) in cases {
        case.state = OperationState::RecoveryRequired;
        case.recovery_context = Some(RecoveryContextV1 { origin_state });
        validate_v1_journal(&case).unwrap();

        case.recovery_context = Some(RecoveryContextV1 {
            origin_state: if origin_state == RecoveryOriginStateV1::Executing {
                RecoveryOriginStateV1::Finalizing
            } else {
                RecoveryOriginStateV1::Executing
            },
        });
        assert_eq!(
            validate_v1_journal(&case).unwrap_err().code,
            ErrorCode::RecoveryEvidenceMismatch
        );
    }
}

#[test]
fn recovery_halt_origin_precedes_a_retained_forward_retry_action() {
    let mut case = record();
    case.state = OperationState::RecoveryRequired;
    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Halted,
    });
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Failed;
    participant.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "failed".to_owned(),
        detail: None,
    });
    participant.pending_action = Some(super::action_tests::pending(
        PendingMergeActionKind::FastForward,
        PendingMergeExpectedResult::FastForward,
        false,
    ));
    validate_v1_journal(&case).unwrap();
}

#[test]
fn every_publication_rollback_step_requires_complete_unretired_evidence() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    case.state = OperationState::RollingBack;
    case.publication = Some(super::acceptance_tests::valid_candidate_publication_for_tests(&case));
    let publication = case.publication.as_mut().unwrap();
    publication.step = PublicationStep::CommittingEvidence;
    publication.composition_commit = Some(oid('e'));
    publication.composition_tree = Some(oid('f'));
    let candidate = publication.candidate.as_ref().unwrap();
    let mut hashes = vec![
        PublicationCandidateHash {
            path: crate::artifact::LOCK_PATH.to_owned(),
            sha256: digest(&candidate.lock_yaml),
        },
        PublicationCandidateHash {
            path: publication.candidate_marker_path.clone().unwrap(),
            sha256: digest(&candidate.marker_yaml),
        },
    ];
    hashes.sort_by(|left, right| left.path.cmp(&right.path));
    publication.candidate_hashes = hashes;

    for next_step in [
        EvidenceRollbackStepV1::EvidenceCommit,
        EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Lock,
        EvidenceRollbackStepV1::Marker,
        EvidenceRollbackStepV1::Index,
        EvidenceRollbackStepV1::Complete,
    ] {
        case.pending_rollback = Some(PendingRollbackActionV1::PublicationEvidence { next_step });
        validate_v1_journal(&case).unwrap();
    }
    case.publication.as_mut().unwrap().evidence_rolled_back = true;
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RollbackEvidenceMismatch
    );
}

#[test]
fn every_selected_root_metadata_step_requires_selected_root_baseline_ownership() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::RolledBack;
    participant.resulting_commit = Some(oid('d'));
    let mut root = case.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.target_kind = MergeTargetKind::Root;
    root.state = ParticipantState::RolledBack;
    root.resulting_commit = Some(oid('d'));
    case.selected_targets.push("@root".to_owned());
    case.participants.insert("@root".to_owned(), root);
    case.accepted_workspace
        .as_mut()
        .unwrap()
        .metadata_base
        .source = AcceptedMetadataSourceV1::SelectedRootResult { commit: oid('d') };
    case.state = OperationState::RollingBack;

    for next_step in [
        RootMetadataRollbackStepV1::Manifest,
        RootMetadataRollbackStepV1::Lock,
        RootMetadataRollbackStepV1::Complete,
    ] {
        case.pending_rollback = Some(PendingRollbackActionV1::SelectedRootMetadata { next_step });
        validate_v1_journal(&case).unwrap();
    }

    let mut pre_acceptance = case.clone();
    pre_acceptance.accepted_workspace = None;
    validate_v1_journal(&pre_acceptance).unwrap();

    case.accepted_workspace
        .as_mut()
        .unwrap()
        .metadata_base
        .source = AcceptedMetadataSourceV1::OperationBaseline;
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RollbackEvidenceMismatch
    );
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
