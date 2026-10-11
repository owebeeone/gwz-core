//! Fixture scripts that product code, or a fixture server, executes in tests.
//!
//! On Windows a script is run by Git for Windows' own shell, as Git runs a credential helper (`sh`, found beside
//! Git); [`write_helper_script`] only writes it there, since the warm-up below answers two problems Windows does
//! not have (the following paragraphs). A fake `git` cannot be a script on Windows, where the program is created as
//! an `.exe`: [`write_git_fixture`] is Unix's, and a Windows test uses [`git_fixture`] or configures the real Git
//! with a script helper.
//!
//! On macOS the first exec of a newly created executable waits for XProtect's
//! [`git_fixture`] is the form a test uses, on both platforms: a `Config` for a fake `git` whose body is the shell
//! script the test gives. On Unix the script is the program; on Windows the program is the test executable run in
//! `fixture_helper`'s `fake-git` mode, which runs the script under Git for Windows' `sh` (plan step 4.11).
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
/// A path as the text of a shell script spells it: the path itself on Unix, and on Windows its forward-slash form
/// (`C:/Users/x`), which Git for Windows' shell reads as it reads a POSIX path.
pub(crate) fn script_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Puts a copy of the program `from` at `to`, as the repair of a missing helper: a hard link where the volume allows
/// one, since the program on Windows is the test executable.
pub(crate) fn install(from: &std::path::Path, to: &std::path::Path) {
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).unwrap();
    }
}

/// The real Git, as an absolute path: the first `git` (`git.exe` on Windows) in an absolute directory of this
/// process's `PATH`.
pub(crate) fn real_git() -> std::path::PathBuf {
    let name = format!("git{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|entry| entry.is_absolute())
        .map(|entry| entry.join(&name))
        .find(|path| path.is_file())
        .expect("git must be installed for the credential-helper fixture")
}

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

        /// A fake `git` that runs `body` as a `/bin/sh` script, in `directory`, as a helper's `Config`: the script
        /// is the program, and the helper's environment is the test's alone.
        pub(crate) fn git_fixture(directory: &Path, body: &str) -> super::https_auth::Config {
            let executable = directory.join("git");
            write_git_fixture(&executable, body);
            super::https_auth::Config {
                executable,
                environment: Vec::new(),
            }
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

        /// A fake `git` that runs `body` as a `/bin/sh` script, in `directory`, as a helper's `Config`. The program
        /// is the test executable in `fixture_helper`'s `fake-git` mode, which runs the script under Git for
        /// Windows' `sh` with the arguments `git` was given; the Config's environment names them, and a test
        /// that gives the helper more environment appends to it. The `config` subcommand, which the product
        /// runs to read the configuration view, is answered by the real Git with no system or global file, as the
        /// Unix fixture does.
        pub(crate) fn git_fixture(directory: &Path, body: &str) -> super::https_auth::Config {
            use super::{
                fixture_helper::{GIT_SCRIPT_ENV, GIT_SH_ENV, HELPER_ENV, Mode},
                fixture_host::{SH_ENV, on_path, posix_path},
            };
            let shell = std::env::var_os(SH_ENV)
                .map(std::path::PathBuf::from)
                .or_else(|| on_path("sh.exe"))
                .expect("a POSIX sh.exe (on PATH, or named by GWZ_TEST_SH) is required, as Git for Windows provides");
            let git = real_git();
            let script = directory.join("git.sh");
            fs::write(
                &script,
                format!(
                    "if [ \"$1\" = config ]; then\n GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=NUL '{}' \"$@\"\n exit \"$?\"\nfi\n{body}\n",
                    posix_path(&git.display().to_string())
                ),
            )
            .unwrap();
            let mut environment: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
                (HELPER_ENV.into(), Mode::FakeGit.name().into()),
                (GIT_SH_ENV.into(), script_path(&shell).into()),
                (GIT_SCRIPT_ENV.into(), script_path(&script).into()),
            ];
            if let Some(root) = std::env::var_os("SystemRoot") {
                environment.push(("SystemRoot".into(), root));
            }
            // The program is the test executable, which finds the libraries it links through the `PATH` it was
            // built and run with: a helper's environment is cleared, and without this the loader fails it with
            // STATUS_DLL_NOT_FOUND.
            if let Some(path) = std::env::var_os("PATH") {
                environment.push(("PATH".into(), path));
            }
            super::https_auth::Config {
                executable: std::env::current_exe().unwrap(),
                environment,
            }
        }
    }
}
