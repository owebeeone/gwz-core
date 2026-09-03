//! The v1 record lifecycle. A1 activated production dispatch into this tree;
//! the compile gate and its `dead_code` allowance expired with the activation.

/// The boundary checker's compiler-root witness. Referenced un-gated from
/// `merge/mod.rs` so the positive-compile assertion survives the activation.
pub(super) const COMPILER_ROOT_SENTINEL: &str = module_path!();

mod archive;
mod archive_result;
mod authority;
mod checked;
mod events;
mod finalization;
mod forward;
mod reverse;
mod service;
mod start;
mod status;
mod store;
mod transition;

#[cfg(test)]
mod tests;

pub(in crate::workspace_ops::merge) use start::{handle_start_durable_v1, handle_v1_command};
