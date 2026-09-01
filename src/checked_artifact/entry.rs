//! Complete purpose-specific operations at the production checked boundary.
//!
//! The general checked capability never leaves this module. Callers receive
//! only facts or transition classifications for their declared merge purpose.

#![forbid(clippy::disallowed_methods)]

use std::path::Path;

use super::bootstrap::CatalogMutationLeaseV1;
use super::capability::CheckedFsError;
use super::catalog::recover_or_create;
use super::coordinator::execution::{
    admit_merge_start_managed_parents, execute_merge_start_managed_parents,
};
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
        .map_err(|cause| render_catalog_refusal(CATALOG_LABEL, cause))
}

/// R2-E Step E4.2 — the first merge record's parent half (ConsumerCheckpoint
/// §10 row `:273`, "`MergeStore` and `PreservationBundles` when missing";
/// frozen clause one, "both parents durable before record").
///
/// **The row's whole creation authority.** Both prefixes are installed by the
/// managed-parent provider through an admitted `ParentOnly` action over a sealed
/// purpose set, never by a raw `create_dir_all` on the writer's side — which is
/// why `store/rewrite.rs` now REFUSES a missing parent instead of making one.
/// Two leases because the Phase-1 owner CONSUMES the retained catalog: admission
/// ends when it returns, and execution recovers again, after admission created
/// the retained directory the execution walk has to find.
///
/// **The §11.3-item-2(b) answer, recorded against freeze `:672-680`
/// (2026-09-01, E4.2).** *For a Git-directory catalog target, which durable root
/// binds a managed parent's prefix?* Its OWN retained root — the Git directory —
/// never the workspace root; and so no production managed parent exists on that
/// variant at all. Four facts settle it: (i) the purposes' declared components
/// are workspace-relative (`bootstrap/managed.rs`), so under a Git directory
/// they have no `.gwz` ancestor and both merge-start purposes fail their minimum
/// retained-parent count — pinned as production behaviour by
/// `tests_provider.rs`'s
/// `a_git_directory_target_refuses_the_workspace_rooted_managed_paths`;
/// (ii) `CheckedActionRequestV1::for_managed_parents` pins
/// `PreCatalogRootKindV1::Workspace` unconditionally, so no managed-parent
/// action can be identified against a Git-directory root kind at all; (iii) this
/// door's lease is workspace-rooted by construction (`catalog_lease/witness.rs`,
/// `WorkspaceRuntime` arm); (iv) no production caller builds a Git-directory
/// catalog lease. Route (b) therefore stands for TEST topologies, where the
/// parent is fixture-placed, and the owner decision closes as: the workspace
/// root binds it, the other variant carrying no production parent.
pub(crate) fn bootstrap_merge_start_parents(
    workspace_id: &str,
    admission: CatalogMutationLeaseV1<'_>,
    execution: CatalogMutationLeaseV1<'_>,
) -> ModelResult<()> {
    let refuse = |cause| render_catalog_refusal("merge start parents", cause);
    let admitted = admit_merge_start_managed_parents(
        workspace_id,
        recover_or_create(admission).map_err(refuse)?,
    )
    .map_err(refuse)?;
    let Some(admitted) = admitted else {
        return Ok(());
    };
    let catalog = recover_or_create(execution).map_err(refuse)?;
    execute_merge_start_managed_parents(workspace_id, &admitted, &catalog)
        .map(|_proved| ())
        .map_err(refuse)
}

/// R2-E Step E4.2 — O13's substantive half on the creation path.
///
/// Row `:280` asks the v1 checked store for "the same purposes and artifact
/// actions" its converted siblings use; this is the creation verb — a checked
/// replacement whose expected fact is `Missing`, publishing onto an absent leaf
/// inside an already-retained parent.
pub(crate) fn create_merge_store_record(
    root: &Path,
    relative: &Path,
    goal: &[u8],
) -> ModelResult<()> {
    let artifact = CheckedArtifact::acquire(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("merge record '{}'", relative.display()),
    )?;
    // Row `:273`'s clause said out loud, rather than left as the generic
    // ambiguity `classify_replace_exact` reports for an absent parent.
    if !artifact.parent_is_canonical()? {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "merge record parent is missing or noncanonical; it is bootstrapped before the record",
        ));
    }
    artifact.replace_exact(&CheckedArtifactFact::Missing, goal)
}

/// The catalog doors' error rendering, as a named function.
///
/// E4.1 review [P3-2]: inline, the three arms were unreachable from a test
/// without a filesystem that lacks the capability, so precondition 1's sentence
/// was driven only by hand on a real FAT32 volume. Named, a direct-constructor
/// row pushes each arm through it. `pub(super)` and not `pub(crate)`:
/// `CheckedFsError` is subsystem private and a crate-visible signature over it
/// trips `clippy::private_interfaces` (E4.1(c) flag 5, proven).
pub(super) fn render_catalog_refusal(label: &str, cause: CheckedFsError) -> ModelError {
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
}

/// E4.1's activation label, spelled once so door and guard cannot drift.
pub(super) const CATALOG_LABEL: &str = "merge artifact catalog";
