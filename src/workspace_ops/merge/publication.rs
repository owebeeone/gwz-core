use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::{GitBackend, GitCandidateFile, GitScopedCommitResult};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::workspace_exclude_path;

pub(super) use super::acceptance::CandidatePublicationPrefix;
use super::acceptance::{
    CandidatePublicationObservation,
    classify_candidate_publication_view as classify_observed_candidate_publication_view,
    publication_prefix_allowed_view as observed_publication_prefix_allowed_view,
};
use super::model::v1::MergeOperationRecordV1;
use super::{PublicationCandidate, PublicationProgress};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RootEvidenceObservation {
    Baseline,
    Composition(GitScopedCommitResult),
}

pub(super) fn classify_candidate_publication(
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    classify_candidate_publication_view(root, super::status::MergeStatusRecordView::from_v1(record))
}

pub(in crate::workspace_ops::merge) fn classify_candidate_publication_view(
    root: &Path,
    view: super::status::MergeStatusRecordView<'_>,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    let candidate = candidate_view(view)?;
    let observation = CandidatePublicationObservation::new(
        file_sha256(&root.join(artifact::LOCK_PATH)),
        file_sha256(&artifact::marker_path(root, &candidate.marker_id)),
        file_sha256(&workspace_exclude_path(root)),
    );
    classify_observed_candidate_publication_view(view, &observation)
}

pub(super) fn publication_prefix_allowed(
    record: &MergeOperationRecordV1,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    publication_prefix_allowed_view(
        super::status::MergeStatusRecordView::from_v1(record),
        prefix,
    )
}

pub(in crate::workspace_ops::merge) fn publication_prefix_allowed_view(
    view: super::status::MergeStatusRecordView<'_>,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    observed_publication_prefix_allowed_view(view, prefix)
}

pub(super) fn observe_root_evidence<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<Option<RootEvidenceObservation>> {
    observe_root_evidence_view(
        backend,
        root,
        super::status::MergeStatusRecordView::from_v1(record),
    )
}

pub(in crate::workspace_ops::merge) fn observe_root_evidence_view<B: GitBackend>(
    backend: &B,
    root: &Path,
    view: super::status::MergeStatusRecordView<'_>,
) -> ModelResult<Option<RootEvidenceObservation>> {
    let head = backend.head(root)?;
    let expected_branch = view
        .publication()
        .and_then(|publication| publication.candidate.as_ref())
        .map(|candidate| candidate.root_branch.as_str())
        .or(view.baseline().root_branch.as_deref());
    if head.is_detached
        || expected_branch.is_some_and(|branch| head.branch.as_deref() != Some(branch))
    {
        return Ok(None);
    }
    let expected_parent = super::root::evidence_parent_view(view)?;
    if head.commit.as_deref() == expected_parent {
        return Ok(Some(RootEvidenceObservation::Baseline));
    }
    let Some(commit) = head.commit.as_deref() else {
        return Ok(None);
    };
    if view
        .publication()
        .and_then(|publication| publication.composition_commit.as_deref())
        .is_some_and(|recorded| recorded != commit)
    {
        return Ok(None);
    }
    let result = backend.verify_gwz_paths_commit(
        root,
        commit,
        expected_parent,
        &candidate_files_view(view)?,
        &composition_message_view(view),
    );
    Ok(result.ok().map(RootEvidenceObservation::Composition))
}

pub(super) fn candidate_files(record: &MergeOperationRecordV1) -> ModelResult<Vec<GitCandidateFile>> {
    candidate_files_view(super::status::MergeStatusRecordView::from_v1(record))
}

pub(in crate::workspace_ops::merge) fn candidate_files_view(
    view: super::status::MergeStatusRecordView<'_>,
) -> ModelResult<Vec<GitCandidateFile>> {
    let candidate = candidate_view(view)?;
    Ok(vec![
        GitCandidateFile {
            path: artifact::LOCK_PATH.to_owned(),
            bytes: candidate.lock_yaml.as_bytes().to_vec(),
        },
        GitCandidateFile {
            path: progress_view(view)?
                .candidate_marker_path
                .clone()
                .ok_or_else(|| unreadable("candidate marker path is missing"))?,
            bytes: candidate.marker_yaml.as_bytes().to_vec(),
        },
    ])
}

pub(super) fn composition_message(record: &MergeOperationRecordV1) -> String {
    composition_message_view(super::status::MergeStatusRecordView::from_v1(record))
}

pub(in crate::workspace_ops::merge) fn composition_message_view(
    view: super::status::MergeStatusRecordView<'_>,
) -> String {
    format!(
        "gwz merge: {}\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        view.source_ref(),
        view.merge_id(),
        view.operation_id()
    )
}

pub(super) fn candidate(record: &MergeOperationRecordV1) -> ModelResult<&PublicationCandidate> {
    candidate_view(super::status::MergeStatusRecordView::from_v1(record))
}

pub(in crate::workspace_ops::merge) fn candidate_view(
    view: super::status::MergeStatusRecordView<'_>,
) -> ModelResult<&PublicationCandidate> {
    progress_view(view)?
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))
}

pub(in crate::workspace_ops::merge) fn progress_view(
    view: super::status::MergeStatusRecordView<'_>,
) -> ModelResult<&PublicationProgress> {
    view.publication()
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
