//! Tests of the native commit and tag route.
//!
//! * `route` proves each condition of the route rule picks the Git CLI, by
//!   the route chosen and, for hooks, by the hook's side effect;
//! * `differential` runs the same operation natively on one repository and
//!   through the Git CLI on an identical twin, and compares object ids,
//!   references, reflogs, the index and failure codes;
//! * `route_config` covers the route's inputs beyond hooks and signing;
//! * `environment` does the same with Git's own variables set, in a clean
//!   child process.

#[macro_use]
mod support;

mod differential_commit;
mod differential_edges;
mod differential_tag;
mod environment;
mod notes;
mod route_commit;
mod route_config;
mod route_tag;
