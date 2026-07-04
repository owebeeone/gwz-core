//! Parity tests for the D1 Git backend diff primitive.
//!
//! Each test builds a real temp Git repository, runs `diff_manifest` through the
//! [`Git2Backend`], and compares the resulting manifest against porcelain
//! `git diff` output (`--name-status -M`, `--numstat`, `ls-tree` modes) so the
//! primitive is checked against Git itself, not just against expectations.

mod fixture;
mod t_binary;
mod t_cached;
mod t_classify;
mod t_handle;
mod t_log;
mod t_mode;
mod t_plan;
mod t_rename;
mod t_simple;
mod t_two_tree;
mod workspace_fixture;

pub(crate) use fixture::*;
