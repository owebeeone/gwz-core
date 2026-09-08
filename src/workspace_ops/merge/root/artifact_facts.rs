#![forbid(clippy::disallowed_methods)]

use crate::filesystem::FileSystem;
use std::path::Path;

use crate::checked_artifact::entry::{MergeArtifactFact, MergeArtifactTransition};
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
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &str,
) -> ModelResult<RegularFileFact> {
    Ok(
        match crate::checked_artifact::entry::observe_merge_root_artifact(
            filesystem,
            root,
            Path::new(relative),
        )? {
            MergeArtifactFact::Missing => RegularFileFact::Missing,
            MergeArtifactFact::Bytes(bytes) => RegularFileFact::Bytes(bytes),
            MergeArtifactFact::Invalid => RegularFileFact::Invalid,
        },
    )
}

pub(in crate::workspace_ops::merge) fn write_checked(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &str,
    expected: &[u8],
    bytes: &[u8],
) -> ModelResult<()> {
    crate::checked_artifact::entry::replace_merge_root_artifact(
        filesystem,
        root,
        Path::new(relative),
        expected,
        bytes,
    )
}

pub(in crate::workspace_ops::merge) fn classify_write(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &str,
    expected: &[u8],
    bytes: &[u8],
) -> ModelResult<RegularFileTransition> {
    map_transition(
        crate::checked_artifact::entry::classify_replace_merge_root_artifact(
            filesystem,
            root,
            Path::new(relative),
            expected,
            bytes,
        )?,
    )
}

pub(in crate::workspace_ops::merge) fn remove_exact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<()> {
    crate::checked_artifact::entry::remove_merge_root_artifact(
        filesystem,
        root,
        Path::new(relative),
        expected,
    )
}

pub(in crate::workspace_ops::merge) fn classify_remove(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &str,
    expected: &[u8],
) -> ModelResult<RegularFileTransition> {
    map_transition(
        crate::checked_artifact::entry::classify_remove_merge_root_artifact(
            filesystem,
            root,
            Path::new(relative),
            expected,
        )?,
    )
}

fn map_transition(value: MergeArtifactTransition) -> ModelResult<RegularFileTransition> {
    Ok(match value {
        MergeArtifactTransition::Before => RegularFileTransition::Before,
        MergeArtifactTransition::After => RegularFileTransition::After,
        MergeArtifactTransition::Recoverable => RegularFileTransition::Recoverable,
        MergeArtifactTransition::Ambiguous => RegularFileTransition::Ambiguous,
    })
}
