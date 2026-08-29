//! Workspace diff — core operation model and the Git backend primitive.
//!
//! D1 establishes the repo-scoped diff model ([`model`]) and the libgit2 diff
//! primitive over a single repository ([`git_diff`]). Higher phases layer on
//! top: D2 plans the workspace-level target set and pathspec routing, D3 builds
//! the manifest/output-log handlers, and D4 renders workspace-relative patch
//! bytes. Everything here works below the wire protocol — one repository,
//! repo-relative paths — and maps onto the generated `Diff*` messages at the D2
//! projection boundary.

mod classify;
mod git_diff;
mod handle_diff;
mod log_service;
mod model;
mod operands;
mod output;
mod plan;
pub mod render;
mod tagged;

#[cfg(test)]
mod tests;

pub(crate) use classify::classify_operands_for_command;
pub use classify::{
    ClassifiedOperands, RevContext, candidate_repos, classify_operands, default_rev_resolver,
};
pub(crate) use git_diff::build_repo_diff;
pub use git_diff::{ComparisonSpec, diff_repo, reject_unsupported_options, resolve_comparison};
pub use handle_diff::{DiffOutcome, handle_diff};
pub(crate) use handle_diff::{read_referenced_snapshots, resolved_cwd_rel};
pub use log_service::{
    DiffLog, DiffLogRegistry, LogReadRequest, LogReadResponse, LogReadState, LogRecord,
};
pub use model::{
    RepoDiffAlgorithm, RepoDiffComparison, RepoDiffComparisonKind, RepoDiffEntry, RepoDiffManifest,
    RepoDiffOptions, RepoDiffStatus, RepoDiffWhitespace,
};
#[cfg(test)]
pub(crate) use operands::parse_revision_arg;
pub use operands::{Endpoint, ParsedComparison, parse_comparison, parse_tagged_comparison};
pub(crate) use operands::{
    ParsedRevisionArg, parse_comparison_with_snapshot_ids, parse_revision_arg_with_snapshot_ids,
    parse_tagged_revision_args,
};
pub use output::{decode_record, encode_record};
pub use plan::{
    DiffPlan, ExcludedTarget, MaterializationOracle, PlanScope, PlannedTarget, ROOT_EXCLUDE_FIXED,
    plan_diff,
};
pub(crate) use tagged::{missing_exact_local_tags, validate_exact_tag_narrowing};
