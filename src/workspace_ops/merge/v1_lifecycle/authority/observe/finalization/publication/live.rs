use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::*;
use crate::artifact::LOCK_PATH;
use crate::git::GitCandidateFile;
use crate::workspace_ops::merge::acceptance::{
    CandidatePublicationObservation, classify_candidate_publication_for_v1, v1_candidate_files,
};
use crate::workspace_ops::workspace_exclude_path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexForm {
    Pre,
    Staged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PublicationResolution {
    Action(PublicationPhysicalAction),
    Complete,
    Ambiguous,
}

pub(super) fn publication_resolution(
    current: &StoredV1Record,
    observed: Option<(CandidatePublicationPrefix, IndexForm)>,
) -> ModelResult<PublicationResolution> {
    let candidate = current
        .record()
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    Ok(resolve_candidate(candidate, observed))
}

fn resolve_candidate(
    candidate: &crate::workspace_ops::merge::PublicationCandidate,
    observed: Option<(CandidatePublicationPrefix, IndexForm)>,
) -> PublicationResolution {
    let lock_same = candidate.lock_yaml == candidate.baseline_lock_yaml;
    let boundary_same = candidate.boundary_text == candidate.baseline_boundary_text;
    match observed {
        Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre)) => {
            PublicationResolution::Action(PublicationPhysicalAction::WriteMarker)
        }
        Some((CandidatePublicationPrefix::Marker, IndexForm::Pre)) if !lock_same => {
            PublicationResolution::Action(PublicationPhysicalAction::WriteLock)
        }
        Some((CandidatePublicationPrefix::Marker, IndexForm::Pre)) if !boundary_same => {
            PublicationResolution::Action(PublicationPhysicalAction::WriteBoundary)
        }
        Some((CandidatePublicationPrefix::Marker, IndexForm::Pre)) => {
            PublicationResolution::Action(PublicationPhysicalAction::StageIndex)
        }
        Some((CandidatePublicationPrefix::Lock, IndexForm::Pre)) => {
            PublicationResolution::Action(PublicationPhysicalAction::WriteBoundary)
        }
        Some((CandidatePublicationPrefix::Boundary, IndexForm::Pre)) => {
            PublicationResolution::Action(PublicationPhysicalAction::StageIndex)
        }
        Some((CandidatePublicationPrefix::Marker, IndexForm::Staged))
            if lock_same && boundary_same =>
        {
            PublicationResolution::Complete
        }
        Some((CandidatePublicationPrefix::Boundary, IndexForm::Staged)) => {
            PublicationResolution::Complete
        }
        _ => PublicationResolution::Ambiguous,
    }
}

pub(super) fn snapshot<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<Option<(CandidatePublicationPrefix, IndexForm)>> {
    let record = current.record();
    let progress = record.publication.as_ref().unwrap();
    let candidate = progress.candidate.as_ref().unwrap();
    let marker_path = progress.candidate_marker_path.as_ref().unwrap();
    let root = current.location().root();
    let lock = regular_digest(&root.join(LOCK_PATH))?;
    let marker = regular_digest(&root.join(marker_path))?;
    let boundary = regular_digest(&workspace_exclude_path(root))?;
    let (FileDigest::Regular(lock), marker, boundary) = (lock, marker, boundary) else {
        return Ok(None);
    };
    let marker = match marker {
        FileDigest::Missing => None,
        FileDigest::Regular(value) => Some(value),
        FileDigest::Other => return Ok(None),
    };
    let boundary = match boundary {
        FileDigest::Missing => None,
        FileDigest::Regular(value) => Some(value),
        FileDigest::Other => return Ok(None),
    };
    let observation = CandidatePublicationObservation::new(Some(lock), marker, boundary);
    let Some(prefix) = classify_candidate_publication_for_v1(record, &observation)? else {
        return Ok(None);
    };
    let pre = backend.index_entries_match_candidate_files(
        root,
        &[GitCandidateFile {
            path: LOCK_PATH.into(),
            bytes: candidate.baseline_lock_yaml.as_bytes().to_vec(),
        }],
        std::slice::from_ref(marker_path),
    )?;
    let candidate_files = v1_candidate_files(record)?;
    let staged = backend.index_entries_match_candidate_files(root, &candidate_files, &[])?;
    let candidate_paths = candidate_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let worktree_aligned = backend.status(root)?.files.iter().all(|file| {
        let owned = candidate_paths.contains(file.path.as_str())
            || file
                .original_path
                .as_deref()
                .is_some_and(|path| candidate_paths.contains(path));
        !owned || file.worktree_status == " "
    });
    Ok(classify_index_form(pre, staged, worktree_aligned).map(|form| (prefix, form)))
}

fn classify_index_form(pre: bool, staged: bool, worktree_aligned: bool) -> Option<IndexForm> {
    match (pre, staged, worktree_aligned) {
        (true, false, _) => Some(IndexForm::Pre),
        (false, true, true) => Some(IndexForm::Staged),
        _ => None,
    }
}

