use super::super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeTargetKind, OperationState,
    participant_semantics,
    publication::{
        RootEvidenceObservation, candidate_files_view, classify_candidate_publication_view,
        observe_root_evidence_view,
    },
    status::MergeStatusRecordView,
};
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::ModelResult;
use std::path::Path;

#[cfg(test)]
use crate::artifact::LOCK_PATH;
#[cfg(test)]
use crate::model::{ErrorCode, ModelError};
#[cfg(test)]
use crate::workspace::WORKSPACE_MANIFEST;
#[cfg(test)]
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, RootMetadataRollbackStepV1};

#[cfg(test)]
use super::artifact_facts;

pub(in crate::workspace_ops::merge) fn interrupted_evidence_rollback_is_exact<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<bool> {
    interrupted_evidence_rollback_is_exact_view(
        backend,
        root,
        MergeStatusRecordView::from_v0(record),
    )
}

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

pub(in crate::workspace_ops::merge) fn normalize_evidence_observation(
    snapshot: &mut MergeStatusSnapshot,
) -> ModelResult<()> {
    participant_semantics::status::apply_interrupted_root_rollback_override(snapshot)
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) use v1_rollback::{
    V1RootRollbackObservation, execute_v1_root_metadata_rollback,
    observe_v1_root_metadata_rollback, observe_v1_selected_root_baseline,
};

#[cfg(test)]
mod v1_rollback {
    use super::*;
    use std::fs::{self, File, Metadata};
    use std::io::Read;
    use std::path::Component;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(in crate::workspace_ops::merge) enum V1RootRollbackObservation {
        Before,
        After,
        Ambiguous,
    }

