//! The v1 reverse path's root-metadata rollback.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** Relocated out of
//! `merge/root/abort.rs`, whose remaining surface belonged to the v0 abort
//! engine.

use super::artifact_facts;
use crate::artifact::LOCK_PATH;
use crate::filesystem::{FileSystem, FsKind, make_filesystem};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, RootMetadataRollbackStepV1};
use std::path::Path;

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
    let baseline_lock =
        record.baseline.lock_yaml.as_deref().ok_or_else(|| {
            root_metadata_error("selected-root operation baseline has no lock bytes")
        })?;
    let manifest = if step == RootMetadataRollbackStepV1::Manifest {
        transition_state(artifact_facts::classify_write(
            root,
            WORKSPACE_MANIFEST,
            before_manifest.as_bytes(),
            baseline_manifest.as_bytes(),
        )?)
    } else {
        artifact_state(
            artifact_facts::observe(root, WORKSPACE_MANIFEST)?,
            &before_manifest,
            baseline_manifest,
        )
    };
    let lock = if step == RootMetadataRollbackStepV1::Lock {
        transition_state(artifact_facts::classify_write(
            root,
            LOCK_PATH,
            before_lock.as_bytes(),
            baseline_lock.as_bytes(),
        )?)
    } else {
        artifact_state(
            artifact_facts::observe(root, LOCK_PATH)?,
            &before_lock,
            baseline_lock,
        )
    };
    let manifest_noop = before_manifest == baseline_manifest;
    let lock_noop = before_lock == baseline_lock;
    let initial = (manifest == RootArtifactState::Before
        || (manifest_noop && manifest == RootArtifactState::After))
        && (lock == RootArtifactState::Before || (lock_noop && lock == RootArtifactState::After));
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
    let baseline_lock =
        record.baseline.lock_yaml.as_deref().ok_or_else(|| {
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

    observe_final_artifact_through_filesystem(root, relative_path)
}

fn observe_final_artifact_through_filesystem(root: &Path, relative: &Path) -> ModelResult<Vec<u8>> {
    let filesystem = make_filesystem();
    let mut directory = filesystem.open_directory(root).map_err(|error| {
        root_metadata_error(format!(
            "failed to inspect selected-root parent '{}': {error}",
            root.display()
        ))
    })?;
    let mut parents = vec![(
        root.to_path_buf(),
        filesystem
            .directory_identity(&directory)
            .map_err(|error| root_metadata_error(error.to_string()))?,
    )];
    let components = relative.components().collect::<Vec<_>>();
    let mut path = root.to_path_buf();
    for component in components.iter().take(components.len() - 1) {
        path.push(component.as_os_str());
        directory = filesystem
            .open_directory_at(&directory, component.as_os_str())
            .map_err(|error| {
                root_metadata_error(format!(
                    "failed to inspect selected-root parent '{}': {error}",
                    path.display()
                ))
            })?;
        parents.push((
            path.clone(),
            filesystem
                .directory_identity(&directory)
                .map_err(|error| root_metadata_error(error.to_string()))?,
        ));
    }
    let leaf = components.last().unwrap().as_os_str();
    path.push(leaf);
    let metadata = filesystem
        .metadata(&path)
        .map_err(|error| root_metadata_error(error.to_string()))?;
    if metadata.kind != FsKind::File || metadata.executable {
        return Err(noncanonical_artifact(&path));
    }
    let file = filesystem
        .open_file_at(&directory, leaf)
        .map_err(|error| root_metadata_error(error.to_string()))?;
    let identity = filesystem
        .file_identity(&file)
        .map_err(|error| root_metadata_error(error.to_string()))?;
    if !filesystem
        .file_entry_matches(&directory, leaf, &file)
        .map_err(|_| noncanonical_artifact(&path))?
    {
        return Err(noncanonical_artifact(&path));
    }
    let bytes = filesystem
        .read_all(&file)
        .map_err(|error| root_metadata_error(error.to_string()))?;
    let reopened = filesystem
        .open_file_at(&directory, leaf)
        .map_err(|_| noncanonical_artifact(&path))?;
    let after = filesystem
        .metadata(&path)
        .map_err(|_| noncanonical_artifact(&path))?;
    if after.kind != FsKind::File
        || after.executable
        || !filesystem
            .file_entry_matches(&directory, leaf, &file)
            .map_err(|_| noncanonical_artifact(&path))?
        || filesystem
            .file_identity(&reopened)
            .map_err(|_| noncanonical_artifact(&path))?
            != identity
        || !filesystem
            .file_entry_matches(&directory, leaf, &reopened)
            .map_err(|_| noncanonical_artifact(&path))?
        || filesystem
            .read_all(&reopened)
            .map_err(|_| noncanonical_artifact(&path))?
            != bytes
    {
        return Err(noncanonical_artifact(&path));
    }
    for (parent, expected) in parents {
        let reopened = filesystem
            .open_directory(&parent)
            .map_err(|_| noncanonical_artifact(&parent))?;
        if filesystem
            .directory_identity(&reopened)
            .map_err(|_| noncanonical_artifact(&parent))?
            != expected
        {
            return Err(noncanonical_artifact(&parent));
        }
    }
    Ok(bytes)
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
                    record
                        .baseline
                        .manifest_yaml
                        .as_deref()
                        .ok_or_else(|| {
                            root_metadata_error(
                                "selected-root operation baseline has no manifest bytes",
                            )
                        })?
                        .as_bytes(),
                )
            } else {
                (
                    LOCK_PATH,
                    before.1.as_bytes(),
                    record
                        .baseline
                        .lock_yaml
                        .as_deref()
                        .ok_or_else(|| {
                            root_metadata_error(
                                "selected-root operation baseline has no lock bytes",
                            )
                        })?
                        .as_bytes(),
                )
            };
            artifact_facts::write_checked(root, relative, expected, target)
        }
        RootMetadataRollbackStepV1::Complete => Err(root_metadata_error(
            "complete selected-root rollback has no physical mutation",
        )),
    }
}

pub(in crate::workspace_ops::merge) fn selected_root_result_artifacts<B: GitBackend>(
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
                .ok_or_else(|| root_metadata_error(format!("selected-root result has no {name}")))
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
            record
                .baseline
                .lock_yaml
                .clone()
                .ok_or_else(|| root_metadata_error("selected-root baseline has no lock bytes"))?,
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

fn transition_state(value: artifact_facts::RegularFileTransition) -> RootArtifactState {
    match value {
        artifact_facts::RegularFileTransition::Before
        | artifact_facts::RegularFileTransition::Recoverable => RootArtifactState::Before,
        artifact_facts::RegularFileTransition::After => RootArtifactState::After,
        artifact_facts::RegularFileTransition::Ambiguous => RootArtifactState::Other,
    }
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
