use super::{
    super::{
        MergeOperationRecord, MergeStore, MergeTargetKind, OperationState,
        publication::{
            CandidatePublicationPrefix, RootEvidenceObservation, candidate_files,
            classify_candidate_publication, composition_message, publication_prefix_allowed,
        },
    },
    runtime::AbortRuntime,
};
use crate::artifact;
use crate::git::GitCandidateFile;
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;
use std::{fs, path::Path};

use crate::workspace_ops::merge::model::v1::{
    AcceptedRootBaseV1, EvidenceRollbackStepV1, MergeOperationRecordV1,
};

use super::super::root::artifact_facts;

mod v1_rollback {
    use super::*;

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
        publication.candidate.as_ref().ok_or_else(|| {
            root_error("publication-evidence rollback has no immutable candidate")
        })?;
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
            == super::artifact_facts::RegularFileFact::Bytes(
                candidate.baseline_boundary_text.as_bytes().to_vec(),
            )
            && marker == super::artifact_facts::RegularFileFact::Missing)
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
            EvidenceRollbackStepV1::Complete => {
                classify(false, head_after && files.index_after(index))
            }
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
            artifact_facts::RegularFileFact::Bytes(bytes) if bytes == baseline => {
                FileState::Baseline
            }
            artifact_facts::RegularFileFact::Bytes(bytes) if bytes == candidate => {
                FileState::Candidate
            }
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
        let accepted = record.accepted_workspace.as_ref().ok_or_else(|| {
            root_error("publication evidence has no accepted-workspace root base")
        })?;
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
}

fn semantic_mismatch(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::DirtyMember
            | ErrorCode::MergeDrift
            | ErrorCode::MergeRecoveryRequired
            | ErrorCode::RecoveryEvidenceMismatch
    )
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1_rollback::classify_v1_evidence_shape_for_test;
pub(in crate::workspace_ops::merge) use v1_rollback::{
    V1EvidenceRollbackObservation, execute_v1_evidence_rollback, observe_v1_evidence_rollback,
    preflight_v1_evidence, v1_evidence_residue_after_selected_root_is_exact,
};

pub(super) struct EvidenceRollback {
    branch: String,
    composition_commit: String,
    baseline_commit: Option<String>,
    marker_id: String,
    baseline_lock_yaml: String,
    baseline_boundary_text: String,
    candidate_files: Vec<GitCandidateFile>,
    composition_message: String,
    pub(super) root_participant_evidence_present: bool,
}

pub(super) fn preflight_evidence<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<EvidenceRollback>> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(None);
    };
    let Some(candidate) = publication.candidate.as_ref() else {
        if publication.candidate_lock_sha256.is_some()
            || publication.candidate_marker_path.is_some()
            || publication.root_merge_commit.is_some()
            || publication.composition_commit.is_some()
            || publication.composition_tree.is_some()
            || !publication.candidate_hashes.is_empty()
            || publication.evidence_rolled_back
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "merge publication has evidence fields but no durable candidate",
            ));
        }
        return Ok(None);
    };
    let root_participant = record.participants.get("@root").filter(|participant| {
        participant.target_kind == MergeTargetKind::Root && participant.path == "."
    });
    if publication.evidence_rolled_back
        && let Some(participant) = root_participant
    {
        let head = runtime.head(root)?;
        if !head.is_detached
            && head.branch.as_deref() == Some(participant.target_branch.as_str())
            && head.commit.as_deref() == Some(participant.before_commit.as_str())
        {
            return Ok(None);
        }
    }
    let prefix = classify_candidate_publication(root, record)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts changed after evidence creation",
        )
        .with_member("@root", ".")
    })?;
    if !matches!(
        record.state,
        OperationState::Preserving | OperationState::RollingBack
    ) && !publication_prefix_allowed(record, prefix)?
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts do not match the recorded publication step",
        )
        .with_member("@root", "."));
    }
    let observation = runtime.observe_root_evidence(root, record)?;
    let (composition_commit, root_participant_evidence_present) = match observation {
        Some(RootEvidenceObservation::Composition(result)) => {
            if root_participant.is_some() && !runtime.root_finalization_is_exact(root, record)? {
                return Err(ModelError::new(
                    ErrorCode::MergeDrift,
                    "workspace root contains post-merge work that must be preserved or removed before abort",
                )
                .with_member("@root", "."));
            }
            (result.commit, root_participant.is_some())
        }
        Some(RootEvidenceObservation::Baseline)
            if publication.composition_commit.is_none()
                && prefix == CandidatePublicationPrefix::Baseline =>
        {
            return Ok(None);
        }
        Some(RootEvidenceObservation::Baseline) => {
            let interrupted_root_rollback = root_participant.is_some()
                && record.state == OperationState::RollingBack
                && !publication.evidence_rolled_back
                && runtime.root_evidence_rollback_is_exact(root, record)?;
            (
                publication.composition_commit.clone().ok_or_else(|| {
                    ModelError::new(
                        ErrorCode::MergeDrift,
                        "published candidate has no recorded root evidence commit",
                    )
                    .with_member("@root", ".")
                })?,
                interrupted_root_rollback,
            )
        }
        None => {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "workspace root moved after merge evidence creation",
            )
            .with_member("@root", "."));
        }
    };
    Ok(Some(EvidenceRollback {
        branch: candidate.root_branch.clone(),
        composition_commit,
        baseline_commit: super::super::root::evidence_parent(record)?.map(str::to_owned),
        marker_id: candidate.marker_id.clone(),
        baseline_lock_yaml: candidate.baseline_lock_yaml.clone(),
        baseline_boundary_text: candidate.baseline_boundary_text.clone(),
        candidate_files: candidate_files(record)?,
        composition_message: composition_message(record),
        root_participant_evidence_present,
    }))
}

