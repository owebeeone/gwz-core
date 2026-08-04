use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::super::super::{
    MergeBaseline, MergeExecutionMode, MergeParticipantRecord, MergeTargetKind, OperationState,
    ParticipantState, PreservationEvidence,
};
use super::super::*;
use super::validate_v1_journal;
use crate::artifact::{
    ArtifactSourceKind, LOCK_SCHEMA, LockArtifact, ManifestArtifact, ManifestMember,
    WORKSPACE_SCHEMA, WorkspaceHeader,
};
use crate::model::ErrorCode;

pub(super) fn oid(byte: char) -> String {
    byte.to_string().repeat(40)
}

pub(super) fn sha(byte: char) -> String {
    byte.to_string().repeat(64)
}

pub(super) fn participant(path: &str, kind: MergeTargetKind) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: path.to_owned(),
        target_kind: kind,
        target_branch: "main".to_owned(),
        before_commit: oid('a'),
        source_commit: oid('b'),
        commit_message: "merge topic\n\nGWZ-Merge-ID: merge_1\nGWZ-Operation-ID: op_1".to_owned(),
        state: ParticipantState::Planned,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

pub(super) fn record() -> MergeOperationRecordV1 {
    let manifest_yaml = ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_test".to_owned(),
        },
        members: vec![ManifestMember {
            id: "mem_a".to_owned(),
            path: "members/a".to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: "src_a".to_owned(),
            active: true,
            desired: None,
            remotes: Vec::new(),
        }],
    }
    .to_yaml()
    .unwrap();
    let lock_yaml = LockArtifact {
        schema: LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_test".to_owned(),
        manifest_schema: WORKSPACE_SCHEMA.to_owned(),
        members: BTreeMap::new(),
    }
    .to_yaml()
    .unwrap();
    MergeOperationRecordV1 {
        schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
        writer_version: "0.11.0".to_owned(),
        workspace_id: "ws_test".to_owned(),
        merge_id: "merge_1".to_owned(),
        operation_id: "op_1".to_owned(),
        state: OperationState::Executing,
        source_ref: "topic".to_owned(),
        mode: MergeExecutionMode::Normal,
        created_at: "2026-08-04T00:00:00Z".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: digest(&lock_yaml),
            manifest_sha256: digest(&manifest_yaml),
            lock_yaml: Some(lock_yaml),
            manifest_yaml: Some(manifest_yaml),
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: Some(oid('c')),
            root_branch: Some("main".to_owned()),
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_a".to_owned()],
        participants: BTreeMap::from([(
            "mem_a".to_owned(),
            participant("members/a", MergeTargetKind::Member),
        )]),
        publication: None,
        operation_drift: Vec::new(),
        accepted_workspace: None,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        extensions: BTreeMap::new(),
    }
}

pub(super) fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn rollback_action() -> PendingRollbackActionV1 {
    PendingRollbackActionV1::Participant {
        member_id: "mem_a".to_owned(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    }
}

fn preservation_action(phase: PreservationStashPhaseV1) -> PendingPreservationActionV1 {
    let ids_present = matches!(
        phase,
        PreservationStashPhaseV1::RestoreRoot
            | PreservationStashPhaseV1::WriteBundle
            | PreservationStashPhaseV1::Complete
    );
    PendingPreservationActionV1::Stash {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        phase,
        stash_id: ids_present.then(|| "stash_merge_1".to_owned()),
        stash_object_id: ids_present.then(|| GitObjectIdV1 {
            algorithm: GitObjectAlgorithmV1::Sha1,
            digest_hex: oid('d'),
        }),
        message: "gwz:stash_merge_1: merge preservation".to_owned(),
        head_commit: oid('a'),
        preimage_sha256: sha('1'),
        root_publication_prefix: matches!(
            phase,
            PreservationStashPhaseV1::NormalizeRoot | PreservationStashPhaseV1::RestoreRoot
        )
        .then_some(PublicationPrefixV1::Boundary),
    }
}

#[test]
fn journal_requires_recovery_context_exactly_in_recovery_required() {
    let mut case = record();
    case.state = OperationState::RecoveryRequired;
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );

    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Executing,
    });
    validate_v1_journal(&case).unwrap();

    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Finalizing,
    });
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );

    case.state = OperationState::Executing;
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );
}

