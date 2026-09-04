//! Complete purpose-specific operations at the production checked boundary.
//!
//! The general checked capability never leaves this module. Callers receive
//! only facts or transition classifications for their declared merge purpose.

#![forbid(clippy::disallowed_methods)]

use std::path::Path;

use super::bootstrap::{CatalogMutationLeaseV1, probe_workspace_admission};
use super::capability::CheckedFsError;
use super::catalog::recover_or_create;
use super::coordinator::execution::{
    admit_merge_start_managed_parents, execute_merge_start_managed_parents,
};
use super::observation::{IdentityGapEscape, directory_handles_ok};
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

/// The merge record's own parent prefix, spelled where the checked boundary can
/// see it. `bootstrap/managed.rs`'s `ManagedParentPurpose::MergeStore` declares
/// the same two components (`.gwz`, `merge`) and is the authority for the
/// ABOVE-bar route; this literal is the below-bar route's, and the two must not
/// drift.
const MERGE_RECORD_PARENT: &str = ".gwz/merge";

/// DR-1 ship (1) W3 — the CATALOG-FREE creation lease's parent half
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03).
///
/// Below the bar there is no catalog, so `bootstrap_merge_start_parents`'
/// managed-parent provider cannot install `.gwz/merge` and the preservation
/// bundle prefix. Both are still made through the LEGACY checked boundary's own
/// `prepare_parent` — the v0 store's route — and never by a raw `create_dir_all`
/// and never inside the managed-parent provider seam that
/// `interface_tests/r2d_seam_freeze.rs` freezes. `create_open` still refuses a
/// missing parent (charter §4.1), so this is the step that makes its refusal
/// unreachable on the warned path exactly as the bootstrap does on the other.
pub(crate) fn prepare_merge_start_parents_uncatalogued(root: &Path) -> ModelResult<()> {
    CheckedArtifact::prepare_parent(
        root,
        Path::new(MERGE_RECORD_PARENT),
        ErrorCode::MergeRecoveryRequired,
        "merge record parent",
    )?;
    prepare_merge_store_parents(root)
}

/// Whether this workspace's volume can prove the durable identity the checked
/// catalog needs for crash recovery.
///
/// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §2, 2026-09-03).
/// Crash recovery is a CAPABILITY, not a gate: below the bar the merge still
/// runs, warns once and activates no catalog; `--filesystem-strict` is the only
/// way to turn the absence back into a refusal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CrashRecoveryDecision {
    /// Identity proved; the catalog is activated exactly as it is today.
    Supported,
    /// Identity absent. `filesystem` is the volume's own name where the
    /// platform can give one, rendered `unknown` where it cannot.
    Unsupported {
        filesystem: Option<String>,
        gap: crate::MergeCrashRecoveryGap,
        /// M5d (`GwzM5-8M5d-Charter.md` §3, 2026-09-03): whether this volume
        /// proves the PERSISTENT FILE HANDLES the checked boundary's doors
        /// need — a second, independent question from the catalog's identity
        /// bar above it.
        ///
        /// The gap does not settle it, which is the whole reason this field
        /// exists: on Linux `no_durable_identity` implies the handle probe
        /// already failed, but `remote` and `volatile` do not — NFS and tmpfs
        /// answer `name_to_handle_at` — and those volumes must NOT carry the
        /// reverse-door limit. `false` means the record create publishes raw
        /// and the reverse doors may refuse; `true` is today's behaviour
        /// unchanged. It is carried only BELOW the bar: above it a handle
        /// failure is an anomaly at the door, not a capability the merge
        /// plans around.
        handles_ok: bool,
    },
}

/// The clause the ONE diagnostic gains on a handle-fail volume
/// (`GwzM5-8M5d-Charter.md` §3, "Reverse doors on handle-fail volumes",
/// third bullet: "Start on those volumes states this limit in the same
/// diagnostic (not a second unrelated warning class)").
///
/// It states the limit and stops. The escape itself is not repeated here:
/// the door that actually refuses renders it in full
/// (`capability::HANDLE_FAIL_REVERSE_DOOR_ESCAPE`), and a start that may
/// never abort at all should not be handed an abort procedure.
///
/// It begins with a space and is APPENDED, so ship (1)'s sentence stays
/// byte-identical at the head of the message for every pin that matches it.
const REVERSE_DOOR_LIMIT: &str = " Selected-root and --preserve abort may refuse until the workspace is on a handle-capable \
     volume.";

