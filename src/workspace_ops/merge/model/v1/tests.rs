use std::collections::BTreeMap;
use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_yaml::Value;

use super::super::{MergeBaseline, MergeExecutionMode, OperationState, ParticipantState};
use super::*;
use crate::artifact::ArtifactSourceKind;

fn assert_yaml_shape<T>(value: &T, expected: &str)
where
    T: Debug + PartialEq + Serialize + DeserializeOwned,
{
    let encoded = serde_yaml::to_value(value).expect("test value must serialize");
    let expected: Value = serde_yaml::from_str(expected).expect("golden YAML must parse");
    assert_eq!(encoded, expected);
    let decoded: T = serde_yaml::from_value(encoded).expect("test value must deserialize");
    assert_eq!(&decoded, value);
}

fn lock_member() -> AcceptedLockMemberV1 {
    AcceptedLockMemberV1 {
        path: "member-a".to_owned(),
        source_id: "source-a".to_owned(),
        source_kind: ArtifactSourceKind::Git,
        commit: Some("aaaa".to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: None,
        dirty: Some(false),
        materialized: Some(true),
        extensions: BTreeMap::from([("future".to_owned(), Value::Bool(true))]),
    }
}

#[test]
fn accepted_member_variants_have_exact_tagged_shapes() {
    assert_yaml_shape(
        &MemberAcceptanceV1::Selected {
            integration: AcceptedIntegrationRefV1 {
                branch: "main".to_owned(),
                before_commit: "1111".to_owned(),
                resulting_commit: "2222".to_owned(),
            },
            final_checkout: AcceptedAttachedCheckoutV1 {
                branch: "main".to_owned(),
                commit: "2222".to_owned(),
            },
            lock_member: lock_member(),
        },
        r#"
kind: selected
integration:
  branch: main
  before_commit: '1111'
  resulting_commit: '2222'
final_checkout:
  branch: main
  commit: '2222'
lock_member:
  path: member-a
  source_id: source-a
  source_kind: git
  commit: aaaa
  branch: main
  detached: false
  upstream: null
  dirty: false
  materialized: true
  future: true
"#,
    );
    assert_yaml_shape(
        &MemberAcceptanceV1::UnselectedPresent {
            lock_member: lock_member(),
        },
        r#"
kind: unselected_present
lock_member:
  path: member-a
  source_id: source-a
  source_kind: git
  commit: aaaa
  branch: main
  detached: false
  upstream: null
  dirty: false
  materialized: true
  future: true
"#,
    );
    assert_yaml_shape(&MemberAcceptanceV1::Absent, "kind: absent\n");
}

#[test]
fn accepted_metadata_and_root_variants_have_exact_tagged_shapes() {
    assert_yaml_shape(
        &AcceptedMetadataSourceV1::OperationBaseline,
        "kind: operation_baseline\n",
    );
    assert_yaml_shape(
        &AcceptedMetadataSourceV1::SelectedRootResult {
            commit: "aaaa".to_owned(),
        },
        "kind: selected_root_result\ncommit: aaaa\n",
    );

    let hashes = RootArtifactHashesV1 {
        lock_worktree_sha256: "11".repeat(32),
        manifest_worktree_sha256: "22".repeat(32),
        lock_commit_sha256: None,
        manifest_commit_sha256: None,
    };
    let cases = [
        (
            AcceptedRootBaseV1::BornAttached {
                commit: "aaaa".to_owned(),
                symbolic_branch: "main".to_owned(),
            },
            Some("main".to_owned()),
            "born_attached",
        ),
        (
            AcceptedRootBaseV1::BornDetached {
                commit: "aaaa".to_owned(),
            },
            None,
            "born_detached",
        ),
        (
            AcceptedRootBaseV1::UnbornAttached {
                symbolic_branch: "main".to_owned(),
            },
            Some("main".to_owned()),
            "unborn_attached",
        ),
    ];
    for (base, publication_branch, expected_kind) in cases {
        let value = RootPublicationInputV1 {
            base,
            publication_branch,
            baseline_artifact_hashes: hashes.clone(),
        };
        let encoded = serde_yaml::to_value(&value).expect("root input must serialize");
        assert_eq!(
            encoded["base"]["kind"],
            Value::String(expected_kind.to_owned())
        );
        assert_eq!(
            serde_yaml::from_value::<RootPublicationInputV1>(encoded)
                .expect("root input must deserialize"),
            value
        );
    }
}

#[test]
fn accepted_workspace_round_trip_keeps_all_evidence_and_sorted_audit_rows() {
    let accepted = AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: "10".repeat(32),
        metadata_base: AcceptedMetadataBaseV1 {
            source: AcceptedMetadataSourceV1::OperationBaseline,
            manifest_exact_yaml: "schema: gwz.workspace/v0\n".to_owned(),
            manifest_sha256: "20".repeat(32),
            lock_exact_yaml: "schema: gwz.lock/v0\n".to_owned(),
            lock_sha256: "10".repeat(32),
        },
        lock: AcceptedLockV1 {
            exact_yaml: "schema: gwz.lock/v0\n".to_owned(),
            sha256: "30".repeat(32),
        },
        member_audit: BTreeMap::from([
            ("mem_a".to_owned(), MemberAcceptanceV1::Absent),
            (
                "mem_z".to_owned(),
                MemberAcceptanceV1::UnselectedPresent {
                    lock_member: lock_member(),
                },
            ),
        ]),
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::UnbornAttached {
                symbolic_branch: "main".to_owned(),
            },
            publication_branch: Some("main".to_owned()),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: "40".repeat(32),
                manifest_worktree_sha256: "50".repeat(32),
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
            },
        },
    };
    let yaml = serde_yaml::to_string(&accepted).expect("accepted workspace must serialize");
    assert!(yaml.find("mem_a:").unwrap() < yaml.find("mem_z:").unwrap());
    assert!(yaml.contains("kind: operation_baseline"));
    assert!(yaml.contains("kind: unborn_attached"));
    assert_eq!(
        serde_yaml::from_str::<AcceptedWorkspaceV1>(&yaml)
            .expect("accepted workspace must deserialize"),
        accepted
    );
}

