#![allow(
    dead_code,
    reason = "v1 lifecycle remains test-reachable until A1 activates production dispatch"
)]
mod authority;
mod checked;
mod finalization;
mod forward;
mod service;
mod store;
mod transition;

#[cfg(test)]
mod tests;
