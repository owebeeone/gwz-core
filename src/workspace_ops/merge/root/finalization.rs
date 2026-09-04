use std::path::Path;

use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::publication::RootEvidenceObservation;
use super::super::publication::{
    candidate_files_view, classify_candidate_publication_view, observe_root_evidence_view,
};
use super::super::{MergeStatusRecordView, MergeTargetKind, OperationState, participant_semantics};

pub(in crate::workspace_ops::merge) fn evidence_parent_view(
    view: super::super::status::MergeStatusRecordView<'_>,
) -> ModelResult<Option<&str>> {
    Ok(match view.selected_root_participant()? {
        Some(participant) => Some(
            participant
                .resulting_commit
                .as_deref()
                .ok_or_else(|| unreadable("root participant has no resulting commit"))?,
        ),
        None => view.baseline().root_head.as_deref(),
    })
}

pub(in crate::workspace_ops::merge) fn root_finalization_is_exact_view<B: GitBackend>(
    backend: &B,
    root: &Path,
    view: super::super::status::MergeStatusRecordView<'_>,
) -> ModelResult<bool> {
    if view
        .publication()
        .and_then(|publication| publication.candidate.as_ref())
        .is_none()
    {
        return Ok(false);
    }
    if view.selected_root_participant()?.is_none() {
        return Ok(false);
    }
    if !matches!(
        super::super::publication::observe_root_evidence_view(backend, root, view)?,
        Some(RootEvidenceObservation::Composition(_))
    ) {
        return Ok(false);
    }
    let Some(prefix) = super::super::publication::classify_candidate_publication_view(root, view)?
    else {
        return Ok(false);
    };
    if !super::super::publication::publication_prefix_allowed_view(view, prefix)?
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = super::super::publication::candidate_files_view(view)?
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    let status = backend.status(root)?;
    Ok(status.unresolved == 0
        && status
            .files
            .iter()
            .all(|file| allowed.iter().any(|path| path == &file.path)))
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

/// Whether the workspace shows an interrupted root-evidence rollback exactly.
///
/// **M5d.** Relocated from `merge/root/abort.rs`, whose v0 abort surface is
/// deleted. This is a status OBSERVATION, not a rollback step: it is what
/// `status/snapshot.rs` asks before it lets `RollingBack` normalise, and it
/// reads through the version-agnostic `MergeStatusRecordView`.
pub(in crate::workspace_ops::merge) fn interrupted_evidence_rollback_is_exact_view<
    B: GitBackend,
>(
    backend: &B,
    root: &Path,
    view: MergeStatusRecordView<'_>,
) -> ModelResult<bool> {
    let Some(publication) = view.publication() else {
        return Ok(false);
    };
    let Some(participant) = view.participants().get("@root") else {
        return Ok(false);
    };
    if view.state() != OperationState::RollingBack
        || publication.candidate.is_none()
        || publication.composition_commit.is_none()
        || publication.evidence_rolled_back
        || participant.target_kind != MergeTargetKind::Root
        || participant.path != "."
        || !view
            .selected_targets()
            .iter()
            .any(|target| target == "@root")
        || !participant_semantics::result::is_successful_result(participant.state)
        || !matches!(
            observe_root_evidence_view(backend, root, view)?,
            Some(RootEvidenceObservation::Baseline)
        )
        || classify_candidate_publication_view(root, view)?.is_none()
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = candidate_files_view(view)?
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