#[test]
fn rollback_journal_variants_have_exact_tagged_shapes() {
    assert_yaml_shape(
        &PendingRollbackActionV1::Participant {
            member_id: "mem_a".to_owned(),
            action: ParticipantRollbackKindV1::ResetIntegrated,
            terminal_state: ParticipantState::RolledBack,
        },
        r#"
kind: participant
member_id: mem_a
action: reset_integrated
terminal_state: rolled_back
"#,
    );
    assert_yaml_shape(
        &PendingRollbackActionV1::PublicationEvidence {
            next_step: EvidenceRollbackStepV1::Boundary,
        },
        "kind: publication_evidence\nnext_step: boundary\n",
    );
    assert_yaml_shape(
        &PendingRollbackActionV1::SelectedRootMetadata {
            next_step: RootMetadataRollbackStepV1::Manifest,
        },
        "kind: selected_root_metadata\nnext_step: manifest\n",
    );
}

#[test]
fn preservation_journal_variants_have_exact_tagged_shapes() {
    let owner = PreservationOwnerV1::Participant {
        member_id: "mem_a".to_owned(),
    };
    assert_yaml_shape(
        &PendingPreservationActionV1::BackupRef {
            owner: owner.clone(),
            name: "refs/gwz/merge/m1/mem_a/head".to_owned(),
            target_commit: "aaaa".to_owned(),
        },
        r#"
kind: backup_ref
owner:
  kind: participant
  member_id: mem_a
name: refs/gwz/merge/m1/mem_a/head
target_commit: aaaa
"#,
    );
    assert_yaml_shape(
        &PendingPreservationActionV1::Stash {
            owner: PreservationOwnerV1::PublicationRoot,
            phase: PreservationStashPhaseV1::WriteBundle,
            stash_id: Some("stash_m1".to_owned()),
            stash_object_id: Some(GitObjectIdV1 {
                algorithm: GitObjectAlgorithmV1::Sha1,
                digest_hex: "11".repeat(20),
            }),
            message: "gwz:stash_m1: merge preservation".to_owned(),
            head_commit: "aaaa".to_owned(),
            preimage_sha256: "22".repeat(32),
            root_publication_handoff: Some(PreservationPublicationCandidateV1 {
                prefix: PublicationPrefixV1::Boundary,
                index: PublicationIndexFormV1::Staged,
            }),
        },
        &format!(
            "kind: stash\nowner:\n  kind: publication_root\nphase: write_bundle\nstash_id: stash_m1\nstash_object_id:\n  algorithm: sha1\n  digest_hex: '{}'\nmessage: 'gwz:stash_m1: merge preservation'\nhead_commit: aaaa\npreimage_sha256: '{}'\nroot_publication_handoff:\n  prefix: boundary\n  index: staged\n",
            "11".repeat(20),
            "22".repeat(32)
        ),
    );
    assert_yaml_shape(
        &PendingPreservationActionV1::ResetAttachedRef {
            owner,
            branch: "main".to_owned(),
            expected_commit: "aaaa".to_owned(),
            restore_commit: "bbbb".to_owned(),
            phase: PreservationRefResetPhaseV1::RestoreParent,
            root_publication_handoff: Some(PreservationPublicationCandidateV1 {
                prefix: PublicationPrefixV1::Lock,
                index: PublicationIndexFormV1::Pre,
            }),
        },
        r#"
kind: reset_attached_ref
owner:
  kind: participant
  member_id: mem_a
branch: main
expected_commit: aaaa
restore_commit: bbbb
phase: restore_parent
root_publication_handoff:
  prefix: lock
  index: pre
"#,
    );
}

