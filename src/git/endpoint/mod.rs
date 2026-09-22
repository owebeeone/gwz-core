//! Candidate SSH endpoint, admitted only by the isolated full-core harness.
#![allow(dead_code, unused_imports)]
pub(crate) mod agent_auth;
pub(crate) mod agent_client;
pub(crate) mod agent_job;
pub(crate) mod agent_socket;
pub(crate) mod placement_endpoint;
pub(crate) mod ssh_admission;
pub(crate) mod ssh_channel;
pub(crate) mod ssh_connection;
pub(crate) mod ssh_destination;
pub(crate) mod ssh_endpoint;
pub(crate) mod ssh_key_auth;
pub(crate) mod ssh_key_container;
pub(crate) mod ssh_key_snapshot;
pub(crate) mod ssh_local;
pub(crate) mod ssh_network;
pub(crate) mod ssh_pool;
pub(crate) mod ssh_pump;
pub(crate) mod ssh_remote;
pub(crate) mod ssh_setup;
pub(crate) mod ssh_shutdown;
pub(crate) mod ssh_worker;
pub(crate) mod stream_io;

pub(crate) mod https_auth;
pub(crate) mod https_connection;
pub(crate) mod https_destination;
pub(crate) mod https_policy;
pub(crate) mod https_pool;
pub(crate) mod https_remote;
pub(crate) mod https_worker;
pub(crate) mod shared_reservation;
pub(crate) mod https_local;
pub(crate) mod https_progress;

pub(crate) mod https_operation;
pub(crate) mod https_opening;
