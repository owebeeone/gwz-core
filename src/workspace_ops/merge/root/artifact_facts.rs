use std::path::Path;

use crate::checked_artifact::{CheckedArtifact, CheckedArtifactFact};
use crate::model::{ErrorCode, ModelResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RegularFileFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
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

pub(in crate::workspace_ops::merge) fn remove_exact(
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<()> {
    let artifact = acquire(root, relative)?;
    artifact.remove_exact(&CheckedArtifactFact::Bytes(expected.to_vec()))
}

fn acquire(root: &Path, relative: &str) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        root,
        Path::new(relative),
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{relative}'"),
    )
}
