#![allow(
    dead_code,
    reason = "v1 lifecycle remains test-reachable until A1 activates production dispatch"
)]
pub(super) const COMPILER_ROOT_SENTINEL: &str = module_path!();

mod archive;
mod archive_result;
mod authority;
mod checked;
mod finalization;
mod forward;
mod reverse;
mod service;
mod status;
mod store;
mod transition;

#[cfg(test)]
mod tests;
