//! Unit tests for the work detector, grouped by the behaviour they pin.

pub(crate) use gwz_repo_contract::{
    BytePath, NativeOperation, Observation, PhysicalState, SuppressionFlag, UnknownKind,
    UnknownReason, WorkKind, WorkObservation,
};

pub(crate) use super::*;
pub(crate) use gwz_repo_contract::{SuppressedEntry, WorkEntry};

mod provenance;
mod records;
mod reporting;
mod support;
mod suppressed;
mod worktree;

pub(crate) use support::*;
