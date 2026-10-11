//! Fixture scripts that product code, or a fixture server, executes in tests.
//!
//! On Windows a script is run by Git for Windows' own shell, as Git runs a credential helper (`sh`, found beside
//! Git); [`write_helper_script`] only writes it there, since the warm-up below answers two problems Windows does
//! not have (the following paragraphs). A fake `git` cannot be a script on Windows, where the program is created as
//! an `.exe`: [`write_git_fixture`] is Unix's, and a Windows test configures the real Git with a script helper.
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
cfg_if::cfg_if! {
    if #[cfg(unix)] {
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

        /// A Git-fill fixture still uses native Git for the adopted read-only config
        /// commands. Its root configuration is empty and independent of the host.
        pub(crate) fn write_git_fixture(path: &Path, body: &str) {
            write_helper_script(
                path,
                &format!(
                    "if [ \"$1\" = config ]; then\n GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null /usr/bin/git \"$@\"\n exit \"$?\"\nfi\n{body}"
                ),
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
    } else if #[cfg(windows)] {
        use std::{fs, path::Path};

        /// Writes `body` as a `/bin/sh` script at `path`, which Git for Windows' shell runs: it reads the file, so
        /// there is no execute bit and nothing to warm up. The script is written with `\n` line ends.
        pub(crate) fn write_helper_script(path: &Path, body: &str) {
            fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        }
    }
}
