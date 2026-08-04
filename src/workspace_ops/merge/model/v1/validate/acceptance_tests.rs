use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::super::super::{
    OperationState, ParticipantState, PublicationCandidate, PublicationProgress, PublicationStep,
};
use super::super::*;
use super::tests::{oid, record};
use super::validate_v1_acceptance;
use crate::artifact::{
    ArtifactSourceKind, CreatedByArtifact, LOCK_SCHEMA, LockArtifact, MARKER_SCHEMA,
    ManifestArtifact, ManifestMember, MarkerArtifact, MarkerRootArtifact, ResolvedMemberArtifact,
    WORKSPACE_SCHEMA, WorkspaceHeader,
};
use crate::model::ErrorCode;

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn resolved(commit: &str) -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: "members/a".to_owned(),
        source_id: Some("src_a".to_owned()),
        source_kind: ArtifactSourceKind::Git,
        commit: Some(commit.to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: None,
        dirty: Some(false),
        materialized: Some(true),
    }
}

fn accepted_member(commit: &str) -> AcceptedLockMemberV1 {
    AcceptedLockMemberV1 {
        path: "members/a".to_owned(),
        source_id: "src_a".to_owned(),
        source_kind: ArtifactSourceKind::Git,
        commit: Some(commit.to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: None,
        dirty: Some(false),
        materialized: Some(true),
        extensions: BTreeMap::new(),
    }
}

fn manifest(include_member: bool) -> String {
    ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_test".to_owned(),
        },
        members: if include_member {
            vec![ManifestMember {
                id: "mem_a".to_owned(),
                path: "members/a".to_owned(),
                source_kind: ArtifactSourceKind::Git,
                source_id: "src_a".to_owned(),
                active: true,
                desired: None,
                remotes: Vec::new(),
            }]
        } else {
            Vec::new()
        },
    }
    .to_yaml()
    .unwrap()
}

fn lock(commit: Option<&str>) -> String {
    LockArtifact {
        schema: LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_test".to_owned(),
        manifest_schema: WORKSPACE_SCHEMA.to_owned(),
        members: commit
            .map(|commit| BTreeMap::from([("mem_a".to_owned(), resolved(commit))]))
            .unwrap_or_default(),
    }
    .to_yaml()
    .unwrap()
}

pub(super) fn selected_acceptance_record_for_tests() -> MergeOperationRecordV1 {
    let mut record = record();
    let result = oid('d');
    record.state = OperationState::Finalizing;
    record.participants.get_mut("mem_a").unwrap().state = ParticipantState::Merged;
    record
        .participants
        .get_mut("mem_a")
        .unwrap()
        .resulting_commit = Some(result.clone());
    let manifest_yaml = manifest(true);
    let baseline_lock_yaml = lock(Some(&oid('a')));
    let accepted_lock_yaml = lock(Some(&result));
    record.baseline.manifest_sha256 = digest(&manifest_yaml);
    record.baseline.lock_sha256 = digest(&baseline_lock_yaml);
    record.baseline.manifest_yaml = Some(manifest_yaml.clone());
    record.baseline.lock_yaml = Some(baseline_lock_yaml.clone());
    record.accepted_workspace = Some(AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: digest(&baseline_lock_yaml),
        metadata_base: AcceptedMetadataBaseV1 {
            source: AcceptedMetadataSourceV1::OperationBaseline,
            manifest_sha256: digest(&manifest_yaml),
            manifest_exact_yaml: manifest_yaml,
            lock_sha256: digest(&baseline_lock_yaml),
            lock_exact_yaml: baseline_lock_yaml,
        },
        lock: AcceptedLockV1 {
            sha256: digest(&accepted_lock_yaml),
            exact_yaml: accepted_lock_yaml,
        },
        member_audit: BTreeMap::from([(
            "mem_a".to_owned(),
            MemberAcceptanceV1::Selected {
                integration: AcceptedIntegrationRefV1 {
                    branch: "main".to_owned(),
                    before_commit: oid('a'),
                    resulting_commit: result.clone(),
                },
                final_checkout: AcceptedAttachedCheckoutV1 {
                    branch: "main".to_owned(),
                    commit: result.clone(),
                },
                lock_member: accepted_member(&result),
            },
        )]),
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::BornAttached {
                commit: oid('c'),
                symbolic_branch: "main".to_owned(),
            },
            publication_branch: Some("main".to_owned()),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: record.baseline.lock_sha256.clone(),
                manifest_worktree_sha256: record.baseline.manifest_sha256.clone(),
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
            },
        },
    });
    record
}

fn selected_acceptance_record() -> MergeOperationRecordV1 {
    selected_acceptance_record_for_tests()
}

