//! The SSH endpoint's suites, which ran in the separate `tests/transport_ssh`
//! crate until TR2.15 folded them in here, where the candidate build's CI
//! runs them.
//!
//! They drive the endpoint's own layers: the channel, pump and pool host; the
//! worker and the placement endpoint through the attachment path production
//! takes (`start_endpoint_open`); and native setup, trust, agent and selected
//! key authentication against [`super::ssh_fixture`]'s loopback `sshd` and
//! [`agent_fixture`]'s proxy agent. Running them needs Git, `/usr/sbin/sshd`,
//! `ssh-keygen`, `ssh-agent`, `ssh-add`, `ps` and `kill`; none of them reads
//! or changes the user's SSH configuration, keys, agent or `known_hosts`.
mod agent_auth;
mod agent_capacity;
mod agent_client;
mod agent_fixture;
mod agent_wait;
mod attachment;
mod channel;
mod cleanup_capacity;
mod host_case;
mod key_container;
mod local_endpoint;
mod max_startups;
mod network;
mod placement_endpoint;
mod pool_host;
mod pooled;
mod pooled_remote;
mod pump;
mod regression;
mod remote_bridge;
mod retry;
mod selected_key;
mod selected_pool;
mod supervised;
mod worker;

/// Runs `test`, an ignored test of `module`, alone in a child of this test
/// binary, and asserts that the child ran exactly that test and passed. The
/// supervised-job and cleanup budgets in `agent_job` are process-wide, so a
/// test that fills them, or that must be the first to start their supervisor,
/// needs a process no other test shares.
fn in_child(module: &str, test: &str) {
    let module = module.split_once("::").map_or(module, |(_, path)| path);
    let test = format!("{module}::{test}");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &test,
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
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
