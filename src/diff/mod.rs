//! Workspace diff — core operation model and the Git backend primitive.
//!
//! D1 establishes the repo-scoped diff model ([`model`]) and the libgit2 diff
//! primitive over a single repository ([`git_diff`]). Higher phases layer on
//! top: D2 plans the workspace-level target set and pathspec routing, D3 builds
//! the manifest/output-log handlers, and D4 renders workspace-relative patch
//! bytes. Everything here works below the wire protocol — one repository,
//! repo-relative paths — and maps onto the generated `Diff*` messages at the D2
//! projection boundary.

mod git_diff;
mod handle_diff;
mod log_service;
mod model;
mod operands;
mod output;
mod plan;
pub mod render;

#[cfg(test)]
mod tests;

pub(crate) use git_diff::build_repo_diff;
pub use git_diff::{ComparisonSpec, diff_repo, reject_unsupported_options, resolve_comparison};
pub use handle_diff::{DiffOutcome, handle_diff};
pub use log_service::{
    DiffLog, DiffLogRegistry, LogReadRequest, LogReadResponse, LogReadState, LogRecord,
};
pub use model::{
    RepoDiffAlgorithm, RepoDiffComparison, RepoDiffComparisonKind, RepoDiffEntry, RepoDiffManifest,
    RepoDiffOptions, RepoDiffStatus, RepoDiffWhitespace,
};
pub use operands::{Endpoint, ParsedComparison, parse_comparison};
pub use output::{decode_record, encode_record};
pub use plan::{
    DiffPlan, ExcludedTarget, MaterializationOracle, PlanScope, PlannedTarget, ROOT_EXCLUDE_FIXED,
    plan_diff,
};