pub(super) fn empty_publication_for_tests(step: PublicationStep) -> PublicationProgress {
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

pub(super) fn valid_candidate_publication_for_tests(
    record: &MergeOperationRecordV1,
) -> PublicationProgress {
    let accepted = record.accepted_workspace.as_ref().unwrap();
    let marker_id = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";
    let marker_yaml = MarkerArtifact {
        schema: MARKER_SCHEMA.to_owned(),
        gwz_commit_id: marker_id.to_owned(),
        workspace_id: record.workspace_id.clone(),
        origin_url_hash: None,
        created_at: record.created_at.clone(),
        created_by: CreatedByArtifact {
            actor_id: "agent_test".to_owned(),
        },
        root: MarkerRootArtifact {
            path: ".".to_owned(),
            before_commit: record.baseline.root_head.clone(),
            branch: Some("main".to_owned()),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets: vec!["mem_a".to_owned(), "@root".to_owned()],
        members: LockArtifact::from_yaml(&accepted.lock.exact_yaml)
            .unwrap()
            .members,
        merge: None,
    }
    .to_yaml()
    .unwrap();
    let marker_path = format!("gwz.conf/markers/{marker_id}.yaml");
    PublicationProgress {
        step: PublicationStep::PreparingCandidate,
        candidate_lock_sha256: Some(accepted.lock.sha256.clone()),
        candidate_marker_path: Some(marker_path),
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: Some(PublicationCandidate {
            marker_id: marker_id.to_owned(),
            root_branch: "main".to_owned(),
            actor_id: "agent_test".to_owned(),
            baseline_lock_yaml: accepted.metadata_base.lock_exact_yaml.clone(),
            lock_yaml: accepted.lock.exact_yaml.clone(),
            marker_sha256: digest(&marker_yaml),
            marker_yaml,
            baseline_boundary_text: String::new(),
            boundary_text: String::new(),
            baseline_boundary_sha256: digest(""),
            boundary_sha256: digest(""),
            extensions: BTreeMap::new(),
        }),
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}

#[test]
fn selected_acceptance_satisfies_the_twelve_cross_field_invariants() {
    validate_v1_acceptance(&selected_acceptance_record()).unwrap();
}

#[test]
fn acceptance_rejects_hash_source_and_selected_result_drift() {
    let mut record = selected_acceptance_record();
    record
        .accepted_workspace
        .as_mut()
        .unwrap()
        .metadata_base
        .manifest_sha256 = "0".repeat(64);
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::AcceptanceInputDrift
    );

    let mut record = selected_acceptance_record();
    let MemberAcceptanceV1::Selected { integration, .. } = record
        .accepted_workspace
        .as_mut()
        .unwrap()
        .member_audit
        .get_mut("mem_a")
        .unwrap()
    else {
        panic!("selected audit missing")
    };
    integration.resulting_commit = oid('e');
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::AcceptanceInputDrift
    );
}

#[test]
fn acceptance_rejects_early_or_unowned_publication_evidence() {
    let mut record = selected_acceptance_record();
    record.state = OperationState::Executing;
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::UnexpectedAcceptanceEvidence
    );

    let mut record = selected_acceptance_record();
    record.accepted_workspace = None;
    record.publication = Some(PublicationProgress {
        step: PublicationStep::PreparingCandidate,
        candidate_lock_sha256: None,
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: Some(oid('e')),
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::UnexpectedAcceptanceEvidence
    );

    let mut no_publication = selected_acceptance_record();
    no_publication.accepted_workspace = None;
    no_publication.publication = Some(PublicationProgress {
        step: PublicationStep::Complete,
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
    });
    assert_eq!(
        validate_v1_acceptance(&no_publication).unwrap_err().code,
        ErrorCode::UnexpectedAcceptanceEvidence
    );
}

#[test]
fn unselected_present_and_manifest_only_absent_rows_are_distinct() {
    let mut record = selected_acceptance_record();
    record.selected_targets.clear();
    record.participants.clear();
    let accepted = record.accepted_workspace.as_mut().unwrap();
    let baseline_row = accepted_member(&oid('a'));
    accepted.member_audit.insert(
        "mem_a".to_owned(),
        MemberAcceptanceV1::UnselectedPresent {
            lock_member: baseline_row.clone(),
        },
    );
    accepted.lock.exact_yaml = accepted.metadata_base.lock_exact_yaml.clone();
    accepted.lock.sha256 = digest(&accepted.lock.exact_yaml);
    validate_v1_acceptance(&record).unwrap();

    let manifest_only_yaml = manifest(true);
    let empty_lock_yaml = lock(None);
    record.baseline.manifest_sha256 = digest(&manifest_only_yaml);
    record.baseline.lock_sha256 = digest(&empty_lock_yaml);
    record.baseline.manifest_yaml = Some(manifest_only_yaml.clone());
    record.baseline.lock_yaml = Some(empty_lock_yaml.clone());
    let accepted = record.accepted_workspace.as_mut().unwrap();
    accepted.operation_baseline_lock_sha256 = digest(&empty_lock_yaml);
    accepted.metadata_base.manifest_exact_yaml = manifest_only_yaml.clone();
    accepted.metadata_base.manifest_sha256 = digest(&manifest_only_yaml);
    accepted.metadata_base.lock_exact_yaml = empty_lock_yaml.clone();
    accepted.metadata_base.lock_sha256 = digest(&empty_lock_yaml);
    accepted.lock.exact_yaml = empty_lock_yaml.clone();
    accepted.lock.sha256 = digest(&empty_lock_yaml);
    accepted
        .member_audit
        .insert("mem_a".to_owned(), MemberAcceptanceV1::Absent);
    accepted.root.baseline_artifact_hashes.lock_worktree_sha256 =
        record.baseline.lock_sha256.clone();
    accepted
        .root
        .baseline_artifact_hashes
        .manifest_worktree_sha256 = record.baseline.manifest_sha256.clone();
    validate_v1_acceptance(&record).unwrap();
}

#[test]
fn candidate_bytes_must_equal_persisted_acceptance_before_r4a_validation() {
    let mut record = selected_acceptance_record();
    record.publication = Some(PublicationProgress {
        step: PublicationStep::PreparingCandidate,
        candidate_lock_sha256: Some("0".repeat(64)),
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: Some(PublicationCandidate {
            marker_id: "marker".to_owned(),
            root_branch: "main".to_owned(),
            actor_id: "actor".to_owned(),
            baseline_lock_yaml: record.baseline.lock_yaml.clone().unwrap(),
            lock_yaml: "different".to_owned(),
            marker_yaml: "different".to_owned(),
            baseline_boundary_text: String::new(),
            boundary_text: String::new(),
            baseline_boundary_sha256: digest(""),
            marker_sha256: digest("different"),
            boundary_sha256: digest(""),
            extensions: BTreeMap::new(),
        }),
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::CandidateIntegrityMismatch
    );
}

#[test]
fn selected_root_metadata_uses_frozen_baseline_member_identity_as_fallback() {
    let mut record = selected_acceptance_record();
    let mut root = record.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.target_kind = super::super::super::MergeTargetKind::Root;
    root.before_commit = oid('c');
    root.resulting_commit = Some(oid('e'));
    record.selected_targets.push("@root".to_owned());
    record.participants.insert("@root".to_owned(), root);
    record.baseline.lock_commit_sha256 = Some(digest(record.baseline.lock_yaml.as_ref().unwrap()));
    record.baseline.manifest_commit_sha256 =
        Some(digest(record.baseline.manifest_yaml.as_ref().unwrap()));

    let accepted = record.accepted_workspace.as_mut().unwrap();
    let empty_manifest = manifest(false);
    let empty_lock = lock(None);
    accepted.metadata_base = AcceptedMetadataBaseV1 {
        source: AcceptedMetadataSourceV1::SelectedRootResult { commit: oid('e') },
        manifest_sha256: digest(&empty_manifest),
        manifest_exact_yaml: empty_manifest,
        lock_sha256: digest(&empty_lock),
        lock_exact_yaml: empty_lock,
    };
    accepted.root.base = AcceptedRootBaseV1::BornAttached {
        commit: oid('e'),
        symbolic_branch: "main".to_owned(),
    };
    accepted.root.baseline_artifact_hashes.lock_commit_sha256 =
        record.baseline.lock_commit_sha256.clone();
    accepted
        .root
        .baseline_artifact_hashes
        .manifest_commit_sha256 = record.baseline.manifest_commit_sha256.clone();
    validate_v1_acceptance(&record).unwrap();

    let MemberAcceptanceV1::Selected { lock_member, .. } = record
        .accepted_workspace
        .as_mut()
        .unwrap()
        .member_audit
        .get_mut("mem_a")
        .unwrap()
    else {
        panic!("selected audit missing")
    };
    lock_member.source_id = "src_other".to_owned();
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::AcceptanceInputDrift
    );
}

#[test]
fn accepted_integration_identity_survives_incremental_and_terminal_rollback() {
    let mut record = selected_acceptance_record();
    record.state = OperationState::RollingBack;
    record.participants.get_mut("mem_a").unwrap().state = ParticipantState::RolledBack;
    validate_v1_acceptance(&record).unwrap();

    record.state = OperationState::Aborted;
    validate_v1_acceptance(&record).unwrap();

    record.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    assert_eq!(
        validate_v1_acceptance(&record).unwrap_err().code,
        ErrorCode::AcceptanceInputDrift
    );
}
