use super::super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeTargetKind, OperationState,
    participant_semantics,
    publication::{
        RootEvidenceObservation, candidate_files, classify_candidate_publication,
        observe_root_evidence,
    },
};
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::ModelResult;
use std::path::Path;

pub(in crate::workspace_ops::merge) fn interrupted_evidence_rollback_is_exact<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<bool> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(false);
    };
    let Some(participant) = record.participants.get("@root") else {
        return Ok(false);
    };
    if record.state != OperationState::RollingBack
        || publication.candidate.is_none()
        || publication.composition_commit.is_none()
        || publication.evidence_rolled_back
        || participant.target_kind != MergeTargetKind::Root
        || participant.path != "."
        || !record
            .selected_targets
            .iter()
            .any(|target| target == "@root")
        || !participant_semantics::result::is_successful_result(participant.state)
        || !matches!(
            observe_root_evidence(backend, root, record)?,
            Some(RootEvidenceObservation::Baseline)
        )
        || classify_candidate_publication(root, record)?.is_none()
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = candidate_files(record)?
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    let status = backend.status(root)?;
    Ok(status.unresolved == 0
        && status.files.iter().all(|file| {
            allowed.iter().any(|path| path == &file.path)
                && file
                    .original_path
                    .as_ref()
                    .is_none_or(|original| allowed.iter().any(|path| path == original))
        }))
}

pub(in crate::workspace_ops::merge) fn normalize_evidence_observation(
    snapshot: &mut MergeStatusSnapshot,
) -> ModelResult<()> {
    participant_semantics::status::apply_interrupted_root_rollback_override(snapshot)
}
