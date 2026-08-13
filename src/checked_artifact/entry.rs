//! Complete purpose-specific operations at the production checked boundary.
//!
//! The general checked capability never leaves this module. Callers receive
//! only facts or transition classifications for their declared merge purpose.

#![forbid(clippy::disallowed_methods)]

use std::path::Path;

use super::{
    CheckedArtifact, CheckedArtifactFact, CheckedArtifactPolicy, CheckedArtifactTransition,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MergeArtifactFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MergeArtifactTransition {
    Before,
    After,
    Recoverable,
    Ambiguous,
}

pub(crate) fn observe_merge_root_artifact(
    root: &Path,
    relative: &Path,
) -> ModelResult<MergeArtifactFact> {
    map_fact(root_artifact(root, relative)?.observe()?)
}

pub(crate) fn replace_merge_root_artifact(
    root: &Path,
    relative: &Path,
    expected: &[u8],
    goal: &[u8],
) -> ModelResult<()> {
    root_artifact(root, relative)?
        .replace_exact(&CheckedArtifactFact::Bytes(expected.to_vec()), goal)
}

pub(crate) fn classify_replace_merge_root_artifact(
    root: &Path,
    relative: &Path,
    expected: &[u8],
    goal: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    map_transition(
        root_artifact(root, relative)?
            .classify_replace(&CheckedArtifactFact::Bytes(expected.to_vec()), goal)?,
    )
}

pub(crate) fn remove_merge_root_artifact(
    root: &Path,
    relative: &Path,
    expected: &[u8],
) -> ModelResult<()> {
    root_artifact(root, relative)?.remove_exact(&CheckedArtifactFact::Bytes(expected.to_vec()))
}

pub(crate) fn classify_remove_merge_root_artifact(
    root: &Path,
    relative: &Path,
    expected: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    map_transition(
        root_artifact(root, relative)?
            .classify_remove(&CheckedArtifactFact::Bytes(expected.to_vec()))?,
    )
}

pub(crate) fn observe_merge_preservation_workspace(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    observe_expected(preservation_workspace(root, relative)?, expected)
}

pub(crate) fn observe_merge_preservation_git_directory(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    observe_expected(preservation_git_directory(root, relative)?, expected)
}

pub(crate) fn replace_merge_preservation_workspace(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<()> {
    replace_expected(preservation_workspace(root, relative)?, expected, goal)
}

pub(crate) fn classify_merge_preservation_workspace(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<MergeArtifactTransition> {
    classify_expected(preservation_workspace(root, relative)?, expected, goal)
}

pub(crate) fn observe_merge_preservation_bundle(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    let artifact = preservation_bundle(root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    observe_expected_durable(artifact, expected)
}

pub(crate) fn classify_merge_preservation_bundle(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    let artifact = preservation_bundle(root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    map_transition(artifact.classify_replace(&fact(expected), goal)?)
}

pub(crate) fn replace_merge_preservation_bundle(
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: &[u8],
) -> ModelResult<()> {
    let artifact = preservation_bundle(root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    artifact.replace_exact(&fact(expected), goal)
}

pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {
    CheckedArtifact::prepare_parent(
        root,
        Path::new(crate::stash::STASH_BUNDLE_DIR),
        ErrorCode::MergeRecoveryRequired,
        "preservation bundle parent",
    )
}

fn root_artifact(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{}'", relative.display()),
    )
}

fn preservation_bundle(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "preservation bundle",
    )
}

fn preservation_workspace(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
    )
}

fn preservation_git_directory(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire(
        CheckedArtifactPolicy::git_directory(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
    )
}

fn observe_expected(artifact: CheckedArtifact, expected: Option<&[u8]>) -> ModelResult<bool> {
    matches_expected(artifact.observe()?, expected)
}

fn observe_expected_durable(
    artifact: CheckedArtifact,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    matches_expected(artifact.observe_durable()?, expected)
}

fn matches_expected(observed: CheckedArtifactFact, expected: Option<&[u8]>) -> ModelResult<bool> {
    Ok(match (observed, expected) {
        (CheckedArtifactFact::Missing, None) => true,
        (CheckedArtifactFact::Bytes(actual), Some(expected)) => actual == expected,
        _ => false,
    })
}

fn replace_expected(
    artifact: CheckedArtifact,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<()> {
    match goal {
        Some(goal) => artifact.replace_exact(&fact(expected), goal),
        None => artifact.remove_exact(&fact(expected)),
    }
}

fn classify_expected(
    artifact: CheckedArtifact,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<MergeArtifactTransition> {
    match goal {
        Some(goal) => map_transition(artifact.classify_replace(&fact(expected), goal)?),
        None if expected.is_some() => map_transition(artifact.classify_remove(&fact(expected))?),
        None => Ok(
            if artifact.observe_durable()? == CheckedArtifactFact::Missing {
                MergeArtifactTransition::After
            } else {
                MergeArtifactTransition::Ambiguous
            },
        ),
    }
}

fn fact(bytes: Option<&[u8]>) -> CheckedArtifactFact {
    bytes.map_or(CheckedArtifactFact::Missing, |bytes| {
        CheckedArtifactFact::Bytes(bytes.to_vec())
    })
}

fn map_fact(value: CheckedArtifactFact) -> ModelResult<MergeArtifactFact> {
    Ok(match value {
        CheckedArtifactFact::Missing => MergeArtifactFact::Missing,
        CheckedArtifactFact::Bytes(bytes) => MergeArtifactFact::Bytes(bytes),
        CheckedArtifactFact::Invalid => MergeArtifactFact::Invalid,
    })
}

fn map_transition(value: CheckedArtifactTransition) -> ModelResult<MergeArtifactTransition> {
    Ok(match value {
        CheckedArtifactTransition::Before => MergeArtifactTransition::Before,
        CheckedArtifactTransition::After => MergeArtifactTransition::After,
        CheckedArtifactTransition::Recoverable => MergeArtifactTransition::Recoverable,
        CheckedArtifactTransition::Ambiguous => MergeArtifactTransition::Ambiguous,
    })
}

fn require_canonical_bundle_parent(artifact: &CheckedArtifact) -> ModelResult<()> {
    if artifact.parent_is_canonical()? {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation bundle parent hierarchy is missing or noncanonical",
        ))
    }
}
