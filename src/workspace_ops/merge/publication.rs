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
use super::{MergeOperationRecord, PublicationCandidate, PublicationProgress};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RootEvidenceObservation {
    Baseline,
    Composition(GitScopedCommitResult),
}

pub(super) fn classify_candidate_publication(
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<CandidatePublicationPrefix>> {
    classify_candidate_publication_view(root, super::status::MergeStatusRecordView::from_v0(record))
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
    record: &MergeOperationRecord,
    prefix: CandidatePublicationPrefix,
) -> ModelResult<bool> {
    publication_prefix_allowed_view(
        super::status::MergeStatusRecordView::from_v0(record),
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
    record: &MergeOperationRecord,
) -> ModelResult<Option<RootEvidenceObservation>> {
    observe_root_evidence_view(
        backend,
        root,
        super::status::MergeStatusRecordView::from_v0(record),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum I2RootObservationFailure {
    AcceptanceInputDrift,
    CandidateIntegrityMismatch,
    AmbiguousEvidenceCommit,
    RecordedEvidenceDrift,
    PublicationPrefixMismatch,
}

pub(crate) fn normalized_i2_root_observation<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Result<&'static str, I2RootObservationFailure> {
    let publication = record.publication.as_ref();
    if publication
        .and_then(|value| value.candidate.as_ref())
        .is_none()
    {
        let head = backend
            .head(root)
            .map_err(|_| I2RootObservationFailure::AcceptanceInputDrift)?;
        if head.is_detached
            || head.commit != record.baseline.root_head
            || head.branch != record.baseline.root_branch
        {
            return Err(I2RootObservationFailure::AcceptanceInputDrift);
        }
        let expected_files = [
            GitCandidateFile {
                path: artifact::LOCK_PATH.to_owned(),
                bytes: record
                    .baseline
                    .lock_yaml
                    .as_deref()
                    .ok_or(I2RootObservationFailure::AcceptanceInputDrift)?
                    .as_bytes()
                    .to_vec(),
            },
            GitCandidateFile {
                path: crate::workspace::WORKSPACE_MANIFEST.to_owned(),
                bytes: record
                    .baseline
                    .manifest_yaml
                    .as_deref()
                    .ok_or(I2RootObservationFailure::AcceptanceInputDrift)?
                    .as_bytes()
                    .to_vec(),
            },
        ];
        let exact_index = backend
            .index_matches_candidate_files(root, &expected_files, &[])
            .map_err(|_| I2RootObservationFailure::AcceptanceInputDrift)?;
        if !exact_index {
            return Err(I2RootObservationFailure::AcceptanceInputDrift);
        }
        return Ok("baseline_unborn");
    }
    super::finalize::validate_candidate_for_i2_fixture(record)
        .map_err(|_| I2RootObservationFailure::CandidateIntegrityMismatch)?;
    let index_prefix = super::classify_index_aligned_root_publication_for_i2(backend, root, record)
        .map_err(|_| I2RootObservationFailure::PublicationPrefixMismatch)?
        .ok_or(I2RootObservationFailure::PublicationPrefixMismatch)?;
    let manifest_file = [GitCandidateFile {
        path: crate::workspace::WORKSPACE_MANIFEST.to_owned(),
        bytes: record
            .baseline
            .manifest_yaml
            .as_deref()
            .ok_or(I2RootObservationFailure::AcceptanceInputDrift)?
            .as_bytes()
            .to_vec(),
    }];
    if !backend
        .index_matches_candidate_files(root, &manifest_file, &[])
        .map_err(|_| I2RootObservationFailure::AcceptanceInputDrift)?
    {
        return Err(I2RootObservationFailure::AcceptanceInputDrift);
    }
    if !publication_prefix_allowed(record, index_prefix)
        .map_err(|_| I2RootObservationFailure::PublicationPrefixMismatch)?
    {
        return Err(I2RootObservationFailure::PublicationPrefixMismatch);
    }
    let recorded_composition = publication.and_then(|value| value.composition_commit.as_deref());
    let evidence_failure = || {
        if recorded_composition.is_some() {
            I2RootObservationFailure::RecordedEvidenceDrift
        } else {
            I2RootObservationFailure::AmbiguousEvidenceCommit
        }
    };
    match observe_root_evidence(backend, root, record).map_err(|_| evidence_failure())? {
        Some(RootEvidenceObservation::Baseline) => {
            if recorded_composition.is_some() {
                Err(I2RootObservationFailure::RecordedEvidenceDrift)
            } else {
                Ok("baseline_unborn")
            }
        }
        Some(RootEvidenceObservation::Composition(observed)) => {
            let publication = progress_view(super::status::MergeStatusRecordView::from_v0(record))
                .map_err(|_| evidence_failure())?;
            if publication.composition_commit.is_none() {
                return Ok("unrecorded_evidence");
            }
            if publication.composition_commit.as_deref() != Some(observed.commit.as_str())
                || publication.composition_tree.as_deref() != Some(observed.tree.as_str())
                || publication.candidate_hashes.len() != observed.candidate_hashes.len()
                || !publication
                    .candidate_hashes
                    .iter()
                    .zip(&observed.candidate_hashes)
                    .all(|(recorded, live)| {
                        recorded.path == live.path && recorded.sha256 == live.sha256
                    })
            {
                return Err(I2RootObservationFailure::RecordedEvidenceDrift);
            }
            Ok(if index_prefix == CandidatePublicationPrefix::Boundary {
                "prefix_boundary"
            } else {
                "recorded_evidence"
            })
        }
        None => Err(evidence_failure()),
    }
}

pub(super) fn candidate_files(record: &MergeOperationRecord) -> ModelResult<Vec<GitCandidateFile>> {
    candidate_files_view(super::status::MergeStatusRecordView::from_v0(record))
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

pub(super) fn composition_message(record: &MergeOperationRecord) -> String {
    composition_message_view(super::status::MergeStatusRecordView::from_v0(record))
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

pub(super) fn candidate(record: &MergeOperationRecord) -> ModelResult<&PublicationCandidate> {
    candidate_view(super::status::MergeStatusRecordView::from_v0(record))
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
