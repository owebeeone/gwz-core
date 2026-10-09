//! Support the crate's tests share.
//!
//! `temp_dir.rs` holds no tests and names nothing outside std, so an
//! integration test under `tests/` reaches the same file by `#[path]`
//! (`tests/diff_render_spike.rs` does).

mod git_daemon;
mod git_daemon_tests;
mod temp_dir;
mod tests;

pub(crate) use git_daemon::GitDaemon;
pub(crate) use temp_dir::{TempDir, unique_dir};
