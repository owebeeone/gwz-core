//! The v1 reverse path's evidence-rollback observation and execution.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** This was `mod v1_rollback` inside
//! `merge/abort/evidence.rs`, whose other half was the v0 abort engine's
//! evidence rollback. That engine is deleted, so the v1 half moves here
//! rather than staying in a file named for it.

use super::super::root::artifact_facts;
use crate::artifact;
use crate::git::GitCandidateFile;
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    AcceptedRootBaseV1, EvidenceRollbackStepV1, MergeOperationRecordV1,
};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum V1EvidenceRollbackObservation {
    Before,
    After,
    Ambiguous,
}

pub(in crate::workspace_ops::merge) fn preflight_v1_evidence<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<()> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(());
    };
    if publication.candidate.is_some()
        && publication.composition_commit.is_some()
        && !publication.evidence_rolled_back
        && !evidence_shape_is_exact(backend, root, record)?
    {
        return Err(root_error(
            "publication evidence is not at an exact rollback-representable state",
        ));
    }
    Ok(())
}

pub(in crate::workspace_ops::merge) fn observe_v1_evidence_rollback<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    step: EvidenceRollbackStepV1,
) -> ModelResult<V1EvidenceRollbackObservation> {
    let publication = publication_v1(record)?;
    publication
        .candidate
        .as_ref()
        .ok_or_else(|| root_error("publication-evidence rollback has no immutable candidate"))?;
    let head_before = exact_evidence_head(backend, root, record, true)?;
    let head_after = exact_evidence_head(backend, root, record, false)?;
    let files = file_states(root, record, Some(step))?;
    let index = index_state(backend, root, record)?;
    Ok(classify_v1_evidence_rollback(
        step,
        head_before,
        head_after,
        &files,
        index,
    ))
}

