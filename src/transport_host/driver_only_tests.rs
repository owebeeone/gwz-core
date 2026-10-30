//! Plan step 1.7: the driver-only builder. Its rows: the setting that selects libgit2's native route is refused when the
//! host is built, with nothing built and nothing leaked (the builder holds no agent, key or helper), every request is
//! the `cli` placement, and a route the client's endpoint does not serve is refused before any effect, each by a
//! refusal that names the route. The type-level row (the builder takes no endpoint settings) is
//! `scripts/checks/check_driver_only_boundary.py`.
use super::driver_tests::block_on;
use super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError};
use crate::session_host::EnvironmentSnapshot;
use crate::transport_setting::Transport;
use crate::{RequestMeta, TransportOptions, TransportPlacement};
use std::{
    ffi::OsString,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tempfile::TempDir;

mod child_process;
mod fetch_push_and_backslash;
mod identity_split;
mod rewrites;
mod routes_and_binding;
mod scratch;
mod setting_and_placement;

pub(crate) use child_process::*;
pub(crate) use fetch_push_and_backslash::*;
pub(crate) use rewrites::*;
pub(crate) use routes_and_binding::*;
pub(crate) use setting_and_placement::*;
