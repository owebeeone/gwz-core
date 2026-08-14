use std::ffi::OsString;
use std::path::PathBuf;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError};

mod authority;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
)]
mod bootstrap;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
)]
mod capability;
#[allow(
    dead_code,
    reason = "R2-C1 freezes the pure catalog grammar before C2 enables its owner"
)]
mod catalog;
#[allow(
    dead_code,
    reason = "R1 freezes the catalog grammar before R2 consumers are converted"
)]
mod catalog_names;
mod classification;
mod cleanup;
#[allow(
    dead_code,
    reason = "R2 freezes coordinator identity and schedule contracts before consumer conversion"
)]
mod coordinator;
#[allow(
    dead_code,
    reason = "R2 inventories checked entry points before production consumer activation"
)]
pub(crate) mod entry;
mod fault;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
)]
mod fault_v1;
mod identity;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
)]
mod leaf;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
)]
mod namespace;
mod observation;
mod platform;
mod policy;
#[allow(
    dead_code,
    reason = "R1 freezes interfaces before R2 consumer conversion"
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