/// CAPABILITY-FREE EXCEPTION, §10 rows `:277`/`:278`/`:279` (2026-09-02,
/// `GwzM5-8R2E-CapabilityFreeAmendment.md` §3): THIS ARM only. Abort is on E0.2
/// §5.2's list, so the v0 boundary, lock and marker writers below stay raw
/// permanently; the converted v1 arms above are untouched, and P-2 scans by region.
pub(super) fn rollback_evidence<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    evidence: &EvidenceRollback,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    if head.commit.as_deref() == Some(evidence.composition_commit.as_str()) {
        runtime.rollback_evidence_commit(
            root,
            &evidence.branch,
            &evidence.composition_commit,
            evidence.baseline_commit.as_deref(),
            &evidence.candidate_files,
            &evidence.composition_message,
        )?;
    }
    super::super::super::publish_workspace_exclude_candidate(
        root,
        &evidence.baseline_boundary_text,
    )?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Boundary)?;
    artifact::write_atomic(
        &root.join(artifact::LOCK_PATH),
        &evidence.baseline_lock_yaml,
    )?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Lock)?;
    let marker_path = artifact::marker_path(root, &evidence.marker_id);
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ModelError::new(
                ErrorCode::IoError,
                format!("failed to remove merge marker during abort: {error}"),
            ));
        }
    }
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Marker)?;
    let marker_relative = format!("{}/{}.yaml", artifact::MARKER_DIR, evidence.marker_id);
    runtime.stage_paths(root, &[artifact::LOCK_PATH, &marker_relative])?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Staging)?;
    if let Some(publication) = record.publication.as_mut() {
        publication.evidence_rolled_back = true;
    }
    super::super::persist_merge_record(store, root, record, emitter)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceRollbackMutation {
    Boundary,
    Lock,
    Marker,
    Staging,
}

#[cfg(test)]
thread_local! {
    static EVIDENCE_ROLLBACK_FAILURE:
        std::cell::Cell<Option<EvidenceRollbackMutation>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_evidence_rollback_after(mutation: EvidenceRollbackMutation) {
    EVIDENCE_ROLLBACK_FAILURE.with(|failure| {
        assert!(
            failure.replace(Some(mutation)).is_none(),
            "an evidence rollback failure is already installed"
        );
    });
}

#[cfg(test)]
fn maybe_fail_evidence_rollback_after(mutation: EvidenceRollbackMutation) -> ModelResult<()> {
    EVIDENCE_ROLLBACK_FAILURE.with(|failure| {
        if failure.get() == Some(mutation) {
            failure.set(None);
            Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("injected failure after evidence {mutation:?} restoration"),
            ))
        } else {
            Ok(())
        }
    })
}

pub(super) fn verify_evidence_baseline<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    evidence: &EvidenceRollback,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    let marker_absent = !artifact::marker_path(root, &evidence.marker_id).exists();
    let lock_matches = fs::read(root.join(artifact::LOCK_PATH)).ok().as_deref()
        == Some(evidence.baseline_lock_yaml.as_bytes());
    let boundary_matches = fs::read(super::super::super::workspace_exclude_path(root))
        .ok()
        .as_deref()
        == Some(evidence.baseline_boundary_text.as_bytes());
    if head.is_detached
        || head.branch.as_deref() != Some(evidence.branch.as_str())
        || head.commit != evidence.baseline_commit
        || !marker_absent
        || !lock_matches
        || !boundary_matches
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root changed during merge evidence rollback",
        )
        .with_member("@root", "."));
    }
    Ok(())
}