/// **On these three names.** The charter's own shorthand for them is
/// `to_protocol`, `warning` and the strict sentence; they are spelled with the
/// `crash_recovery_` prefix here because this module's `pub(crate)` surface is
/// an equality-pinned INVENTORY —
/// `check_checked_artifact_boundaries.py`'s `ENTRY_REFERENCES` scans every
/// production file for each visible name and requires the reference set to be
/// exactly the boundary's consumers. A bare `warning` or `to_protocol` matches
/// nine unrelated files between them, which is why every door in this file
/// already carries a long distinctive name. Same items, checkable names.
impl CrashRecoveryDecision {
    /// The response's machine truth (charter §3.4 channel 2): every consumer
    /// that must not depend on stderr reads this, not the diagnostic.
    pub(crate) fn crash_recovery_protocol(&self) -> crate::MergeCrashRecovery {
        match self {
            Self::Supported => crate::MergeCrashRecovery {
                supported: true,
                filesystem: None,
                gap: None,
                // M5d charter §3: ABSENT above the bar. The field says how a
                // below-bar merge behaves; above the bar there is nothing for
                // a consumer to plan around.
                handles_ok: None,
            },
            Self::Unsupported {
                filesystem,
                gap,
                handles_ok,
            } => crate::MergeCrashRecovery {
                supported: false,
                filesystem: filesystem.clone(),
                gap: Some(*gap),
                handles_ok: Some(*handles_ok),
            },
        }
    }

    /// The operator's exact sentence (charter §3.4). Drivers print `warning: `
    /// + this string on stderr; the `warning: ` prefix is theirs, not core's.
    ///
    /// **M5d (`GwzM5-8M5d-Charter.md` §3, 2026-09-03): ONE diagnostic, not
    /// two.** When the volume also fails the handle probe, this SAME string
    /// gains the reverse-door limit as an appended clause. Ship (1)'s sentence
    /// is byte-identical in front of it — the docs-manifest regex and the
    /// gwz-cli / gwz-py echo pins all match the first sentence — because a
    /// second Diagnostic would be a second warning class, which the charter
    /// forbids ("No second warning for the raw write itself").
    pub(crate) fn crash_recovery_warning(&self) -> String {
        let warning = format!(
            "{}. Merge will continue. Use --filesystem-strict to refuse.",
            self.gap_sentence()
        );
        match self {
            Self::Unsupported {
                handles_ok: false, ..
            } => format!("{warning}{REVERSE_DOOR_LIMIT}"),
            Self::Supported | Self::Unsupported { .. } => warning,
        }
    }

    /// The `--filesystem-strict` refusal (charter §3.6): the same sentence,
    /// then the one remedy a user can act on.
    pub(crate) fn crash_recovery_strict_refusal(&self) -> ModelError {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            format!(
                "checked catalog: {}; {}",
                self.gap_sentence(),
                super::capability::PERSISTENT_FILESYSTEM_IDENTITY_REMEDY
            ),
        )
    }

    /// `crash recovery is unsupported on <fs> (<parenthetical>)`, shared by the
    /// warning and the strict refusal so the two can never word the gap
    /// differently. `Supported` has no gap and never reaches either caller.
    fn gap_sentence(&self) -> String {
        let (filesystem, gap) = match self {
            Self::Supported => (None, None),
            Self::Unsupported {
                filesystem, gap, ..
            } => (filesystem.as_deref(), Some(*gap)),
        };
        let parenthetical = match gap {
            Some(crate::MergeCrashRecoveryGap::RemoteFilesystem) => "remote filesystem",
            Some(crate::MergeCrashRecoveryGap::VolatileFilesystem) => "volatile filesystem",
            Some(crate::MergeCrashRecoveryGap::NoDurableIdentity) | None => {
                "no durable filesystem identity"
            }
        };
        format!(
            "crash recovery is unsupported on {} ({parenthetical})",
            filesystem.unwrap_or("unknown")
        )
    }
}

