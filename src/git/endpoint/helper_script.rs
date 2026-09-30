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
//!
//! On Linux the warm-up is also the script's "Text file busy" barrier. A child
//! that another test thread forks while `fs::write` holds the script open keeps
//! that write descriptor until the child execs, and Linux refuses to execute a
//! file that any process holds open for writing (ETXTBSY). The warm-up retries
//! until the script runs. Once it has, no process can hold the script open for
//! writing again, so the code under test never meets the error.
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WARMUP: &str = "GWZ_TEST_HELPER_WARMUP";
const BUSY_RETRY: Duration = Duration::from_millis(5);
const BUSY_DEADLINE: Duration = Duration::from_secs(10);

/// Writes `body` as an executable `/bin/sh` script at `path`, then runs it once
/// so that its first-exec assessment happens outside every timed window.
pub(crate) fn write_helper_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ -z \"${WARMUP}\" ] || exit 0\n{body}"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    let deadline = Instant::now() + BUSY_DEADLINE;
    let status = loop {
        match Command::new(path)
            .env(WARMUP, "1")
            .stdin(Stdio::null())
            .status()
        {
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < deadline =>
            {
                thread::sleep(BUSY_RETRY);
            }
            result => break result.unwrap(),
        }
    };
    assert!(
        status.success(),
        "fixture script warm-up failed: {}",
        path.display()
    );
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        #[test]
        fn the_warm_up_waits_out_a_process_that_holds_the_script_open_for_writing() {
            // This test's own descriptor stands in for the one a child forked
            // mid-write inherits. Linux refuses the exec while it is open.
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("helper.sh");
            fs::write(&path, "").unwrap();
            let writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
            let release = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                drop(writer);
            });
            write_helper_script(&path, "exit 0\n");
            release.join().unwrap();
        }
    }
}
