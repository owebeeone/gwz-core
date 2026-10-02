#![forbid(clippy::disallowed_methods)]

use super::*;

/// **The four REVERSE doors** below all acquire with
/// [`IdentityGapEscape::ReverseMergeDoor`] (`GwzM5-8M5d-Charter.md` §3(b),
/// 2026-09-03).
///
/// They are the doors a selected-root, `--preserve` or published-evidence
/// abort takes, and the only production consumers of any of them are the
/// reverse path's: `merge/root/artifact_facts.rs` (reached solely from
/// `merge/v1_rollback/evidence.rs`), `merge/preserve/checked_bundle.rs` and
/// `git/gitbackend/preservation_root/files.rs`. On a volume without
/// persistent handles they still REFUSE — the charter forbids reverse-path
/// raw — but they refuse with an escape that is true here instead of the
/// substrate remedy's `gwz merge --abort`, which is this very door.
///
/// The forward create door below does NOT take this treatment: it does not
/// refuse at all on such a volume, it publishes raw.
pub(super) fn root_artifact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape_in(
        filesystem,
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{}'", relative.display()),
        IdentityGapEscape::ReverseMergeDoor,
    )
}

pub(super) fn preservation_bundle(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape_in(
        filesystem,
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "preservation bundle",
        IdentityGapEscape::ReverseMergeDoor,
    )
}

pub(super) fn preservation_workspace(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape_in(
        filesystem,
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
        IdentityGapEscape::ReverseMergeDoor,
    )
}

pub(super) fn observe_expected_durable(
    artifact: CheckedArtifact,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    matches_expected(artifact.observe_durable()?, expected)
}

#[rustfmt::skip]
pub(super) fn matches_expected(observed: CheckedArtifactFact, expected: Option<&[u8]>) -> ModelResult<bool> {
    Ok(match (observed, expected) {
        (CheckedArtifactFact::Missing, None) => true,
        (CheckedArtifactFact::Bytes(actual), Some(expected)) => actual == expected,
        _ => false,
    })
}

pub(super) fn filesystem_fact(observed: MergeArtifactFact) -> CheckedArtifactFact {
    match observed {
        MergeArtifactFact::Missing => CheckedArtifactFact::Missing,
        MergeArtifactFact::Bytes(bytes) => CheckedArtifactFact::Bytes(bytes),
        MergeArtifactFact::Invalid => CheckedArtifactFact::Invalid,
    }
}

pub(super) fn replace_expected(
    artifact: CheckedArtifact,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<()> {
    match goal {
        Some(goal) => artifact.replace_exact(&fact(expected), goal),
        None => artifact.remove_exact(&fact(expected)),
    }
}

pub(super) fn classify_expected(
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

pub(super) fn fact(bytes: Option<&[u8]>) -> CheckedArtifactFact {
    bytes.map_or(CheckedArtifactFact::Missing, |bytes| {
        CheckedArtifactFact::Bytes(bytes.to_vec())
    })
}

#[rustfmt::skip]
pub(super) fn map_transition(value: CheckedArtifactTransition) -> ModelResult<MergeArtifactTransition> {
    Ok(match value {
        CheckedArtifactTransition::Before => MergeArtifactTransition::Before,
        CheckedArtifactTransition::After => MergeArtifactTransition::After,
        CheckedArtifactTransition::Recoverable => MergeArtifactTransition::Recoverable,
        CheckedArtifactTransition::Ambiguous => MergeArtifactTransition::Ambiguous,
    })
}

pub(super) fn require_canonical_bundle_parent(artifact: &CheckedArtifact) -> ModelResult<()> {
    if artifact.parent_is_canonical()? {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation bundle parent hierarchy is missing or noncanonical",
        ))
    }
}