/// The decision, made ONCE per process, before any lease is taken.
///
/// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §2/§3.1, 2026-09-03).
/// It runs the catalog's OWN admission probe — `dir_identity` on the retained
/// workspace target and its related Git directory, the same calls
/// `catalog_lease/target.rs::finish` makes — and creates, recovers and leases
/// nothing; in particular it never makes a `catalog-final` directory and never
/// touches the final slot.
///
/// **Which errors are an absent identity, and which are errors.** Every refusal
/// raised BY THE PROBE maps onto the warning path: `Unsupported` because that is
/// the bar, and `Io` because a probe that cannot answer is an absent identity,
/// not a reason to stop a merge that never needed the catalog. That includes the
/// Linux provider's volatile refusal (§3.2), which is a CATALOG-ADMISSION
/// refusal and not a merge refusal — the operator's ruling of 2026-09-03 (§0.1)
/// is explicit that tmpfs/ramfs warn with gap `volatile_filesystem` rather than
/// stopping the merge. An `Ambiguous` stays an error: it says the workspace is
/// not what it claims — a bare repository, a path that is not the worktree root,
/// an identity that changed under the probe — and none of those is a filesystem
/// capability the user can act on by dropping crash recovery.
///
/// **The gap comes from the description, never from a name list** (§0.1):
/// volatile wins over remote, remote over the bare absence, and `remote` is a
/// wording REASON, never a denylist. A description that cannot be taken at all
/// leaves `NoDurableIdentity` and an unnamed filesystem.
///
/// **M5d (`GwzM5-8M5d-Charter.md` §3, "Where handle capability is learned",
/// 2026-09-03): the decision also learns HANDLE capability.** Ship (1)'s
/// decision learned identity, remoteness and volatility, and the handle probe
/// was met later, at the create door — where its failure killed a start that
/// had already warned. So the decision now runs the create door's own probe
/// against the WORKSPACE ROOT (`directory_handles_ok`) and carries the answer
/// beside the gap. Three consequences the charter states explicitly, all
/// visible here: it is the workspace root and never `.gwz` (a first merge has
/// no `.gwz`, and a missing private directory is not a capability gap); NFS
/// and tmpfs, which answer `name_to_handle_at`, come out `handles_ok = true`
/// and carry no reverse-door limit; and the probe runs ONLY below the bar,
/// because above it a handle failure remains an anomaly at the door rather
/// than a capability the merge plans around.
pub(crate) fn crash_recovery_decision(root: &Path) -> ModelResult<CrashRecoveryDecision> {
    let probe = probe_workspace_admission(root);
    let cause = match probe.admitted {
        Ok(()) => return Ok(CrashRecoveryDecision::Supported),
        Err(cause) => cause,
    };
    if let CheckedFsError::Ambiguous { .. } = cause {
        return Err(render_catalog_refusal(CATALOG_LABEL, cause));
    }
    let (filesystem, gap) = match probe.volume {
        Some(volume) if volume.volatile => (
            volume.name,
            crate::MergeCrashRecoveryGap::VolatileFilesystem,
        ),
        Some(volume) if volume.remote => {
            (volume.name, crate::MergeCrashRecoveryGap::RemoteFilesystem)
        }
        Some(volume) => (volume.name, crate::MergeCrashRecoveryGap::NoDurableIdentity),
        None => (None, crate::MergeCrashRecoveryGap::NoDurableIdentity),
    };
    Ok(CrashRecoveryDecision::Unsupported {
        filesystem,
        gap,
        handles_ok: directory_handles_ok(root),
    })
}

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
fn root_artifact(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("workspace artifact '{}'", relative.display()),
        IdentityGapEscape::ReverseMergeDoor,
    )
}

fn preservation_bundle(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "preservation bundle",
        IdentityGapEscape::ReverseMergeDoor,
    )
}

fn preservation_workspace(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape(
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
        IdentityGapEscape::ReverseMergeDoor,
    )
}