    pub(in crate::workspace_ops::merge) fn observe_v1_root_metadata_rollback<B: GitBackend>(
        backend: &B,
        root: &Path,
        record: &MergeOperationRecordV1,
        step: RootMetadataRollbackStepV1,
    ) -> ModelResult<V1RootRollbackObservation> {
        let (before_manifest, before_lock) = selected_root_result_artifacts(backend, root, record)?;
        let baseline_manifest = record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
            root_metadata_error("selected-root operation baseline has no manifest bytes")
        })?;
        let baseline_lock = record.baseline.lock_yaml.as_deref().ok_or_else(|| {
            root_metadata_error("selected-root operation baseline has no lock bytes")
        })?;
        let manifest = artifact_state(
            artifact_facts::observe(root, WORKSPACE_MANIFEST)?,
            &before_manifest,
            baseline_manifest,
        );
        let lock = artifact_state(
            artifact_facts::observe(root, LOCK_PATH)?,
            &before_lock,
            baseline_lock,
        );
        let manifest_noop = before_manifest == baseline_manifest;
        let lock_noop = before_lock == baseline_lock;
        let initial = (manifest == RootArtifactState::Before
            || (manifest_noop && manifest == RootArtifactState::After))
            && (lock == RootArtifactState::Before
                || (lock_noop && lock == RootArtifactState::After));
        let complete = manifest == RootArtifactState::After && lock == RootArtifactState::After;
        Ok(match step {
            RootMetadataRollbackStepV1::Manifest => classify_root(
                manifest == RootArtifactState::Before
                    && (lock == RootArtifactState::Before
                        || (lock_noop && lock == RootArtifactState::After)),
                manifest == RootArtifactState::After
                    && (lock == RootArtifactState::Before
                        || (lock_noop && lock == RootArtifactState::After)),
            ),
            RootMetadataRollbackStepV1::Lock => classify_root(
                manifest == RootArtifactState::After && lock == RootArtifactState::Before,
                complete,
            ),
            RootMetadataRollbackStepV1::Complete => {
                if complete {
                    V1RootRollbackObservation::After
                } else if initial {
                    V1RootRollbackObservation::Before
                } else {
                    V1RootRollbackObservation::Ambiguous
                }
            }
        })
    }

    /// Reacquire the complete selected-root rollback destination through the
    /// same canonical, no-follow artifact observer used by the step matrix.
    /// The caller binds this fresh fact to the exact checked record before it
    /// can authorize terminal rollback.
    pub(in crate::workspace_ops::merge) fn observe_v1_selected_root_baseline(
        root: &Path,
        record: &MergeOperationRecordV1,
    ) -> ModelResult<(String, String)> {
        let baseline_manifest = record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
            root_metadata_error("selected-root operation baseline has no manifest bytes")
        })?;
        let baseline_lock = record.baseline.lock_yaml.as_deref().ok_or_else(|| {
            root_metadata_error("selected-root operation baseline has no lock bytes")
        })?;
        if observe_final_artifact(root, WORKSPACE_MANIFEST)? != baseline_manifest.as_bytes()
            || observe_final_artifact(root, LOCK_PATH)? != baseline_lock.as_bytes()
        {
            return Err(root_metadata_error(
                "selected-root manifest and lock do not exactly match the operation baseline",
            ));
        }
        Ok((
            record.baseline.manifest_sha256.clone(),
            record.baseline.lock_sha256.clone(),
        ))
    }

    fn observe_final_artifact(root: &Path, relative: &str) -> ModelResult<Vec<u8>> {
        let relative_path = Path::new(relative);
        if relative_path.as_os_str().is_empty()
            || relative_path.is_absolute()
            || relative_path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(root_metadata_error(format!(
                "selected-root artifact path '{relative}' is not canonical"
            )));
        }

        let mut parents = Vec::new();
        let mut path = root.to_path_buf();
        parents.push((path.clone(), require_real_directory(&path)?));
        let components = relative_path.components().collect::<Vec<_>>();
        for component in components.iter().take(components.len() - 1) {
            path.push(component.as_os_str());
            parents.push((path.clone(), require_real_directory(&path)?));
        }
        path.push(components.last().unwrap().as_os_str());

        let before = fs::symlink_metadata(&path).map_err(|error| {
            root_metadata_error(format!(
                "failed to inspect selected-root artifact '{}': {error}",
                path.display()
            ))
        })?;
        if !before.file_type().is_file() || executable(&before) {
            return Err(noncanonical_artifact(&path));
        }
        let mut file = File::open(&path).map_err(|error| {
            root_metadata_error(format!(
                "failed to open selected-root artifact '{}': {error}",
                path.display()
            ))
        })?;
        let opened = file.metadata().map_err(|error| {
            root_metadata_error(format!(
                "failed to inspect opened selected-root artifact '{}': {error}",
                path.display()
            ))
        })?;
        if !same_file(&before, &opened) {
            return Err(noncanonical_artifact(&path));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|error| {
            root_metadata_error(format!(
                "failed to read selected-root artifact '{}': {error}",
                path.display()
            ))
        })?;
        let after = fs::symlink_metadata(&path).map_err(|_| noncanonical_artifact(&path))?;
        if !after.file_type().is_file()
            || executable(&after)
            || !same_file(&before, &after)
            || !same_file(&opened, &after)
            || opened.len() != bytes.len() as u64
        {
            return Err(noncanonical_artifact(&path));
        }
        for (parent, expected) in parents {
            let actual = require_real_directory(&parent)?;
            if !same_file(&expected, &actual) {
                return Err(noncanonical_artifact(&parent));
            }
        }
        Ok(bytes)
    }

    fn require_real_directory(path: &Path) -> ModelResult<Metadata> {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            root_metadata_error(format!(
                "failed to inspect selected-root parent '{}': {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_dir() {
            Ok(metadata)
        } else {
            Err(noncanonical_artifact(path))
        }
    }

    #[cfg(unix)]
    fn same_file(left: &Metadata, right: &Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;

        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.file_type() == right.file_type()
    }

    #[cfg(windows)]
    fn same_file(left: &Metadata, right: &Metadata) -> bool {
        use std::os::windows::fs::MetadataExt;

        left.volume_serial_number() == right.volume_serial_number()
            && left.file_index() == right.file_index()
            && left.file_type() == right.file_type()
    }

    #[cfg(unix)]
    fn executable(metadata: &Metadata) -> bool {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    fn executable(_metadata: &Metadata) -> bool {
        false
    }

    fn noncanonical_artifact(path: &Path) -> ModelError {
        root_metadata_error(format!(
            "selected-root artifact '{}' is not a stable canonical regular file",
            path.display()
        ))
    }

    pub(in crate::workspace_ops::merge) fn execute_v1_root_metadata_rollback<B: GitBackend>(
        backend: &B,
        root: &Path,
        record: &MergeOperationRecordV1,
        step: RootMetadataRollbackStepV1,
    ) -> ModelResult<()> {
        if observe_v1_root_metadata_rollback(backend, root, record, step)?
            != V1RootRollbackObservation::Before
        {
            return Err(root_metadata_error(
                "selected-root metadata rollback is not at its exact before state",
            ));
        }
        match step {
            RootMetadataRollbackStepV1::Manifest | RootMetadataRollbackStepV1::Lock => {
                let before = selected_root_result_artifacts(backend, root, record)?;
                let (relative, expected, target) = if step == RootMetadataRollbackStepV1::Manifest {
                    (
                        WORKSPACE_MANIFEST,
                        before.0.as_bytes(),
                        record.baseline.manifest_yaml.as_deref().unwrap().as_bytes(),
                    )
                } else {
                    (
                        LOCK_PATH,
                        before.1.as_bytes(),
                        record.baseline.lock_yaml.as_deref().unwrap().as_bytes(),
                    )
                };
                artifact_facts::write_checked(root, relative, expected, target)
            }
            RootMetadataRollbackStepV1::Complete => Err(root_metadata_error(
                "complete selected-root rollback has no physical mutation",
            )),
        }
    }

    fn selected_root_result_artifacts<B: GitBackend>(
        backend: &B,
        root: &Path,
        record: &MergeOperationRecordV1,
    ) -> ModelResult<(String, String)> {
        let row = record
            .participants
            .get("@root")
            .ok_or_else(|| root_metadata_error("selected-root participant is missing"))?;
        let result = if let Some(commit) = row.resulting_commit.as_deref() {
            let read = |relative, name| {
                backend
                    .read_file_at_commit(root, commit, relative)?
                    .ok_or_else(|| {
                        root_metadata_error(format!("selected-root result has no {name}"))
                    })
                    .and_then(|bytes| utf8_artifact(bytes, name))
            };
            (
                read(WORKSPACE_MANIFEST, "manifest")?,
                read(LOCK_PATH, "lock")?,
            )
        } else {
            (
                record.baseline.manifest_yaml.clone().ok_or_else(|| {
                    root_metadata_error("selected-root baseline has no manifest bytes")
                })?,
                record.baseline.lock_yaml.clone().ok_or_else(|| {
                    root_metadata_error("selected-root baseline has no lock bytes")
                })?,
            )
        };
        if let Some(accepted) = record.accepted_workspace.as_ref()
            && (accepted.metadata_base.manifest_exact_yaml != result.0
                || accepted.metadata_base.lock_exact_yaml != result.1)
        {
            return Err(root_metadata_error(
                "accepted root metadata does not match the selected-root result",
            ));
        }
        Ok(result)
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RootArtifactState {
        Before,
        After,
        Other,
    }

    fn artifact_state(
        fact: artifact_facts::RegularFileFact,
        before: &str,
        after: &str,
    ) -> RootArtifactState {
        match fact {
            artifact_facts::RegularFileFact::Bytes(bytes)
                if bytes == before.as_bytes() && before != after =>
            {
                RootArtifactState::Before
            }
            artifact_facts::RegularFileFact::Bytes(bytes) if bytes == after.as_bytes() => {
                RootArtifactState::After
            }
            artifact_facts::RegularFileFact::Missing
            | artifact_facts::RegularFileFact::Bytes(_)
            | artifact_facts::RegularFileFact::Invalid => RootArtifactState::Other,
        }
    }

    fn classify_root(before: bool, after: bool) -> V1RootRollbackObservation {
        match (before, after) {
            (true, false) => V1RootRollbackObservation::Before,
            (false, true) => V1RootRollbackObservation::After,
            _ => V1RootRollbackObservation::Ambiguous,
        }
    }

    fn utf8_artifact(bytes: Vec<u8>, name: &str) -> ModelResult<String> {
        String::from_utf8(bytes)
            .map_err(|_| root_metadata_error(format!("selected-root result {name} is not UTF-8")))
    }

    fn root_metadata_error(detail: impl Into<String>) -> ModelError {
        ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into()).with_member("@root", ".")
    }
}
