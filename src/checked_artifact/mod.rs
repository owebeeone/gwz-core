use std::ffi::OsString;
use std::path::PathBuf;

use cap_std::fs::Dir;

use crate::model::{ErrorCode, ModelError};

mod fault;
mod observation;
mod platform;
mod residue;
mod transition;

#[cfg(test)]
pub(crate) use fault::{
    CheckedArtifactFault, fail_next_checked_artifact_at, fail_next_checked_artifact_at_for,
    run_next_checked_artifact_at,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CheckedArtifactFact {
    Missing,
    Bytes(Vec<u8>),
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckedArtifactTransition {
    Before,
    After,
    Recoverable,
    Ambiguous,
}

pub(super) enum ParentState {
    Missing,
    Invalid,
    Open { dir: Dir, identity: (u64, u64) },
}

/// A no-follow capability for one workspace-relative regular-file artifact.
///
/// Acquisition never creates a managed parent. Mutations remain bound to the
/// retained parent and reobserve the exact expected leaf immediately before
/// their handle-relative linearization point.
pub(crate) struct CheckedArtifact {
    pub(super) root: Dir,
    pub(super) relative: PathBuf,
    pub(super) parent_relative: PathBuf,
    pub(super) parent: ParentState,
    pub(super) leaf: OsString,
    pub(super) private_root: PathBuf,
    pub(super) quarantine_parent: PathBuf,
    pub(super) code: ErrorCode,
    pub(super) label: String,
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
