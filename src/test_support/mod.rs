//! Support the crate's tests share.
//!
//! `temp_dir.rs` holds no tests and names nothing outside std, so an
//! integration test under `tests/` reaches the same file by `#[path]`
//! (`tests/diff_render_spike.rs` does).

mod fixture_port;
mod git_daemon;
mod git_daemon_relay;
mod git_daemon_relay_tests;
mod git_daemon_tests;
mod spellings;
mod temp_dir;
mod tests;

pub(crate) use fixture_port::{fixture_listener, fixture_port};
pub(crate) use git_daemon::{GitDaemon, retry_a_lost_tail};
pub(crate) use spellings::case_variant;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub(crate) use spellings::symlink_spelling;
    }
}
pub(crate) use temp_dir::{TempDir, unique_dir};
