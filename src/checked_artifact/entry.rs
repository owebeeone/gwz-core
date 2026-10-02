//! Complete purpose-specific operations at the production checked boundary.
//!
//! The general checked capability never leaves this module. Callers receive
//! only facts or transition classifications for their declared merge purpose.

#![forbid(clippy::disallowed_methods)]

use crate::filesystem::{FileSystem, FsDirectory, FsKind};
use std::ffi::OsStr;
use std::path::{Component, Path};

use super::bootstrap::{CatalogMutationLeaseV1, probe_workspace_admission};
use super::capability::CheckedFsError;
use super::coordinator::execution::{
    admit_merge_start_managed_parents, execute_merge_start_managed_parents,
};
use super::observation::{IdentityGapEscape, directory_handles_ok};
use super::{
    CheckedArtifact, CheckedArtifactFact, CheckedArtifactPolicy, CheckedArtifactTransition,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

// These private parts share the checked entry's capability and lint boundary.
mod artifacts;
mod catalog;
mod observation;
mod recovery;

use super::{capability, identity};
use artifacts::*;
use observation::observe_filesystem_artifact_in;

pub(super) use catalog::{CATALOG_LABEL, render_catalog_refusal};
pub(crate) use catalog::{
    activate_workspace_catalog, bootstrap_merge_start_parents, create_merge_store_record,
    prepare_merge_start_parents_uncatalogued,
};
pub(crate) use recovery::{CrashRecoveryDecision, crash_recovery_decision_in};

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
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
) -> ModelResult<MergeArtifactFact> {
    observe_filesystem_artifact_in(
        filesystem,
        root,
        relative,
        ErrorCode::MergeRecoveryRequired,
        &format!("workspace artifact '{}'", relative.display()),
    )
}

pub(crate) fn replace_merge_root_artifact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: &[u8],
    goal: &[u8],
) -> ModelResult<()> {
    root_artifact(filesystem, root, relative)?
        .replace_exact(&CheckedArtifactFact::Bytes(expected.to_vec()), goal)
}

pub(crate) fn classify_replace_merge_root_artifact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: &[u8],
    goal: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    map_transition(
        root_artifact(filesystem, root, relative)?
            .classify_replace(&CheckedArtifactFact::Bytes(expected.to_vec()), goal)?,
    )
}

pub(crate) fn remove_merge_root_artifact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: &[u8],
) -> ModelResult<()> {
    root_artifact(filesystem, root, relative)?
        .remove_exact(&CheckedArtifactFact::Bytes(expected.to_vec()))
}

pub(crate) fn classify_remove_merge_root_artifact(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    map_transition(
        root_artifact(filesystem, root, relative)?
            .classify_remove(&CheckedArtifactFact::Bytes(expected.to_vec()))?,
    )
}

pub(crate) fn observe_merge_preservation_workspace(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    matches_expected(
        filesystem_fact(observe_filesystem_artifact_in(
            filesystem,
            root,
            relative,
            ErrorCode::PreservationEvidenceMismatch,
            "root preservation artifact",
        )?),
        expected,
    )
}

pub(crate) fn observe_merge_preservation_git_directory(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    matches_expected(
        filesystem_fact(observe_filesystem_artifact_in(
            filesystem,
            root,
            relative,
            ErrorCode::PreservationEvidenceMismatch,
            "root preservation artifact",
        )?),
        expected,
    )
}

pub(crate) fn replace_merge_preservation_workspace(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<()> {
    replace_expected(
        preservation_workspace(filesystem, root, relative)?,
        expected,
        goal,
    )
}

pub(crate) fn classify_merge_preservation_workspace(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: Option<&[u8]>,
) -> ModelResult<MergeArtifactTransition> {
    classify_expected(
        preservation_workspace(filesystem, root, relative)?,
        expected,
        goal,
    )
}

pub(crate) fn observe_merge_preservation_bundle(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
) -> ModelResult<bool> {
    let artifact = preservation_bundle(filesystem, root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    observe_expected_durable(artifact, expected)
}

pub(crate) fn classify_merge_preservation_bundle(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: &[u8],
) -> ModelResult<MergeArtifactTransition> {
    let artifact = preservation_bundle(filesystem, root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    map_transition(artifact.classify_replace(&fact(expected), goal)?)
}

pub(crate) fn replace_merge_preservation_bundle(
    filesystem: &dyn FileSystem,
    root: &Path,
    relative: &Path,
    expected: Option<&[u8]>,
    goal: &[u8],
) -> ModelResult<()> {
    let artifact = preservation_bundle(filesystem, root, relative)?;
    require_canonical_bundle_parent(&artifact)?;
    artifact.replace_exact(&fact(expected), goal)
}

fn prepare_merge_store_parents_in(filesystem: &dyn FileSystem, root: &Path) -> ModelResult<()> {
    CheckedArtifact::prepare_parent_in(
        filesystem,
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
#[cfg(test)]
pub(crate) fn crash_recovery_decision(root: &Path) -> ModelResult<CrashRecoveryDecision> {
    crash_recovery_decision_in(
        &crate::operation_context::OperationServices::existing(),
        root,
    )
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
fn create_merge_store_record_raw(
    filesystem: &dyn FileSystem,
    ids: &gwz_ids::IdSource,
    root: &Path,
    relative: &Path,
    goal: &[u8],
) -> ModelResult<()> {
    let path = root.join(relative);
    if filesystem.metadata(&path).is_ok() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("merge record '{}' already exists", relative.display()),
        ));
    }
    crate::verified_write::write_atomic_verified(filesystem, ids, &path, goal)
}

#[cfg(test)]
mod filesystem_observation_tests {
    use super::*;

    #[test]
    fn root_level_artifact_uses_the_retained_filesystem_path() {
        let filesystem = crate::filesystem::make_filesystem();
        let workspace = filesystem.test_workspace().unwrap();
        crate::filesystem::write_for_test(&workspace.path().join("gwz.lock"), b"lock bytes")
            .unwrap();

        assert_eq!(
            observe_merge_root_artifact(&filesystem, workspace.path(), Path::new("gwz.lock"))
                .unwrap(),
            MergeArtifactFact::Bytes(b"lock bytes".to_vec())
        );
    }
}