#[test]
fn journal_pending_kinds_are_legal_only_in_their_owned_phase() {
    let mut rollback = record();
    rollback.state = OperationState::RollingBack;
    rollback.participants.get_mut("mem_a").unwrap().state = ParticipantState::Merged;
    rollback
        .participants
        .get_mut("mem_a")
        .unwrap()
        .resulting_commit = Some(oid('c'));
    rollback.pending_rollback = Some(rollback_action());
    validate_v1_journal(&rollback).unwrap();

    rollback.state = OperationState::Preserving;
    assert_eq!(
        validate_v1_journal(&rollback).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );

    let mut preserving = record();
    preserving.state = OperationState::Preserving;
    let participant = preserving.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::UpToDate;
    participant.resulting_commit = Some(oid('a'));
    participant.preservation.push(PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".to_owned()),
        backup_commit: Some(oid('a')),
        stash_id: None,
        stash_object_id: None,
    });
    preserving.pending_preservation =
        Some(preservation_action(PreservationStashPhaseV1::CreateStash));
    validate_v1_journal(&preserving).unwrap();
    preserving.pending_rollback = Some(rollback_action());
    assert_eq!(
        validate_v1_journal(&preserving).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );
}

#[test]
fn journal_recovery_retains_only_the_matching_reverse_owner() {
    let mut case = record();
    case.state = OperationState::RecoveryRequired;
    case.participants.get_mut("mem_a").unwrap().state = ParticipantState::Merged;
    case.participants.get_mut("mem_a").unwrap().resulting_commit = Some(oid('c'));
    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::RollingBack,
    });
    case.pending_rollback = Some(rollback_action());
    validate_v1_journal(&case).unwrap();

    case.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Finalizing,
    });
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RecoveryEvidenceMismatch
    );
}

#[test]
fn journal_rollback_participant_kind_and_terminal_state_are_cross_checked() {
    let mut case = record();
    case.state = OperationState::RollingBack;
    case.participants.get_mut("mem_a").unwrap().state = ParticipantState::Conflicted;
    case.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".to_owned(),
        action: ParticipantRollbackKindV1::AbortConflict,
        terminal_state: ParticipantState::Aborted,
    });
    validate_v1_journal(&case).unwrap();

    let Some(PendingRollbackActionV1::Participant { terminal_state, .. }) =
        case.pending_rollback.as_mut()
    else {
        panic!("test rollback action missing")
    };
    *terminal_state = ParticipantState::RolledBack;
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::RollbackEvidenceMismatch
    );
}

#[test]
fn journal_backup_ref_and_stash_phase_fields_are_derived() {
    let mut case = record();
    case.state = OperationState::Preserving;
    case.participants.get_mut("mem_a").unwrap().state = ParticipantState::UpToDate;
    case.participants.get_mut("mem_a").unwrap().resulting_commit = Some(oid('a'));
    case.pending_preservation = Some(PendingPreservationActionV1::BackupRef {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".to_owned(),
        },
        name: "refs/gwz/merge/merge_1/mem_a/head".to_owned(),
        target_commit: oid('a'),
    });
    validate_v1_journal(&case).unwrap();

    let Some(PendingPreservationActionV1::BackupRef { name, .. }) =
        case.pending_preservation.as_mut()
    else {
        panic!("test backup action missing")
    };
    *name = "refs/gwz/merge/other/mem_a/head".to_owned();
    assert_eq!(
        validate_v1_journal(&case).unwrap_err().code,
        ErrorCode::PreservationEvidenceMismatch
    );

    for phase in [
        PreservationStashPhaseV1::CreateStash,
        PreservationStashPhaseV1::WriteBundle,
        PreservationStashPhaseV1::Complete,
    ] {
        if case
            .participants
            .get("mem_a")
            .unwrap()
            .preservation
            .is_empty()
        {
            case.participants
                .get_mut("mem_a")
                .unwrap()
                .preservation
                .push(PreservationEvidence {
                    backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".to_owned()),
                    backup_commit: Some(oid('a')),
                    stash_id: None,
                    stash_object_id: None,
                });
        }
        if matches!(
            phase,
            PreservationStashPhaseV1::WriteBundle | PreservationStashPhaseV1::Complete
        ) {
            let evidence = case
                .participants
                .get_mut("mem_a")
                .unwrap()
                .preservation
                .first_mut()
                .unwrap();
            evidence.stash_id = Some("stash_merge_1".to_owned());
            evidence.stash_object_id = Some(oid('d'));
        }
        case.pending_preservation = Some(preservation_action(phase));
        validate_v1_journal(&case).unwrap();
    }
}
