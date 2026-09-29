//! Fixture scripts that product code, or a fixture server, executes in tests.
//!
//! On macOS the first exec of a newly created executable waits for XProtect's
//! assessment. Measured on 2026-09-29: running a freshly written helper script
//! took a median of 765 ms across 8 threads × 40 execs, against 3.3 ms to run
//! one that already existed, and under the parallel test load that queue
//! outran the tests' two-second helper deadlines. Every fixture script is
//! therefore run once, as a warm-up, before any test times it.
//!
//! Only the warm-up command carries `GWZ_TEST_HELPER_WARMUP`, never the process
//! environment, and the guard line exits before the script's body. No product
//! spawn can see it: the `gh` helper is spawned with `env_clear()` plus its
//! config's environment, and sshd builds a forced command's environment itself.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

const WARMUP: &str = "GWZ_TEST_HELPER_WARMUP";

/// Writes `body` as an executable `/bin/sh` script at `path`, then runs it once
/// so that its first-exec assessment happens outside every timed window.
pub(crate) fn write_helper_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ -z \"${WARMUP}\" ] || exit 0\n{body}"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    let status = Command::new(path)
        .env(WARMUP, "1")
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "fixture script warm-up failed: {}",
        path.display()
    );
}
