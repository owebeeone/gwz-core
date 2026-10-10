//! The transport's endpoint side, SSH and HTTPS, which only the candidate
//! build (`gwz_transport_candidate`) compiles.
pub(crate) mod agent_auth;
pub(crate) mod agent_client;
pub(crate) mod agent_job;
pub(crate) mod agent_keys;
pub(crate) mod agent_socket;
pub(crate) mod git_turns;
pub(crate) mod placement_endpoint;
pub(crate) mod setup_retry;
pub(crate) mod socket_wait;
pub(crate) mod ssh_admission;
pub(crate) mod ssh_channel;
pub(crate) mod ssh_connection;
pub(crate) mod ssh_destination;
pub(crate) mod ssh_handoff;
pub(crate) mod ssh_key_auth;
pub(crate) mod ssh_key_container;
pub(crate) mod ssh_key_snapshot;
pub(crate) mod ssh_limits;
pub(crate) mod ssh_local;
pub(crate) mod ssh_network;
pub(crate) mod ssh_password_helpers;
pub(crate) mod ssh_pool;
pub(crate) mod ssh_pump;
pub(crate) mod ssh_remote;
pub(crate) mod ssh_setup_context;
pub(crate) mod ssh_shutdown;
pub(crate) mod ssh_worker;
pub(crate) mod stream_io;

pub(crate) mod ca_bundle;
pub(crate) mod https_auth;
pub(crate) mod https_connection;
pub(crate) mod https_destination;
pub(crate) mod https_handshake;
pub(crate) mod https_policy;
pub(crate) mod https_pool;
pub(crate) mod https_progress;
pub(crate) mod https_remote;
pub(crate) mod https_tls;
pub(crate) mod https_wake;
pub(crate) mod https_worker;
pub(crate) mod shared_reservation;
pub(crate) mod shutdown_watch;
cfg_if::cfg_if! {
    if #[cfg(any(all(test, unix), not(any(windows, target_vendor = "apple"))))] {
        pub(crate) mod verify_paths;
    }
}

pub(crate) mod https_operation;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) mod cut_proxy;
        // The TLS test servers are rustls, so that they run where the platform's
        // key store is denied (Windows under a key-based OpenSSH logon).
        pub(crate) mod https_fixture;
        // The HTTPS worker's in-process fixtures: production composes the
        // worker through the transport host's HTTPS endpoint instead.
        pub(crate) mod https_local;
        pub(crate) mod https_opening;
        pub(crate) mod loopback;
    }
}

// Windows parity (GwzTransportWindowsParityPlan.md): the modules below still compile on Unix only. Each block belongs
// to one step, whose rows are in scripts/checks/windows_parity/<step>.json, and that step ungates its block in place
// (the cfg_if wrapper becomes plain declarations) without touching the others. The blank lines between blocks are
// deliberate: they keep two steps' edits from ever being adjacent lines, which git reports as a conflict.

// Step 1.4: the setup chain (ungated), and the SSH endpoint's suites, which compile on every platform: each suite
// file keeps its own gate until its owner step.
pub(crate) mod idle_watch;
pub(crate) mod ssh_password;
pub(crate) mod ssh_setup;
cfg_if::cfg_if! { if #[cfg(test)] { mod ssh_tests; } }

// Step 3.5: Pageant's exchange primitive. Its pure rules compile everywhere; the OS calls are Windows'.
pub(crate) mod pageant_exchange;

// Step 1.5: option A's sshd close fixture (ungated; it has a Windows form of its forced script).
cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) mod ssh_close_fixture;
        // The native helper process of the close fixtures: a shell script on Windows cannot end a channel's output
        // and keep its process (the shells above it hold the channel's pipes).
        cfg_if::cfg_if! { if #[cfg(windows)] { pub(crate) mod fixture_helper; } }
    }
}

// Step 1.8: the integrated SSH test modules.
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod budget_wait_tests;
        mod job_budget_wait_tests;
        mod git_turns_tests;
        mod ssh_destination_tests;
        mod ssh_pump_clock_tests;
    }
}

// Step 4.3: the helper script warm-up.
cfg_if::cfg_if! { if #[cfg(all(test, unix))] { pub(crate) mod helper_script; } }

// The SSH test servers run on every platform: Windows' own `sshd.exe` stands in for `/usr/sbin/sshd` (step 1.1).
cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) mod fixture_host;
        pub(crate) mod fixture_job;
        pub(crate) mod ssh_fixture;
        pub(crate) mod ssh_password_fixture;
    }
}