enum FileDigest {
    Missing,
    Regular(String),
    Other,
}

fn regular_digest(path: &Path) -> ModelResult<FileDigest> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileDigest::Missing);
        }
        Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
    };
    if !metadata.file_type().is_file() {
        return Ok(FileDigest::Other);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            return Ok(FileDigest::Other);
        }
    }
    let bytes =
        fs::read(path).map_err(|error| ModelError::new(ErrorCode::IoError, error.to_string()))?;
    Ok(FileDigest::Regular(format!("{:x}", Sha256::digest(bytes))))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::workspace_ops::merge::PublicationCandidate;

    #[test]
    fn prefix_and_index_resolution_matrix_is_closed() {
        let candidate = candidate(false, false);
        for (prefix, form, expected) in [
            (
                CandidatePublicationPrefix::Baseline,
                IndexForm::Pre,
                PublicationResolution::Action(PublicationPhysicalAction::WriteMarker),
            ),
            (
                CandidatePublicationPrefix::Marker,
                IndexForm::Pre,
                PublicationResolution::Action(PublicationPhysicalAction::WriteLock),
            ),
            (
                CandidatePublicationPrefix::Lock,
                IndexForm::Pre,
                PublicationResolution::Action(PublicationPhysicalAction::WriteBoundary),
            ),
            (
                CandidatePublicationPrefix::Boundary,
                IndexForm::Pre,
                PublicationResolution::Action(PublicationPhysicalAction::StageIndex),
            ),
            (
                CandidatePublicationPrefix::Baseline,
                IndexForm::Staged,
                PublicationResolution::Ambiguous,
            ),
            (
                CandidatePublicationPrefix::Marker,
                IndexForm::Staged,
                PublicationResolution::Ambiguous,
            ),
            (
                CandidatePublicationPrefix::Lock,
                IndexForm::Staged,
                PublicationResolution::Ambiguous,
            ),
            (
                CandidatePublicationPrefix::Boundary,
                IndexForm::Staged,
                PublicationResolution::Complete,
            ),
        ] {
            assert_eq!(
                resolve_candidate(&candidate, Some((prefix, form))),
                expected,
                "{prefix:?}/{form:?}"
            );
        }
        assert_eq!(
            resolve_candidate(&candidate, None),
            PublicationResolution::Ambiguous
        );
    }

    #[test]
    fn degenerate_terminal_marker_has_only_stage_and_complete_successors() {
        let candidate = candidate(true, true);
        assert_eq!(
            resolve_candidate(
                &candidate,
                Some((CandidatePublicationPrefix::Marker, IndexForm::Pre))
            ),
            PublicationResolution::Action(PublicationPhysicalAction::StageIndex)
        );
        assert_eq!(
            resolve_candidate(
                &candidate,
                Some((CandidatePublicationPrefix::Marker, IndexForm::Staged))
            ),
            PublicationResolution::Complete
        );
    }

    #[test]
    fn marker_successor_covers_each_single_unchanged_candidate_component() {
        for (lock_same, boundary_same, expected) in [
            (true, false, PublicationPhysicalAction::WriteBoundary),
            (false, true, PublicationPhysicalAction::WriteLock),
        ] {
            assert_eq!(
                resolve_candidate(
                    &candidate(lock_same, boundary_same),
                    Some((CandidatePublicationPrefix::Marker, IndexForm::Pre))
                ),
                PublicationResolution::Action(expected),
                "lock_same={lock_same}, boundary_same={boundary_same}"
            );
        }
    }

    #[test]
    fn index_form_rejects_every_mixed_or_worktree_diverged_shape() {
        assert_eq!(classify_index_form(true, false, true), Some(IndexForm::Pre));
        assert_eq!(
            classify_index_form(true, false, false),
            Some(IndexForm::Pre)
        );
        assert_eq!(
            classify_index_form(false, true, true),
            Some(IndexForm::Staged)
        );
        for shape in [
            (false, false, false),
            (false, false, true),
            (false, true, false),
            (true, true, false),
            (true, true, true),
        ] {
            assert_eq!(classify_index_form(shape.0, shape.1, shape.2), None);
        }
    }

    fn candidate(lock_same: bool, boundary_same: bool) -> PublicationCandidate {
        PublicationCandidate {
            marker_id: "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91".into(),
            root_branch: "main".into(),
            actor_id: "agent_test".into(),
            baseline_lock_yaml: "baseline lock".into(),
            lock_yaml: if lock_same {
                "baseline lock".into()
            } else {
                "candidate lock".into()
            },
            marker_yaml: "marker".into(),
            marker_sha256: "a".repeat(64),
            baseline_boundary_text: "baseline boundary".into(),
            boundary_text: if boundary_same {
                "baseline boundary".into()
            } else {
                "candidate boundary".into()
            },
            baseline_boundary_sha256: "b".repeat(64),
            boundary_sha256: "c".repeat(64),
            extensions: BTreeMap::new(),
        }
    }
}
