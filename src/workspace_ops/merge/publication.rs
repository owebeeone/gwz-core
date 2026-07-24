use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::{GitBackend, GitCandidateFile, GitScopedCommitResult};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::workspace_exclude_path;

use super::{MergeOperationRecord, PublicationCandidate, PublicationProgress, PublicationStep};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum CandidatePublicationPrefix {
    Baseline,
    Marker,
    Lock,
    Boundary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RootEvidenceObservation {
    Baseline,
    Composition(GitScopedCommitResult),
}

pub(super) fn classify_candidate_publication(
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    let candidate = candidate(record)?;
    let publication = progress(record)?;
    let lock = file_sha256(&root.join(artifact::LOCK_PATH));
    let marker = file_sha256(&artifact::marker_path(root, &candidate.marker_id));
    let boundary = file_sha256(&workspace_exclude_path(root));
    let baseline_lock = lock.as_deref() == Some(record.baseline.lock_sha256.as_str());
    let candidate_lock = lock.as_deref() == publication.candidate_lock_sha256.as_deref();
    let marker_absent = marker.is_none();
    let candidate_marker = marker.as_deref() == Some(candidate.marker_sha256.as_str());
    let baseline_boundary = boundary.as_deref()
        == Some(candidate.baseline_boundary_sha256.as_str())
        || (boundary.is_none() && candidate.baseline_boundary_text.is_empty());
    let candidate_boundary = boundary.as_deref() == Some(candidate.boundary_sha256.as_str());

    Ok(if baseline_lock && marker_absent && baseline_boundary {
        Some(CandidatePublicationPrefix::Baseline)
    } else if baseline_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Marker)
    } else if candidate_lock && candidate_marker && candidate_boundary {
        Some(CandidatePublicationPrefix::Boundary)
    } else if candidate_lock && candidate_marker && baseline_boundary {
        Some(CandidatePublicationPrefix::Lock)
    } else {
        None
    })
}

pub(super) fn publication_prefix_allowed(
    record: &MergeOperationRecord,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    Ok(match progress(record)?.step {
        PublicationStep::NotStarted
        | PublicationStep::ValidatingResults
        | PublicationStep::PreparingCandidate
        | PublicationStep::CommittingEvidence => prefix == CandidatePublicationPrefix::Baseline,
        PublicationStep::PublishingCandidate => true,
        PublicationStep::VerifyingPublication | PublicationStep::Complete => {
            prefix == CandidatePublicationPrefix::Boundary
        }
    })
}

pub(super) fn observe_root_evidence<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<RootEvidenceObservation>> {
    let head = backend.head(root)?;
    let expected_branch = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .map(|candidate| candidate.root_branch.as_str())
        .or(record.baseline.root_branch.as_deref());
    if head.is_detached
        || expected_branch.is_some_and(|branch| head.branch.as_deref() != Some(branch))
    {
        return Ok(None);
    }
    if head.commit == record.baseline.root_head {
        return Ok(Some(RootEvidenceObservation::Baseline));
    }
    let Some(commit) = head.commit.as_deref() else {
        return Ok(None);
    };
    if record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_deref())
        .is_some_and(|recorded| recorded != commit)
    {
        return Ok(None);
    }
    let result = backend.verify_gwz_paths_commit(
        root,
        commit,
        record.baseline.root_head.as_deref(),
        &candidate_files(record)?,
        &composition_message(record),
    );
    Ok(result.ok().map(RootEvidenceObservation::Composition))
}

pub(super) fn candidate_files(record: &MergeOperationRecord) -> ModelResult<Vec<GitCandidateFile>> {
    let candidate = candidate(record)?;
    Ok(vec![
        GitCandidateFile {
            path: artifact::LOCK_PATH.to_owned(),
            bytes: candidate.lock_yaml.as_bytes().to_vec(),
        },
        GitCandidateFile {
            path: progress(record)?
                .candidate_marker_path
                .clone()
                .ok_or_else(|| unreadable("candidate marker path is missing"))?,
            bytes: candidate.marker_yaml.as_bytes().to_vec(),
        },
    ])
}

pub(super) fn composition_message(record: &MergeOperationRecord) -> String {
    format!(
        "gwz merge: {}\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        record.source_ref, record.merge_id, record.operation_id
    )
}

pub(super) fn candidate(record: &MergeOperationRecord) -> ModelResult<&PublicationCandidate> {
    progress(record)?
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))
}

pub(super) fn progress(record: &MergeOperationRecord) -> ModelResult<&PublicationProgress> {
    record
        .publication
        .as_ref()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

pub(super) fn file_sha256(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| sha256(&bytes))
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}
