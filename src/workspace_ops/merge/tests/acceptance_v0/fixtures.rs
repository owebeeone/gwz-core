use std::collections::BTreeMap;

use crate::artifact::{
    ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact,
    WorkspaceHeader,
};

use super::super::{
    MergeBaseline, MergeExecutionMode, MergeOperationRecord, MergeParticipantRecord,
    MergeTargetKind, OperationState, ParticipantState, PublicationCandidate, PublicationProgress,
    PublicationStep,
};

pub(super) fn participant(
    state: ParticipantState,
    before: &str,
    result: Option<&str>,
) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: "member".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: before.to_owned(),
        source_commit: result.unwrap_or(before).to_owned(),
        commit_message: "merge".to_owned(),
        state,
        resulting_commit: result.map(str::to_owned),
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

pub(super) fn record() -> MergeOperationRecord {
    MergeOperationRecord {
        schema: "gwz.merge-operation/v0".to_owned(),
        record_schema_version: 0,
        writer_version: "test".to_owned(),
        workspace_id: "ws_test".to_owned(),
        merge_id: "merge_test".to_owned(),
        operation_id: "op_test".to_owned(),
        state: OperationState::Finalizing,
        source_ref: "feature".to_owned(),
        mode: MergeExecutionMode::Normal,
        created_at: "2026-08-04T00:00:00Z".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: "lock".to_owned(),
            manifest_sha256: "manifest".to_owned(),
            lock_yaml: None,
            manifest_yaml: None,
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: Some("root-before".to_owned()),
            root_branch: Some("main".to_owned()),
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_one".to_owned()],
        participants: BTreeMap::from([(
            "mem_one".to_owned(),
            participant(ParticipantState::FastForwarded, "before", Some("after")),
        )]),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

pub(super) fn manifest_and_lock() -> (ManifestArtifact, LockArtifact) {
    let member = ManifestMember {
        id: "mem_one".to_owned(),
        path: "member".to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: "src_one".to_owned(),
        active: true,
        desired: None,
        remotes: Vec::new(),
    };
    let resolved = ResolvedMemberArtifact {
        path: member.path.clone(),
        source_id: Some(member.source_id.clone()),
        source_kind: member.source_kind,
        commit: Some("before".to_owned()),
        branch: Some("main".to_owned()),
        detached: Some(false),
        upstream: Some("origin/main".to_owned()),
        dirty: Some(true),
        materialized: Some(true),
    };
    (
        ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: "ws_test".to_owned(),
            },
            members: vec![member],
        },
        LockArtifact {
            schema: crate::artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_test".to_owned(),
            manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            members: BTreeMap::from([("mem_one".to_owned(), resolved)]),
        },
    )
}

fn candidate() -> PublicationCandidate {
    PublicationCandidate {
        marker_id: "marker".to_owned(),
        root_branch: "main".to_owned(),
        actor_id: "actor".to_owned(),
        baseline_lock_yaml: "baseline".to_owned(),
        lock_yaml: "candidate".to_owned(),
        marker_yaml: "marker".to_owned(),
        baseline_boundary_text: "baseline-boundary".to_owned(),
        boundary_text: "candidate-boundary".to_owned(),
        baseline_boundary_sha256: super::super::publication::sha256(b"baseline-boundary"),
        marker_sha256: super::super::publication::sha256(b"marker"),
        boundary_sha256: super::super::publication::sha256(b"candidate-boundary"),
        extensions: BTreeMap::new(),
    }
}

pub(super) fn progress(step: PublicationStep, with_candidate: bool) -> PublicationProgress {
    let candidate = with_candidate.then(candidate);
    PublicationProgress {
        step,
        candidate_lock_sha256: candidate
            .as_ref()
            .map(|_| super::super::publication::sha256(b"candidate")),
        candidate_marker_path: candidate.as_ref().map(|_| "marker".to_owned()),
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}