fn preservation_git_directory(root: &Path, relative: &Path) -> ModelResult<CheckedArtifact> {
    CheckedArtifact::acquire_with_escape(
        CheckedArtifactPolicy::git_directory(root),
        relative,
        ErrorCode::PreservationEvidenceMismatch,
        "root preservation artifact",
        IdentityGapEscape::ReverseMergeDoor,
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
/// (`coordinator/execution.rs`'s admission session says the same). E4.2
/// converted the merge-record consumers; no further conversion arrives
/// (`GwzM5-8R2E-CapabilityFreeAmendment.md` §7, ADOPTED 2026-09-02 — E4.4-E4.6
/// as chartered do not start, and the three `finalization/execute.rs` forward
/// arms stay raw as the [R2-P3-1] dated residual on the operator's ruling (a)
/// of the same date). Re-pointed at E4.7, 2026-09-02. [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.]
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
///
/// **M5d step (3) — the RAW arm (`GwzM5-8M5d-Charter.md` §3, 2026-09-03).**
/// This door is the merge's ONLY forward checked door: `commit` is the
/// record-root exception's raw rewrite and the archive and publications are
/// raw already, so it is the one place a handle-fail volume could stop a
/// merge that has already been told it may continue. Below the handle bar it
/// therefore publishes through the neutral raw primitive instead of the
/// boundary, and the charter's table says so in one line: *record create =
/// raw (`write_atomic_verified`)*.
///
/// **Gated by the DECISION, not by a re-probe.** The caller threads the
/// decision `crash_recovery_decision` already made for this process (charter
/// §3.1's "decide once", extended to this door), so the door and the
/// diagnostic can never disagree about which volume this is. Exactly one
/// shape takes the raw arm — `Unsupported { handles_ok: false }`. `Supported`,
/// `Unsupported { handles_ok: true }` and `None` all keep the checked
/// publication, so a create-door handle failure while the decision said
/// handles were fine stays what it is today: an anomaly error, unchanged text.
///
/// The raw arm keeps the checked arm's two guarantees that a user can
/// observe: the same NO-REPLACE semantics (an existing record is refused, not
/// overwritten) and the same re-read verification (the primitive compares the
/// published bytes back). What it does not keep is the catalog, which does
/// not exist on this volume, and crash recovery, which the charter states is
/// absent rather than degraded here.
pub(crate) fn create_merge_store_record(
    root: &Path,
    relative: &Path,
    goal: &[u8],
    crash_recovery: Option<&CrashRecoveryDecision>,
) -> ModelResult<()> {
    if matches!(
        crash_recovery,
        Some(CrashRecoveryDecision::Unsupported {
            handles_ok: false,
            ..
        })
    ) {
        return create_merge_store_record_raw(root, relative, goal);
    }
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

/// The record create's RAW arm, on a volume without persistent file handles
/// (`GwzM5-8M5d-Charter.md` §3; `GwzM5-8R2E-CapabilityFreeAmendment.md` §3 as
/// revised at this step — this function is the carved arm the entering
/// inventory row names, and `tests/capability_free_exception.rs` scans exactly
/// this region for boundary-door vocabulary).
///
/// A named function rather than an inline block for that reason and one more:
/// the arm is the thing the two gates pin, and a region a scan can extract by
/// signature is a region a reviewer can read whole. It is the ONLY production
/// site in the crate that names the neutral raw primitive.
///
/// **No-replace, kept.** The checked arm publishes with an expected fact of
/// `Missing`, so an existing record is a refusal and never an overwrite.
/// `rename_durable(replace = true)` inside the primitive would not refuse by
/// itself, so the guard is spelled here instead, with the same
/// `MergeRecoveryRequired` code and the same sentence `create_open`'s own
/// pre-flight uses. `symlink_metadata` and not `exists`: a symlink standing
/// where the record belongs must refuse, not be followed.
fn create_merge_store_record_raw(root: &Path, relative: &Path, goal: &[u8]) -> ModelResult<()> {
    let path = root.join(relative);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("merge record '{}' already exists", relative.display()),
        ));
    }
    crate::verified_write::write_atomic_verified(&path, goal)
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
