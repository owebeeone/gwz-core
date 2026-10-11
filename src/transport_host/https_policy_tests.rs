//! Policy and failure receipts exercised through the actual host mux.
use super::https_tests::{endpoint_home, fixture, meta};
use super::*;
use crate::git::endpoint::{https_auth, https_remote::RpcIo};
use gwz_transport::protocol::{AuthMethod, ErrorCode as TransportError, Facts, GitService, Opened};
use std::{
    io::Read,
    sync::atomic::{AtomicUsize, Ordering},
};

fn auth(root: &std::path::Path) -> https_auth::Config {
    crate::git::endpoint::helper_script::git_fixture(
        root,
        "while IFS= read -r line; do [ -z \"$line\" ] && break; done\nprintf 'username=fixture\\npassword=sentinel-h2-token\\n\\n'\n",
    )
}
fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}
use std::future::Future;

mod admission;
mod auth_receipts;
mod cancellation;
mod route_policy;
