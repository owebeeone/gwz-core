//! The SSH endpoint's suites, which ran in the separate `tests/transport_ssh`
//! crate until TR2.15 folded them in here, where the candidate build's CI
//! runs them.
//!
//! They drive the endpoint's own layers: the channel, pump and pool host; the
//! worker and the placement endpoint through the attachment path production
//! takes (`start_endpoint_open`); and native setup, trust, agent and selected
//! key authentication against [`super::ssh_fixture`]'s loopback `sshd` and
//! [`agent_fixture`]'s proxy agent, and TR2.8's key types against
//! [`key_fixture`]'s agent. Running them needs Git, `/usr/sbin/sshd`,
//! `ssh-keygen`, `ssh-agent`, `ssh-add`, `ps`, `kill`, `python3` and
//! `openssl`; none of them reads or changes the user's SSH configuration,
//! keys, agent or `known_hosts`. On Windows the server is the system's
//! `sshd.exe` (`fixture_host`), the agent twins are `agent_fixture_pipe` and
//! `key_fixture_pipe` (an in-process agent on a named pipe), and the
//! close fixtures' hard cases use `fixture_helper`. The rows that need a
//! channel whose output ends while the server's session lives (the stuck-close
//! rows) run on every platform against `ssh_controlled_server`, a small
//! test-controlled SSH server, since a Windows `sshd.exe` cannot serve them.
mod agent_auth;
mod agent_capacity;
mod agent_client;
mod ambient_words;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod agent_fixture;
        // The login fixture of the agent rows (`agent_auth`, `network_agent`): the server and an agent that lists a
        // rejected key and the trusted one.
        use agent_fixture as login_fixture;
    } else {
        pub(crate) mod agent_fixture_pipe;
        mod agent_pipe_server;
        mod agent_login_fixture_pipe;
        use agent_login_fixture_pipe as login_fixture;
    }
}
mod agent_keys;
mod agent_wait;
mod attachment;
mod background_close;
mod channel;
mod cleanup_capacity;
mod encrypted_key;
mod host_case;
mod idle_loss;
mod idle_loss_budget;
mod key_container;
mod key_files;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod key_fixture;
    } else {
        mod key_fixture_pipe;
    }
}
mod key_material;
mod key_types;
mod local_endpoint;
// Step 3.6: Pageant as the endpoint's agent, over a synthetic Pageant and over PuTTY's own (Windows only: so is Pageant).
cfg_if::cfg_if! { if #[cfg(windows)] { mod pageant_endpoint; mod real_pageant; } }
mod max_startups;
mod network;
mod network_agent;
mod no_agent;
mod password_helpers;
mod placement_endpoint;
mod platform_limits;
mod pool_host;
mod pooled;
mod pooled_remote;
mod pump;
mod pump_close;
mod regression;
mod release_wake;
mod remote_bridge;
mod retry;
// Step 1.6: a connection another binding last used is leased only after this binding's reuse proof.
pub(crate) mod reuse_agent;
mod reuse_auth;
mod rsa_sha1;
mod selected_key;
mod selected_pool;
mod supervised;
mod worker;
