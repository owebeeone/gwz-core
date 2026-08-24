use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::artifact::{
    CreatedByArtifact, LockArtifact, ManifestArtifact, MarkerArtifact, MarkerRootArtifact,
};
use crate::git::GitCandidateFile;
use crate::git::GitHeadState;
use crate::model::{ErrorCode, ModelError, ModelResult};
#[cfg(test)]
use crate::workspace_ops::merge::PublicationProgress;
use crate::workspace_ops::merge::marker::{
    marker_merge_from_v1_acceptance, selected_v1_result_changed,
};
use crate::workspace_ops::merge::model::v1::{AcceptedRootBaseV1, MergeOperationRecordV1};
use crate::workspace_ops::merge::{OperationState, PublicationCandidate, PublicationStep};

pub(in crate::workspace_ops::merge) struct V1CandidateBuildInput<'a> {
    pub(in crate::workspace_ops::merge) marker_id: &'a str,
    pub(in crate::workspace_ops::merge) actor_id: &'a str,
    pub(in crate::workspace_ops::merge) root_head: &'a GitHeadState,
    pub(in crate::workspace_ops::merge) baseline_boundary_text: &'a str,
    pub(in crate::workspace_ops::merge) boundary_text: &'a str,
}

pub(in crate::workspace_ops::merge) struct BuiltV1Candidate {
    pub(in crate::workspace_ops::merge) candidate: PublicationCandidate,
    pub(in crate::workspace_ops::merge) marker_path: String,
    pub(in crate::workspace_ops::merge) lock_sha256: String,
}

pub(in crate::workspace_ops::merge) fn candidate_artifacts(
    record: &MergeOperationRecordV1,
) -> ModelResult<(ManifestArtifact, LockArtifact)> {
    let accepted = accepted(record)?;
    let manifest = ManifestArtifact::from_yaml(&accepted.metadata_base.manifest_exact_yaml)?;
    let lock = LockArtifact::from_yaml(&accepted.lock.exact_yaml)?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(candidate_error(
            record,
            "accepted metadata identifies a different workspace",
        ));
    }
    Ok((manifest, lock))
}

pub(in crate::workspace_ops::merge) fn build_v1_candidate(
    record: &MergeOperationRecordV1,
    input: V1CandidateBuildInput<'_>,
) -> ModelResult<BuiltV1Candidate> {
    if record.state != OperationState::Finalizing
        || record.publication.as_ref().is_none_or(|progress| {
            progress.step != PublicationStep::PreparingCandidate || progress.candidate.is_some()
        })
    {
        return Err(candidate_error(record, "record is not candidate-ready"));
    }
    let accepted = accepted(record)?;
    let (_manifest, lock) = candidate_artifacts(record)?;
    let (evidence_parent, root_branch) = publication_base(record)?;
    let head = input.root_head;
    if head.is_detached
        || head.commit.as_deref() != evidence_parent
        || head.branch.as_deref() != Some(root_branch)
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root changed after acceptance was frozen",
        )
        .with_member("@root", "."));
    }
    let marker_merge = marker_merge_from_v1_acceptance(record)?;
    let mut committed_targets = record
        .selected_targets
        .iter()
        .filter(|target| selected_v1_result_changed(record, target).unwrap_or(true))
        .cloned()
        .collect::<Vec<_>>();
    if !committed_targets.iter().any(|target| target == "@root") {
        committed_targets.push("@root".into());
    }
    let marker = MarkerArtifact {
        schema: crate::artifact::MARKER_SCHEMA.into(),
        gwz_commit_id: input.marker_id.into(),
        workspace_id: record.workspace_id.clone(),
        origin_url_hash: None,
        created_at: record.created_at.clone(),
        created_by: CreatedByArtifact {
            actor_id: input.actor_id.into(),
        },
        root: MarkerRootArtifact {
            path: ".".into(),
            before_commit: evidence_parent.map(str::to_owned),
            branch: Some(root_branch.into()),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets,
        members: lock.members,
        merge: Some(marker_merge),
    };
    let marker_yaml = marker.to_yaml()?;
    let marker_path = format!("{}/{}.yaml", crate::artifact::MARKER_DIR, input.marker_id);
    let candidate = PublicationCandidate {
        marker_id: input.marker_id.into(),
        root_branch: root_branch.into(),
        actor_id: input.actor_id.into(),
        baseline_lock_yaml: accepted.metadata_base.lock_exact_yaml.clone(),
        lock_yaml: accepted.lock.exact_yaml.clone(),
        marker_sha256: digest(&marker_yaml),
        marker_yaml,
        baseline_boundary_text: input.baseline_boundary_text.into(),
        baseline_boundary_sha256: digest(input.baseline_boundary_text),
        boundary_text: input.boundary_text.into(),
        boundary_sha256: digest(input.boundary_text),
        extensions: BTreeMap::new(),
    };
    Ok(BuiltV1Candidate {
        lock_sha256: digest(&candidate.lock_yaml),
        marker_path,
        candidate,
    })
}

