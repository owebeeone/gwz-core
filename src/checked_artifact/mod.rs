use std::ffi::OsString;
use std::path::PathBuf;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError};

mod admission;
mod authority;
// [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.] Class members here: the `dead_code` allows on `bootstrap`, `capability`, `fault_v1`, `leaf`, `namespace` and `protocol` below — RE-REASONED PERMANENT at E4.7 (2026-09-02). `entry`'s allow EXPIRED at the same step: measured (removed, `cargo check --all-targets` and `clippy -D warnings` both green), it suppressed nothing, every one of its doors having a production caller.
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
)]
mod bootstrap;
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
)]
mod capability;
mod catalog;
mod catalog_names;
mod classification;
mod cleanup;
mod coordinator;
pub(crate) mod entry;
mod fault;
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
)]
mod fault_v1;
mod identity;
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
)]
mod leaf;
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
)]
mod namespace;
mod observation;
mod platform;
mod policy;
#[allow(
    dead_code,
    reason = "frozen interface, PERMANENT: the R2-E consumer conversion \
              named here does not arrive — E4.4-E4.6 do not start and rows \
              `:275`-`:279`'s writers are carved out (dev-docs/\
              GwzM5-8R2E-CapabilityFreeAmendment.md §3/§7, ADOPTED \
              2026-09-02, on the operator's ruling of the same date); \
              narrowed at Phase 4 Step 4.3 to the subtrees that still carry \
              one. Any future removal is DR-1's, not an E4 step's."
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

/// DR-1 ship (1) W3's test-only seam (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.8, 2026-09-03): the merge-level rows arm it here, exactly as they arm
/// `fail_next_checked_artifact_at` above.
#[cfg(test)]
pub(crate) use capability::{InjectedVolumeDescription, with_identity_unavailable};

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
