//! TR2.11 (transport release plan amendment 2, §3.5): only a host context
//! reaches the transport. `Git2Backend::new()` carries none, so its SSH
//! operations take libgit2's native route, as 1.0.17's do, and construct no
//! transport endpoint. The same operations inside `with_local_transport` take
//! the transport.
//!
//! Both routes find `known_hosts` through `HOME`: libgit2 itself, and the
//! endpoint `with_local_transport` builds. So the test runs in a child of this
//! test binary whose `HOME` is a temporary directory that trusts the fixture's
//! host key, and this process's environment is never changed.
use super::*;

pub(super) const CHILD: &str = "GWZ_TR2_11_ROUTE_CHILD";

/// Runs the test `name` of `module`, a `module_path!()`, in a child of this
/// test binary, with a clean environment whose `HOME` is a new temporary
/// directory and which sets `CHILD`, and asserts that the child ran exactly
/// that test and passed.
pub(super) fn run_in_child(module: &str, name: &str) {
    run_in_child_with(module, name, &[]);
}

/// `run_in_child`, with the child's environment also holding `extra`.
pub(super) fn run_in_child_with(module: &str, name: &str, extra: &[(&str, String)]) {
    let home = tempfile::TempDir::new().unwrap();
    let module = module.split_once("::").map_or(module, |(_, path)| path);
    let test = format!("{module}::{name}");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env(CHILD, "1")
        .env("HOME", home.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command.envs(crate::git::endpoint::fixture_host::system_environment());
    for key in ["PATH", "TMPDIR", "GWZ_TEST_GIT", "GWZ_TEST_FS"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command
        .args(["--exact", &test, "--nocapture", "--test-threads", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the child failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert_eq!(
        stdout.matches(&format!("test {test} ... ok")).count(),
        1,
        "the child did not run exactly one test: {stdout}"
    );
}
