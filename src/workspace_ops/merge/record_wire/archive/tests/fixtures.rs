use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::super::super::super::model::v1::*;
use super::super::super::super::*;
use super::super::MergeOperationRecordV0;
use crate::artifact::{
    ArtifactSourceKind, CreatedByArtifact, LOCK_PATH, LOCK_SCHEMA, LockArtifact, MARKER_DIR,
    MARKER_SCHEMA, ManifestArtifact, ManifestMember, MarkerArtifact, MarkerMergeArtifact,
    MarkerMergeParticipantArtifact, MarkerMergeTargetKind, MarkerRootArtifact,
    ResolvedMemberArtifact, WORKSPACE_SCHEMA, WorkspaceHeader,
};

pub(in crate::workspace_ops::merge::record_wire::archive) const MERGE_ID: &str = "merge_archive";
const MARKER_ID: &str = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";

#[derive(Clone, Copy)]
pub(in crate::workspace_ops::merge::record_wire::archive) enum Shape {
    CompletedCandidate,
    CompletedNoPublication,
    AbortedPreAcceptance,
    AbortedCompleteCandidate,
    AbortedPartialCandidate,
}

pub(super) fn oid(value: char) -> String {
    value.to_string().repeat(40)
}

pub(super) fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn manifest_yaml() -> String {
    ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_archive".to_owned(),
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
    .unwrap()
}

fn resolved(commit: &str) -> ResolvedMemberArtifact {
    resolved_member("members/a", "src_a", commit)
}

fn resolved_member(path: &str, source_id: &str, commit: &str) -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: path.to_owned(),
        source_id: Some(source_id.to_owned()),
        source_kind: ArtifactSourceKind::Git,
        commit: Some(commit.to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: None,
        dirty: Some(false),
        materialized: Some(true),
    }
}

fn lock_yaml(commit: &str) -> String {
    LockArtifact {
        schema: LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_archive".to_owned(),
        manifest_schema: WORKSPACE_SCHEMA.to_owned(),
        members: BTreeMap::from([("mem_a".to_owned(), resolved(commit))]),
    }
    .to_yaml()
    .unwrap()
}

fn participant(state: ParticipantState, result: Option<String>) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: "members/a".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: oid('a'),
        source_commit: oid('b'),
        commit_message: format!(
            "merge topic\n\nGWZ-Merge-ID: {MERGE_ID}\nGWZ-Operation-ID: op_archive"
        ),
        state,
        resulting_commit: result,
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