pub(in crate::workspace_ops::merge) fn v1_evidence_residue_after_selected_root_is_exact(
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<bool> {
    let candidate = candidate_v1(record)?;
    let boundary = artifact_facts::observe(root, &boundary_relative(root)?)?;
    let marker = artifact_facts::observe(root, marker_path_v1(record)?)?;
    Ok(boundary
        == artifact_facts::RegularFileFact::Bytes(
            candidate.baseline_boundary_text.as_bytes().to_vec(),
        )
        && marker == artifact_facts::RegularFileFact::Missing)
}

fn classify_v1_evidence_rollback(
    step: EvidenceRollbackStepV1,
    head_before: bool,
    head_after: bool,
    files: &EvidenceFileStates,
    index: FileState,
) -> V1EvidenceRollbackObservation {
    match step {
        EvidenceRollbackStepV1::EvidenceCommit => {
            let exact_files = files.initial_publication(index);
            classify(head_before && exact_files, head_after && exact_files)
        }
        EvidenceRollbackStepV1::Boundary => classify(
            head_after && files.boundary_before(index),
            head_after && files.boundary_after(index),
        ),
        EvidenceRollbackStepV1::Lock => classify(
            head_after && files.lock_before(index),
            head_after && files.lock_after(index),
        ),
        EvidenceRollbackStepV1::Marker => classify(
            head_after && files.marker_before(index),
            head_after && files.marker_after(index),
        ),
        EvidenceRollbackStepV1::Index => classify(
            head_after && files.index_before(index),
            head_after && files.index_after(index),
        ),
        EvidenceRollbackStepV1::Complete => classify(false, head_after && files.index_after(index)),
    }
}

pub(in crate::workspace_ops::merge) fn execute_v1_evidence_rollback<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    step: EvidenceRollbackStepV1,
) -> ModelResult<()> {
    if observe_v1_evidence_rollback(backend, root, record, step)?
        != V1EvidenceRollbackObservation::Before
    {
        return Err(root_error(
            "publication-evidence rollback is not at the exact before state",
        ));
    }
    let candidate = candidate_v1(record)?;
    match step {
        EvidenceRollbackStepV1::EvidenceCommit => backend.rollback_gwz_paths_commit_checked(
            root,
            &candidate.root_branch,
            composition_commit_v1(record)?,
            evidence_parent_v1(record)?,
            &crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?,
            &crate::workspace_ops::merge::acceptance::v1_composition_message(record),
        ),
        EvidenceRollbackStepV1::Boundary => artifact_facts::write_checked(
            root,
            &boundary_relative(root)?,
            candidate.boundary_text.as_bytes(),
            candidate.baseline_boundary_text.as_bytes(),
        ),
        EvidenceRollbackStepV1::Lock => artifact_facts::write_checked(
            root,
            artifact::LOCK_PATH,
            candidate.lock_yaml.as_bytes(),
            candidate.baseline_lock_yaml.as_bytes(),
        ),
        EvidenceRollbackStepV1::Marker => artifact_facts::remove_exact(
            root,
            marker_path_v1(record)?,
            candidate.marker_yaml.as_bytes(),
        ),
        EvidenceRollbackStepV1::Index => backend
            .stage_paths(root, &[artifact::LOCK_PATH, marker_path_v1(record)?])
            .map(|_| ()),
        EvidenceRollbackStepV1::Complete => Err(root_error(
            "complete evidence rollback has no physical mutation",
        )),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileState {
    Candidate,
    Baseline,
    Other,
}

struct EvidenceFileStates {
    boundary: FileState,
    lock: FileState,
    marker: FileState,
    boundary_noop: bool,
    lock_noop: bool,
}

impl EvidenceFileStates {
    fn matches(&self, index: FileState, expected: [FileState; 4]) -> bool {
        [self.boundary, self.lock, self.marker, index]
            .into_iter()
            .zip(expected)
            .zip([self.boundary_noop, self.lock_noop, false, false])
            .all(|((actual, expected), noop)| {
                actual == expected || (noop && actual == B && expected == C)
            })
    }

    fn one_of(&self, index: FileState, expected: &[[FileState; 4]]) -> bool {
        expected.iter().any(|shape| self.matches(index, *shape))
    }

    fn initial_publication(&self, index: FileState) -> bool {
        self.one_of(
            index,
            &[
                [B, B, B, B],
                [B, B, C, B],
                [B, C, C, B],
                [C, B, C, B],
                [C, C, C, B],
                [B, B, C, C],
                [B, C, C, C],
                [C, B, C, C],
                [C, C, C, C],
            ],
        )
    }

    fn boundary_before(&self, index: FileState) -> bool {
        !self.boundary_noop
            && self.one_of(
                index,
                &[[C, B, C, B], [C, C, C, B], [C, B, C, C], [C, C, C, C]],
            )
    }

    fn boundary_after(&self, index: FileState) -> bool {
        self.one_of(
            index,
            &[
                [B, B, B, B],
                [B, B, C, B],
                [B, C, C, B],
                [B, B, C, C],
                [B, C, C, C],
            ],
        )
    }

    fn lock_before(&self, index: FileState) -> bool {
        !self.lock_noop && self.one_of(index, &[[B, C, C, B], [B, C, C, C]])
    }

    fn lock_after(&self, index: FileState) -> bool {
        self.one_of(index, &[[B, B, B, B], [B, B, C, B], [B, B, C, C]])
    }

    fn marker_before(&self, index: FileState) -> bool {
        self.one_of(index, &[[B, B, C, B], [B, B, C, C]])
    }

    fn marker_after(&self, index: FileState) -> bool {
        self.one_of(index, &[[B, B, B, B], [B, B, B, C]])
    }

    fn index_before(&self, index: FileState) -> bool {
        self.matches(index, [B, B, B, C])
    }

    fn index_after(&self, index: FileState) -> bool {
        self.matches(index, [B, B, B, B])
    }
}

use FileState::{Baseline as B, Candidate as C};

fn file_states(
    root: &Path,
    record: &MergeOperationRecordV1,
    pending: Option<EvidenceRollbackStepV1>,
) -> ModelResult<EvidenceFileStates> {
    let candidate = candidate_v1(record)?;
    Ok(EvidenceFileStates {
        boundary: if pending == Some(EvidenceRollbackStepV1::Boundary) {
            transition_file(artifact_facts::classify_write(
                root,
                &boundary_relative(root)?,
                candidate.boundary_text.as_bytes(),
                candidate.baseline_boundary_text.as_bytes(),
            )?)
        } else {
            classify_file(
                artifact_facts::observe(root, &boundary_relative(root)?)?,
                candidate.boundary_text.as_bytes(),
                candidate.baseline_boundary_text.as_bytes(),
                false,
            )
        },
        lock: if pending == Some(EvidenceRollbackStepV1::Lock) {
            transition_file(artifact_facts::classify_write(
                root,
                artifact::LOCK_PATH,
                candidate.lock_yaml.as_bytes(),
                candidate.baseline_lock_yaml.as_bytes(),
            )?)
        } else {
            classify_file(
                artifact_facts::observe(root, artifact::LOCK_PATH)?,
                candidate.lock_yaml.as_bytes(),
                candidate.baseline_lock_yaml.as_bytes(),
                false,
            )
        },
        marker: if pending == Some(EvidenceRollbackStepV1::Marker) {
            transition_file(artifact_facts::classify_remove(
                root,
                marker_path_v1(record)?,
                candidate.marker_yaml.as_bytes(),
            )?)
        } else {
            classify_file(
                artifact_facts::observe(root, marker_path_v1(record)?)?,
                candidate.marker_yaml.as_bytes(),
                &[],
                true,
            )
        },
        boundary_noop: candidate.boundary_text == candidate.baseline_boundary_text,
        lock_noop: candidate.lock_yaml == candidate.baseline_lock_yaml,
    })
}

fn transition_file(value: artifact_facts::RegularFileTransition) -> FileState {
    match value {
        artifact_facts::RegularFileTransition::Before
        | artifact_facts::RegularFileTransition::Recoverable => FileState::Candidate,
        artifact_facts::RegularFileTransition::After => FileState::Baseline,
        artifact_facts::RegularFileTransition::Ambiguous => FileState::Other,
    }
}

fn boundary_relative(root: &Path) -> ModelResult<String> {
    let path = crate::workspace_ops::workspace_exclude_path(root);
    path.strip_prefix(root)
        .map_err(|_| root_error("workspace boundary escaped root"))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| root_error("workspace boundary path is not UTF-8"))
}

fn classify_file(
    fact: artifact_facts::RegularFileFact,
    candidate: &[u8],
    baseline: &[u8],
    missing_is_baseline: bool,
) -> FileState {
    match fact {
        artifact_facts::RegularFileFact::Bytes(bytes) if bytes == baseline => FileState::Baseline,
        artifact_facts::RegularFileFact::Bytes(bytes) if bytes == candidate => FileState::Candidate,
        artifact_facts::RegularFileFact::Missing if missing_is_baseline => FileState::Baseline,
        artifact_facts::RegularFileFact::Missing
        | artifact_facts::RegularFileFact::Bytes(_)
        | artifact_facts::RegularFileFact::Invalid => FileState::Other,
    }
}

fn index_state<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<FileState> {
    let candidate = candidate_v1(record)?;
    let marker = marker_path_v1(record)?;
    let before = backend.index_entries_match_candidate_files(
        root,
        &crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?,
        &[],
    )?;
    let after = backend.index_entries_match_candidate_files(
        root,
        &[GitCandidateFile {
            path: artifact::LOCK_PATH.into(),
            bytes: candidate.baseline_lock_yaml.as_bytes().to_vec(),
        }],
        std::slice::from_ref(marker),
    )?;
    Ok(match (before, after) {
        (true, false) => FileState::Candidate,
        (false, true) => FileState::Baseline,
        _ => FileState::Other,
    })
}

fn evidence_shape_is_exact<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<bool> {
    let files = file_states(root, record, None)?;
    let index = index_state(backend, root, record)?;
    Ok(classify_v1_evidence_rollback(
        EvidenceRollbackStepV1::EvidenceCommit,
        exact_evidence_head(backend, root, record, true)?,
        exact_evidence_head(backend, root, record, false)?,
        &files,
        index,
    ) != V1EvidenceRollbackObservation::Ambiguous)
}

fn exact_evidence_head<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
    before: bool,
) -> ModelResult<bool> {
    if backend.repository_state(root)? != GitRepositoryState::Clean {
        return Ok(false);
    }
    let publication = publication_v1(record)?;
    let candidate = candidate_v1(record)?;
    let expected = if before {
        publication.composition_commit.as_deref()
    } else {
        evidence_parent_v1(record)?
    };
    let head = backend.head(root)?;
    let head_matches = !head.is_detached
        && head.branch.as_deref() == Some(candidate.root_branch.as_str())
        && head.commit.as_deref() == expected;
    if before && head_matches {
        match backend.verify_gwz_paths_commit(
            root,
            composition_commit_v1(record)?,
            evidence_parent_v1(record)?,
            &crate::workspace_ops::merge::acceptance::v1_candidate_files(record)?,
            &crate::workspace_ops::merge::acceptance::v1_composition_message(record),
        ) {
            Ok(_) => {}
            Err(error) if semantic_mismatch(&error) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(head_matches)
}

fn evidence_parent_v1(record: &MergeOperationRecordV1) -> ModelResult<Option<&str>> {
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| root_error("publication evidence has no accepted-workspace root base"))?;
    Ok(match &accepted.root.base {
        AcceptedRootBaseV1::BornAttached { commit, .. }
        | AcceptedRootBaseV1::BornDetached { commit } => Some(commit.as_str()),
        AcceptedRootBaseV1::UnbornAttached { .. } => None,
    })
}

fn publication_v1(
    record: &MergeOperationRecordV1,
) -> ModelResult<&crate::workspace_ops::merge::PublicationProgress> {
    record
        .publication
        .as_ref()
        .ok_or_else(|| root_error("publication progress is missing"))
}

fn candidate_v1(
    record: &MergeOperationRecordV1,
) -> ModelResult<&crate::workspace_ops::merge::PublicationCandidate> {
    publication_v1(record)?
        .candidate
        .as_ref()
        .ok_or_else(|| root_error("publication evidence has no immutable candidate"))
}

fn composition_commit_v1(record: &MergeOperationRecordV1) -> ModelResult<&str> {
    publication_v1(record)?
        .composition_commit
        .as_deref()
        .ok_or_else(|| root_error("publication evidence has no composition commit"))
}

fn marker_path_v1(record: &MergeOperationRecordV1) -> ModelResult<&String> {
    publication_v1(record)?
        .candidate_marker_path
        .as_ref()
        .ok_or_else(|| root_error("publication evidence has no marker path"))
}

fn classify(before: bool, after: bool) -> V1EvidenceRollbackObservation {
    match (before, after) {
        (true, false) => V1EvidenceRollbackObservation::Before,
        (false, true) => V1EvidenceRollbackObservation::After,
        _ => V1EvidenceRollbackObservation::Ambiguous,
    }
}

fn root_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into()).with_member("@root", ".")
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) fn classify_v1_evidence_shape_for_test(
    step: EvidenceRollbackStepV1,
    head_before: bool,
    head_after: bool,
    shape: &str,
) -> V1EvidenceRollbackObservation {
    let states = shape
        .bytes()
        .map(|value| if value == b'B' { B } else { C })
        .collect::<Vec<_>>();
    classify_v1_evidence_rollback(
        step,
        head_before,
        head_after,
        &EvidenceFileStates {
            boundary: states[0],
            lock: states[1],
            marker: states[2],
            boundary_noop: false,
            lock_noop: false,
        },
        states[3],
    )
}

/// A refused verification that failed on CONTENT, not on I/O.
///
/// The checked verification helpers answer one typed code for "these bytes are
/// not what the record says they are"; every other code is a real failure and
/// must propagate. Relocated with this module from `merge/abort/evidence.rs`.
fn semantic_mismatch(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::MergeRecoveryRequired | ErrorCode::MergeDrift | ErrorCode::MergeRecordUnreadable
    )
}