#[test]
fn recovery_and_phase_scalar_spellings_are_closed() {
    let recovery_cases = [
        (RecoveryOriginStateV1::Executing, "executing"),
        (
            RecoveryOriginStateV1::AwaitingResolution,
            "awaiting_resolution",
        ),
        (RecoveryOriginStateV1::Halted, "halted"),
        (RecoveryOriginStateV1::Finalizing, "finalizing"),
        (RecoveryOriginStateV1::Preserving, "preserving"),
        (RecoveryOriginStateV1::RollingBack, "rolling_back"),
    ];
    for (origin_state, expected) in recovery_cases {
        assert_yaml_shape(
            &RecoveryContextV1 { origin_state },
            &format!("origin_state: {expected}\n"),
        );
    }

    let evidence_steps = [
        (EvidenceRollbackStepV1::EvidenceCommit, "evidence_commit"),
        (EvidenceRollbackStepV1::Boundary, "boundary"),
        (EvidenceRollbackStepV1::Lock, "lock"),
        (EvidenceRollbackStepV1::Marker, "marker"),
        (EvidenceRollbackStepV1::Index, "index"),
        (EvidenceRollbackStepV1::Complete, "complete"),
    ];
    for (step, expected) in evidence_steps {
        assert_yaml_shape(&step, expected);
    }
}

