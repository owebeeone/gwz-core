use std::path::Path;

use crate::checked_artifact::{CheckedArtifact, CheckedArtifactFact, CheckedArtifactTransition};
use crate::model::ModelResult;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RegularFileFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RegularFileTransition {
    Before,
    After,
    Recoverable,
    Ambiguous,
}

pub(in crate::workspace_ops::merge) fn observe(
    root: &Path,
    relative: &str,
) -> ModelResult<RegularFileFact> {
    Ok(match acquire(root, relative)?.observe()? {
        CheckedArtifactFact::Missing => RegularFileFact::Missing,
        CheckedArtifactFact::Bytes(bytes) => RegularFileFact::Bytes(bytes),
        CheckedArtifactFact::Invalid => RegularFileFact::Invalid,
    })
}

pub(in crate::workspace_ops::merge) fn write_checked(
    root: &Path,
    relative: &str,
    expected: &[u8],
    bytes: &[u8],
) -> ModelResult<()> {
    let artifact = acquire(root, relative)?;
    artifact.replace_exact(&CheckedArtifactFact::Bytes(expected.to_vec()), bytes)
}

pub(in crate::workspace_ops::merge) fn classify_write(
    root: &Path,
    relative: &str,
    expected: &[u8],
    bytes: &[u8],
) -> ModelResult<RegularFileTransition> {
    map_transition(
        acquire(root, relative)?
            .classify_replace(&CheckedArtifactFact::Bytes(expected.to_vec()), bytes)?,
    )
}

pub(in crate::workspace_ops::merge) fn remove_exact(
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<()> {
    let artifact = acquire(root, relative)?;
    artifact.remove_exact(&CheckedArtifactFact::Bytes(expected.to_vec()))
}

pub(in crate::workspace_ops::merge) fn classify_remove(
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<RegularFileTransition> {
    map_transition(
        acquire(root, relative)?.classify_remove(&CheckedArtifactFact::Bytes(expected.to_vec()))?,
    )
}

fn map_transition(value: CheckedArtifactTransition) -> ModelResult<RegularFileTransition> {
    Ok(match value {
        CheckedArtifactTransition::Before => RegularFileTransition::Before,
        CheckedArtifactTransition::After => RegularFileTransition::After,
        CheckedArtifactTransition::Recoverable => RegularFileTransition::Recoverable,
        CheckedArtifactTransition::Ambiguous => RegularFileTransition::Ambiguous,
    })
}

fn acquire(root: &Path, relative: &str) -> ModelResult<CheckedArtifact> {
    crate::checked_artifact::entry::acquire_merge_root_artifact(root, Path::new(relative))
}
