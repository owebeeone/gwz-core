use std::ffi::OsString;
use std::path::PathBuf;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError};

mod admission;
mod authority;
// [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.] Class members here: the `dead_code` allows on `bootstrap`, `capability`, `entry`, `fault_v1`, `leaf`, `namespace` and `protocol` below.
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod bootstrap;
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod capability;
mod catalog;
mod catalog_names;
mod classification;
mod cleanup;
mod coordinator;
#[allow(
    dead_code,
    reason = "the checked entry-point inventory is consumed by the legacy leaf writers that are \
              converted, and by R2-E for the rest; production activation of the remainder is \
              R2-E's (plan §5 item 1)"
)]
pub(crate) mod entry;
mod fault;
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod fault_v1;
mod identity;
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod leaf;
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod namespace;
mod observation;
mod platform;
mod policy;
#[allow(
    dead_code,
    reason = "frozen interface awaiting R2-E consumer conversion (plan §5 item 1); narrowed \
              at Phase 4 Step 4.3 to the subtrees that still carry one"
)]
mod protocol;
mod residue;
mod transition;

use policy::CheckedArtifactPolicy;

pub(crate) use bootstrap::{
    CatalogMutationLeaseV1, WorkspaceRuntimeLease, try_acquire_workspace_runtime,
};

#[cfg(test)]
pub(crate) use fault::{
    CheckedArtifactFault, fail_next_checked_artifact_at, fail_next_checked_artifact_at_for,
    run_next_checked_artifact_at,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum CheckedArtifactFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckedArtifactTransition {
    Before,
    After,
    Recoverable,
    Ambiguous,
}

enum ParentState {
    Missing,
    Invalid,
    Open {
        dir: Dir,
        identity: identity::ObjectIdentity,
    },
}

/// A no-follow capability for one workspace-relative regular-file artifact.
///
/// Acquisition never creates a managed parent. Mutations remain bound to the
/// retained parent and reobserve the exact expected leaf immediately before
/// their handle-relative linearization point.
struct CheckedArtifact {
    root: Dir,
    root_identity: identity::ObjectIdentity,
    canonical_path_identity: Vec<u8>,
    parent_relative: PathBuf,
    parent: ParentState,
    leaf: OsString,
    private_root: PathBuf,
    quarantine_parent: PathBuf,
    code: ErrorCode,
    label: String,
}

pub(super) fn io_error(code: ErrorCode, label: &str, cause: std::io::Error) -> ModelError {
    error(code, label, cause)
}

pub(super) fn error(code: ErrorCode, label: &str, detail: impl std::fmt::Display) -> ModelError {
    ModelError::new(code, format!("checked {label}: {detail}"))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod interface_tests;
