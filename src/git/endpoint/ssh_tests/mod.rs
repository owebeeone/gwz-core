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
//! keys, agent or `known_hosts`.
mod agent_auth;
mod agent_capacity;
mod agent_client;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod agent_fixture;
    }
}
mod agent_keys;
mod agent_wait;
mod attachment;
mod background_close;
mod channel;
mod cleanup_capacity;
mod host_case;
mod idle_loss;
mod idle_loss_budget;
mod key_container;
mod key_files;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod key_fixture;
    }
}
mod key_material;
mod key_types;
mod local_endpoint;
mod max_startups;
mod network;
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod network_agent;
    }
}
mod no_agent;
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
mod rsa_sha1;
mod selected_key;
mod selected_pool;
mod supervised;
mod worker;
cfg_if::cfg_if! { if #[cfg(unix)] { mod password_helpers; } }