fn candidate(record: &MergeOperationRecordV0, accepted_lock: &str) -> PublicationCandidate {
    let members = LockArtifact::from_yaml(accepted_lock).unwrap().members;
    let merge_participants = record
        .participants
        .iter()
        .map(|(target_id, participant)| {
            (
                target_id.clone(),
                MarkerMergeParticipantArtifact {
                    target_kind: if target_id == "@root" {
                        MarkerMergeTargetKind::Root
                    } else {
                        MarkerMergeTargetKind::Member
                    },
                    target_branch: participant.target_branch.clone(),
                    before_commit: participant.before_commit.clone(),
                    source_commit: participant.source_commit.clone(),
                    resulting_commit: participant.resulting_commit.clone().unwrap(),
                },
            )
        })
        .collect();
    let marker_yaml = MarkerArtifact {
        schema: MARKER_SCHEMA.to_owned(),
        gwz_commit_id: MARKER_ID.to_owned(),
        workspace_id: record.workspace_id.clone(),
        origin_url_hash: None,
        created_at: record.created_at.clone(),
        created_by: CreatedByArtifact {
            actor_id: "agent_archive".to_owned(),
        },
        root: MarkerRootArtifact {
            path: ".".to_owned(),
            before_commit: record.baseline.root_head.clone(),
            branch: Some("main".to_owned()),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets: vec!["mem_a".to_owned(), "@root".to_owned()],
        members,
        merge: Some(MarkerMergeArtifact {
            merge_id: record.merge_id.clone(),
            operation_id: record.operation_id.clone(),
            source_ref: record.source_ref.clone(),
            selected_targets: record.selected_targets.clone(),
            participants: merge_participants,
            root_merge_commit: record
                .participants
                .get("@root")
                .and_then(|root| root.resulting_commit.clone()),
        }),
    }
    .to_yaml()
    .unwrap();
    PublicationCandidate {
        marker_id: MARKER_ID.to_owned(),
        root_branch: "main".to_owned(),
        actor_id: "agent_archive".to_owned(),
        baseline_lock_yaml: record.baseline.lock_yaml.clone().unwrap(),
        lock_yaml: accepted_lock.to_owned(),
        marker_yaml: marker_yaml.clone(),
        baseline_boundary_text: String::new(),
        boundary_text: String::new(),
        baseline_boundary_sha256: digest(""),
        marker_sha256: digest(&marker_yaml),
        boundary_sha256: digest(""),
        extensions: BTreeMap::new(),
    }
}

fn candidate_publication(
    record: &MergeOperationRecordV0,
    partial: bool,
    rolled_back: bool,
) -> PublicationProgress {
    let accepted_lock = lock_yaml(&oid('d'));
    let candidate = candidate(record, &accepted_lock);
    let marker_path = format!("{MARKER_DIR}/{MARKER_ID}.yaml");
    PublicationProgress {
        step: if partial {
            PublicationStep::PreparingCandidate
        } else {
            PublicationStep::Complete
        },
        candidate_lock_sha256: Some(digest(&accepted_lock)),
        candidate_marker_path: Some(marker_path.clone()),
        root_merge_commit: None,
        composition_commit: (!partial).then(|| oid('e')),
        composition_tree: (!partial).then(|| oid('f')),
        candidate_hashes: if partial {
            Vec::new()
        } else {
            vec![
                PublicationCandidateHash {
                    path: LOCK_PATH.to_owned(),
                    sha256: digest(&accepted_lock),
                },
                PublicationCandidateHash {
                    path: marker_path,
                    sha256: digest(&candidate.marker_yaml),
                },
            ]
        },
        candidate: Some(candidate),
        evidence_rolled_back: rolled_back,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}

pub(in crate::workspace_ops::merge::record_wire::archive) fn v0_record(
    shape: Shape,
) -> MergeOperationRecordV0 {
    let manifest = manifest_yaml();
    let baseline_lock = lock_yaml(&oid('a'));
    let (state, participant_state, result) = match shape {
        Shape::CompletedCandidate => (
            OperationState::Completed,
            ParticipantState::Merged,
            Some(oid('d')),
        ),
        Shape::CompletedNoPublication => (
            OperationState::Completed,
            ParticipantState::UpToDate,
            Some(oid('a')),
        ),
        Shape::AbortedPreAcceptance => (
            OperationState::Aborted,
            ParticipantState::Aborted,
            Some(oid('a')),
        ),
        Shape::AbortedCompleteCandidate | Shape::AbortedPartialCandidate => (
            OperationState::Aborted,
            ParticipantState::RolledBack,
            Some(oid('d')),
        ),
    };
    let mut record = MergeOperationRecordV0 {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: "0.10.3".to_owned(),
        workspace_id: "ws_archive".to_owned(),
        merge_id: MERGE_ID.to_owned(),
        operation_id: "op_archive".to_owned(),
        state,
        source_ref: "feature/topic".to_owned(),
        mode: MergeExecutionMode::Normal,
        created_at: "2026-08-04T00:00:00Z".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: digest(&baseline_lock),
            manifest_sha256: digest(&manifest),
            lock_yaml: Some(baseline_lock),
            manifest_yaml: Some(manifest),
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: Some(oid('c')),
            root_branch: Some("main".to_owned()),
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_a".to_owned()],
        participants: BTreeMap::from([(
            "mem_a".to_owned(),
            participant(participant_state, result),
        )]),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    record.publication = match shape {
        Shape::CompletedCandidate => Some(candidate_publication(&record, false, false)),
        Shape::CompletedNoPublication => Some(PublicationProgress {
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
        }),
        Shape::AbortedPreAcceptance => None,
        Shape::AbortedCompleteCandidate => Some(candidate_publication(&record, false, true)),
        Shape::AbortedPartialCandidate => Some(candidate_publication(&record, true, false)),
    };
    record
}

pub(super) fn bytes(record: &MergeOperationRecordV0) -> Vec<u8> {
    serde_yaml::to_string(record).unwrap().into_bytes()
}

pub(super) fn add_unselected_member(record: &mut MergeOperationRecordV0) {
    let mut manifest =
        ManifestArtifact::from_yaml(record.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    manifest.members.push(ManifestMember {
        id: "mem_b".to_owned(),
        path: "members/b".to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: "src_b".to_owned(),
        active: true,
        desired: None,
        remotes: Vec::new(),
    });
    let manifest_yaml = manifest.to_yaml().unwrap();
    record.baseline.manifest_sha256 = digest(&manifest_yaml);
    record.baseline.manifest_yaml = Some(manifest_yaml);

    let mut baseline =
        LockArtifact::from_yaml(record.baseline.lock_yaml.as_deref().unwrap()).unwrap();
    baseline.members.insert(
        "mem_b".to_owned(),
        resolved_member("members/b", "src_b", &oid('c')),
    );
    let baseline_yaml = baseline.to_yaml().unwrap();
    record.baseline.lock_sha256 = digest(&baseline_yaml);
    record.baseline.lock_yaml = Some(baseline_yaml.clone());

    let has_candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_some();
    if has_candidate {
        record
            .publication
            .as_mut()
            .unwrap()
            .candidate
            .as_mut()
            .unwrap()
            .baseline_lock_yaml = baseline_yaml;
        rewrite_candidate_lock(record, |lock| {
            lock.members.insert(
                "mem_b".to_owned(),
                resolved_member("members/b", "src_b", &oid('c')),
            );
        });
    }
}

pub(super) fn rewrite_candidate_lock(
    record: &mut MergeOperationRecordV0,
    update: impl FnOnce(&mut LockArtifact),
) {
    let publication = record.publication.as_mut().unwrap();
    let candidate = publication.candidate.as_mut().unwrap();
    let mut lock = LockArtifact::from_yaml(&candidate.lock_yaml).unwrap();
    update(&mut lock);
    candidate.lock_yaml = lock.to_yaml().unwrap();
    publication.candidate_lock_sha256 = Some(digest(&candidate.lock_yaml));

    let mut marker = MarkerArtifact::from_yaml(&candidate.marker_yaml).unwrap();
    marker.members = lock.members;
    candidate.marker_yaml = marker.to_yaml().unwrap();
    candidate.marker_sha256 = digest(&candidate.marker_yaml);

    let marker_path = format!("{MARKER_DIR}/{MARKER_ID}.yaml");
    for hash in &mut publication.candidate_hashes {
        if hash.path == LOCK_PATH {
            hash.sha256 = digest(&candidate.lock_yaml);
        } else if hash.path == marker_path {
            hash.sha256 = candidate.marker_sha256.clone();
        }
    }
}

fn accepted_lock_member(commit: &str) -> AcceptedLockMemberV1 {
    let row = resolved(commit);
    AcceptedLockMemberV1 {
        path: row.path,
        source_id: row.source_id.unwrap(),
        source_kind: row.source_kind,
        commit: row.commit,
        branch: row.branch,
        detached: row.detached,
        upstream: row.upstream,
        dirty: row.dirty,
        materialized: row.materialized,
        extensions: BTreeMap::new(),
    }
}

pub(in crate::workspace_ops::merge::record_wire::archive) fn v1_record(
    shape: Shape,
) -> MergeOperationRecordV1 {
    let v0 = v0_record(shape);
    let accepted = (!matches!(shape, Shape::AbortedPreAcceptance)).then(|| {
        let result = oid('d');
        let lock = lock_yaml(&result);
        AcceptedWorkspaceV1 {
            operation_baseline_lock_sha256: v0.baseline.lock_sha256.clone(),
            metadata_base: AcceptedMetadataBaseV1 {
                source: AcceptedMetadataSourceV1::OperationBaseline,
                manifest_exact_yaml: v0.baseline.manifest_yaml.clone().unwrap(),
                manifest_sha256: v0.baseline.manifest_sha256.clone(),
                lock_exact_yaml: v0.baseline.lock_yaml.clone().unwrap(),
                lock_sha256: v0.baseline.lock_sha256.clone(),
            },
            lock: AcceptedLockV1 {
                exact_yaml: lock,
                sha256: digest(&lock_yaml(&result)),
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
                    lock_member: accepted_lock_member(&result),
                },
            )]),
            root: RootPublicationInputV1 {
                base: AcceptedRootBaseV1::BornAttached {
                    commit: oid('c'),
                    symbolic_branch: "main".to_owned(),
                },
                publication_branch: Some("main".to_owned()),
                baseline_artifact_hashes: RootArtifactHashesV1 {
                    lock_worktree_sha256: v0.baseline.lock_sha256.clone(),
                    manifest_worktree_sha256: v0.baseline.manifest_sha256.clone(),
                    lock_commit_sha256: None,
                    manifest_commit_sha256: None,
                },
            },
        }
    });
    MergeOperationRecordV1 {
        schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
        writer_version: "A1-test".to_owned(),
        workspace_id: v0.workspace_id,
        merge_id: v0.merge_id,
        operation_id: v0.operation_id,
        state: v0.state,
        source_ref: v0.source_ref,
        mode: v0.mode,
        created_at: v0.created_at,
        baseline: v0.baseline,
        selected_targets: v0.selected_targets,
        participants: v0.participants,
        publication: v0.publication,
        operation_drift: v0.operation_drift,
        accepted_workspace: accepted,
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        preservation_publication_handoff: None,
        extensions: BTreeMap::new(),
    }
}

pub(super) fn v1_bytes(record: &MergeOperationRecordV1) -> Vec<u8> {
    serde_yaml::to_string(record).unwrap().into_bytes()
}