#[test]
fn every_journal_scalar_variant_has_the_frozen_spelling() {
    for (value, expected) in [
        (ParticipantRollbackKindV1::AbortConflict, "abort_conflict"),
        (
            ParticipantRollbackKindV1::ResetIntegrated,
            "reset_integrated",
        ),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (PublicationIndexFormV1::Pre, "pre"),
        (PublicationIndexFormV1::Staged, "staged"),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (
            PreservationPublicationHandoffV1::NoCandidate,
            "kind: no_candidate\n",
        ),
        (
            PreservationPublicationHandoffV1::EvidencePending,
            "kind: evidence_pending\n",
        ),
        (
            PreservationPublicationHandoffV1::Candidate {
                prefix: PublicationPrefixV1::Boundary,
                index: PublicationIndexFormV1::Staged,
            },
            "kind: candidate\nprefix: boundary\nindex: staged\n",
        ),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (RootMetadataRollbackStepV1::Manifest, "manifest"),
        (RootMetadataRollbackStepV1::Lock, "lock"),
        (RootMetadataRollbackStepV1::Complete, "complete"),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (
            PreservationStashPhaseV1::NormalizeParent,
            "normalize_parent",
        ),
        (
            PreservationStashPhaseV1::NormalizeMarker,
            "normalize_marker",
        ),
        (PreservationStashPhaseV1::NormalizeLock, "normalize_lock"),
        (PreservationStashPhaseV1::NormalizeIndex, "normalize_index"),
        (PreservationStashPhaseV1::CreateStash, "create_stash"),
        (PreservationStashPhaseV1::RestoreIndex, "restore_index"),
        (PreservationStashPhaseV1::RestoreLock, "restore_lock"),
        (PreservationStashPhaseV1::RestoreParent, "restore_parent"),
        (PreservationStashPhaseV1::RestoreMarker, "restore_marker"),
        (PreservationStashPhaseV1::WriteBundle, "write_bundle"),
        (PreservationStashPhaseV1::Complete, "complete"),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (PreservationRefResetPhaseV1::PrepareParent, "prepare_parent"),
        (PreservationRefResetPhaseV1::PrepareMarker, "prepare_marker"),
        (PreservationRefResetPhaseV1::PrepareLock, "prepare_lock"),
        (PreservationRefResetPhaseV1::PrepareIndex, "prepare_index"),
        (PreservationRefResetPhaseV1::ResetRef, "reset_ref"),
        (PreservationRefResetPhaseV1::RestoreIndex, "restore_index"),
        (PreservationRefResetPhaseV1::RestoreLock, "restore_lock"),
        (PreservationRefResetPhaseV1::RestoreParent, "restore_parent"),
        (PreservationRefResetPhaseV1::RestoreMarker, "restore_marker"),
        (PreservationRefResetPhaseV1::Complete, "complete"),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (GitObjectAlgorithmV1::Sha1, "sha1"),
        (GitObjectAlgorithmV1::Sha256, "sha256"),
    ] {
        assert_yaml_shape(&value, expected);
    }
    for (value, expected) in [
        (PublicationPrefixV1::Baseline, "baseline"),
        (PublicationPrefixV1::Marker, "marker"),
        (PublicationPrefixV1::Lock, "lock"),
        (PublicationPrefixV1::Boundary, "boundary"),
    ] {
        assert_yaml_shape(&value, expected);
    }
}

#[test]
fn retired_compound_preservation_phase_spellings_are_rejected() {
    for spelling in ["normalize_root", "restore_root"] {
        assert!(serde_yaml::from_str::<PreservationStashPhaseV1>(spelling).is_err());
        assert!(serde_yaml::from_str::<PreservationRefResetPhaseV1>(spelling).is_err());
    }
}

#[test]
fn complete_v1_record_keeps_v0_shape_and_omits_absent_v1_fields() {
    let record = MergeOperationRecordV1 {
        schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
        writer_version: "0.11.0".to_owned(),
        workspace_id: "ws".to_owned(),
        merge_id: "merge_1".to_owned(),
        operation_id: "op_1".to_owned(),
        state: OperationState::Finalizing,
        source_ref: "topic".to_owned(),
        mode: MergeExecutionMode::Normal,
        created_at: "2026-08-04T00:00:00Z".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: "11".repeat(32),
            manifest_sha256: "22".repeat(32),
            lock_yaml: Some("schema: gwz.lock/v1\n".to_owned()),
            manifest_yaml: Some("schema: gwz.workspace/v1\n".to_owned()),
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: None,
            root_branch: Some("main".to_owned()),
            extensions: BTreeMap::new(),
        },
        selected_targets: Vec::new(),
        participants: BTreeMap::new(),
        publication: None,
        operation_drift: Vec::new(),
        accepted_workspace: None,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        preservation_publication_handoff: None,
        extensions: BTreeMap::from([("future".to_owned(), Value::String("kept".to_owned()))]),
    };
    let encoded = serde_yaml::to_value(&record).expect("v1 record must serialize in tests");
    assert_eq!(
        encoded["schema"],
        Value::String(MERGE_RECORD_SCHEMA_V1.to_owned())
    );
    assert_eq!(encoded["record_schema_version"], Value::Number(1.into()));
    for absent in [
        "accepted_workspace",
        "recovery_context",
        "pending_rollback",
        "pending_preservation",
        "preservation_publication_handoff",
    ] {
        assert!(encoded.get(absent).is_none(), "{absent} must be omitted");
    }
    assert_eq!(encoded["future"], Value::String("kept".to_owned()));
    assert_eq!(
        serde_yaml::from_value::<MergeOperationRecordV1>(encoded)
            .expect("v1 record must deserialize"),
        record
    );
}
