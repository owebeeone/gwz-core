//! Complete purpose-specific operations at the production checked boundary.
//!
//! The general checked capability never leaves this module. Callers receive
//! only facts or transition classifications for their declared merge purpose.

#![forbid(clippy::disallowed_methods)]

use std::path::Path;

use super::bootstrap::CatalogMutationLeaseV1;
use super::capability::CheckedFsError;
use super::catalog::recover_or_create;
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

/// R2-E Phase E4 Step E4.1 (O2): the first production catalog activation.
///
/// `recover_or_create` is `pub(in crate::checked_artifact)`, so its caller must
/// live inside this module tree, and this module is the crate's declared
/// production checked boundary — so the door is here and the operation calls
/// it. Its callers are the arms that mutate a record toward v1 semantics —
/// `v1_lifecycle`'s `V1MutationLease::acquire_activated` (start's creation
/// lease and the forward service loop) and `dispatch.rs`'s A1 adapter, which
/// proves viability here before its durable v0->v1 upgrade.
///
/// **Where the capability is required, and where it is not** (E0.2 §5.2 with
/// E0.2b §6.4's fifth ground, corrected by the E4.1 review's [P1-1]/[P2-1]):
/// `WorkspaceMutatorLock::try_acquire` probes no durable identity, here or
/// after. `gwz repo create`, `init-from-sources`, GC, the mutation guard and
/// `gwz merge --abort` never reach this door — so a refusal always has an exit.
/// An ordinary or `--ff-only` merge reaches it only through the A1 adapter's
/// viability window, where a refusal is never surfaced: the v0 lifecycle stays
/// in command. What refuses, typed, is a `--no-ff` start and the resume of a
/// record already at v1.
///
/// **The retained catalog is dropped.** Activation proves the catalog and
/// leaves it durable; the lease model is that each consumer re-acquires
/// (`coordinator/execution.rs`'s admission session says the same). E4.2-E4.6
/// convert the consumers that will read it.
///
/// **Two scope clauses this door's consumers must not lean on** (E7.2's
/// [R2-P3-1] and its terminal sibling, written at the plan's E4 gate note):
/// a settled barrier ordinal does not imply its target parent's dirents were
/// ever ordered, and a converged-by-observation restart does not imply key #8's
/// retired-root flush or key #9's catalog-root barrier ran on that drive.
/// Converged does not imply flushed; settled does not imply barriered. A
/// consumer that needs either must barrier or flush for itself.
pub(crate) fn activate_workspace_catalog(lease: CatalogMutationLeaseV1<'_>) -> ModelResult<()> {
    recover_or_create(lease)
        .map(|_retained| ())
        .map_err(|cause| {
            let label = "merge artifact catalog";
            match cause {
                CheckedFsError::Unsupported { capability, detail } => ModelError::new(
                    ErrorCode::UnsupportedOperation,
                    match capability.remedy() {
                        // Precondition 1: the one gap a user can act on arrives as
                        // the sentence that says what to do, with the substrate's
                        // own words kept for diagnosis.
                        Some(remedy) => format!("checked {label}: {remedy} (detail: {detail})"),
                        None => format!("checked {label} is unsupported: {detail}"),
                    },
                ),
                CheckedFsError::Io { operation, source } => ModelError::new(
                    ErrorCode::IoError,
                    format!("checked {label} {operation}: {source}"),
                ),
                CheckedFsError::Ambiguous { fact, detail } => ModelError::new(
                    ErrorCode::IoError,
                    format!("checked {label} rejected {fact}: {detail}"),
                ),
            }
        })
}