fn accepted(
    record: &MergeOperationRecordV1,
) -> ModelResult<&crate::workspace_ops::merge::model::v1::AcceptedWorkspaceV1> {
    record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| candidate_error(record, "accepted workspace is missing"))
}

pub(in crate::workspace_ops::merge) fn publication_base(
    record: &MergeOperationRecordV1,
) -> ModelResult<(Option<&str>, &str)> {
    let root = &accepted(record)?.root;
    let branch = root
        .publication_branch
        .as_deref()
        .ok_or_else(|| candidate_error(record, "accepted publication branch is missing"))?;
    let parent = match &root.base {
        AcceptedRootBaseV1::BornAttached {
            commit,
            symbolic_branch,
        } if symbolic_branch == branch => Some(commit.as_str()),
        AcceptedRootBaseV1::UnbornAttached { symbolic_branch } if symbolic_branch == branch => None,
        AcceptedRootBaseV1::BornDetached { .. }
        | AcceptedRootBaseV1::BornAttached { .. }
        | AcceptedRootBaseV1::UnbornAttached { .. } => {
            return Err(candidate_error(
                record,
                "accepted root cannot publish on the frozen branch",
            ));
        }
    };
    Ok((parent, branch))
}

pub(in crate::workspace_ops::merge) fn candidate_files(
    record: &MergeOperationRecordV1,
) -> ModelResult<Vec<GitCandidateFile>> {
    let publication = record
        .publication
        .as_ref()
        .ok_or_else(|| candidate_error(record, "publication progress is missing"))?;
    let candidate = publication
        .candidate
        .as_ref()
        .ok_or_else(|| candidate_error(record, "publication candidate is missing"))?;
    let marker_path = publication
        .candidate_marker_path
        .as_ref()
        .ok_or_else(|| candidate_error(record, "candidate marker path is missing"))?;
    Ok(vec![
        GitCandidateFile {
            path: crate::artifact::LOCK_PATH.into(),
            bytes: candidate.lock_yaml.as_bytes().to_vec(),
        },
        GitCandidateFile {
            path: marker_path.clone(),
            bytes: candidate.marker_yaml.as_bytes().to_vec(),
        },
    ])
}

pub(in crate::workspace_ops::merge) fn composition_message(
    record: &MergeOperationRecordV1,
) -> String {
    format!(
        "gwz merge: {}\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        record.source_ref, record.merge_id, record.operation_id
    )
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn candidate_error(record: &MergeOperationRecordV1, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::CandidateIntegrityMismatch,
        format!("merge '{}' candidate rejected: {detail}", record.merge_id),
    )
}

#[cfg(test)]
mod tests {
    use super::super::v1::{V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance};
    use super::*;
    use crate::artifact::MarkerArtifact;
    use crate::workspace_ops::merge::ParticipantState;

    #[test]
    fn candidate_consumes_frozen_acceptance_and_exact_root_input() {
        let mut record = crate::workspace_ops::merge::model::v1::test_record();
        record.state = OperationState::Finalizing;
        let row = record.participants.get_mut("mem_a").unwrap();
        row.state = ParticipantState::FastForwarded;
        row.resulting_commit = Some("d".repeat(40));
        let accepted = build_v1_acceptance(
            V1AcceptanceRecord::V1(&record),
            V1AcceptanceMetadata::OperationBaseline,
        )
        .unwrap()
        .into_accepted_workspace();
        let root_head = match &accepted.root.base {
            AcceptedRootBaseV1::BornAttached {
                commit,
                symbolic_branch,
            } => GitHeadState {
                commit: Some(commit.clone()),
                branch: Some(symbolic_branch.clone()),
                is_detached: false,
            },
            _ => panic!("fixture root is attached and born"),
        };
        record.accepted_workspace = Some(accepted.clone());
        record.publication = Some(PublicationProgress {
            step: PublicationStep::PreparingCandidate,
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

        let built = build_v1_candidate(
            &record,
            V1CandidateBuildInput {
                marker_id: "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91",
                actor_id: "agent_test",
                root_head: &root_head,
                baseline_boundary_text: "baseline\n",
                boundary_text: "candidate\n",
            },
        )
        .unwrap();

        let marker = MarkerArtifact::from_yaml(&built.candidate.marker_yaml).unwrap();
        assert_eq!(built.candidate.lock_yaml, accepted.lock.exact_yaml);
        assert_eq!(marker.created_by.actor_id, "agent_test");
        assert_eq!(
            marker.merge.unwrap().participants["mem_a"].resulting_commit,
            "d".repeat(40)
        );
        assert_eq!(
            built.marker_path,
            "gwz.conf/markers/01987b0c-2f75-7c4a-9a32-8fd22f7d7c91.yaml"
        );
        assert_eq!(built.lock_sha256, digest(&accepted.lock.exact_yaml));
    }
}
