use super::*;
use crate::artifact::{self, LOCK_PATH};
use crate::workspace_ops::merge::acceptance::{
    v1_candidate_files, v1_composition_message, v1_publication_base,
};
use crate::workspace_ops::publish_workspace_exclude_candidate;

pub(super) fn publication<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
) -> ModelResult<()> {
    verify_finalization_action(backend, current, action)?;
    let record = current.record();
    let progress = record.publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication progress is missing",
        )
    })?;
    let candidate = progress.candidate.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication candidate is missing",
        )
    })?;
    let root = current.location().root();
    match action {
        PublicationPhysicalAction::EvidenceCommit => {
            let (parent, _) = v1_publication_base(record)?;
            backend.commit_gwz_paths_checked(
                root,
                parent,
                &v1_candidate_files(record)?,
                &v1_composition_message(record),
            )?;
        }
        PublicationPhysicalAction::WriteMarker => {
            let path = progress.candidate_marker_path.as_ref().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    "candidate marker path is missing",
                )
            })?;
            artifact::write_atomic(&root.join(path), &candidate.marker_yaml)?;
        }
        PublicationPhysicalAction::WriteLock => {
            artifact::write_atomic(&root.join(LOCK_PATH), &candidate.lock_yaml)?;
        }
        PublicationPhysicalAction::WriteBoundary => {
            publish_workspace_exclude_candidate(root, &candidate.boundary_text)?;
        }
        PublicationPhysicalAction::StageIndex => {
            let marker = progress.candidate_marker_path.as_deref().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    "candidate marker path is missing",
                )
            })?;
            backend.stage_paths(root, &[LOCK_PATH, marker])?;
        }
    }
    Ok(())
}
