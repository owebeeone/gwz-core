//! The composition's process safety (step 1.3a of dev-docs/GwzTransportReleaseConvergencePlan.md; operator decision D-E;
//! contract 7.3 of dev-docs/GwzConnectionApiContract.md): the one place a spawn gate is made.
//!
//! A composition root calls [`process_safety`] once and hands the result to every host it builds, so every host in the
//! process holds the same gate (`NativeHost::with_process_safety`, `HelperSlots::with_gate`, the request backends). The
//! entries that exist today (`NativeHost::new`, `with_local_transport` and its cancellable twin) call it themselves until
//! phase 3 moves the roots onto it. No production code makes a gate anywhere else; `scripts/checks/check_spawn_gate_minting.py`
//! holds that line.
//!
//! The gate is a [`gwz_connection::ProcessSafety`], the capability the connection host takes from the composition.
//! Elsewhere than Windows a spawn needs no creation window, so it is qualified and excludes nothing. On Windows it is a named
//! kernel object with a per-process name (`git/spawn_gate/kernel.rs`), and **fails closed**: if the object cannot be made,
//! or exists with an owner that is not this process's, `state()` is `Unavailable`, `hold()` returns the reason, and
//! native authentication refuses before it reads a credential. The residual: the claim is scoped to gwz callers, so a spawn
//! by code gwz does not write (a library's own `CreateProcess` with handle inheritance) is not excluded.
use crate::git::spawn_gate::{GateFault, SpawnGate};
use gwz_connection::{ProcessSafety, SafetyHold, SafetyState, SafetyUnavailable};

/// The process's spawn gate as the composition holds it. A clone is another handle to the same gate.
#[derive(Clone)]
pub struct ProcessGate {
    gate: SpawnGate,
}
impl ProcessGate {
    pub(crate) fn gate(&self) -> &SpawnGate {
        &self.gate
    }
}

/// The process's gate: the only site that makes one.
pub fn process_safety() -> ProcessGate {
    ProcessGate {
        gate: SpawnGate::process(),
    }
}

/// The process's gate, for the entries that build a host's helper slots and backends on it.
pub(crate) fn spawn_gate() -> SpawnGate {
    process_safety().gate
}

fn unavailable(fault: GateFault) -> SafetyUnavailable {
    match fault {
        GateFault::NotCreated => SafetyUnavailable::ObjectNotCreated,
        GateFault::ForeignOwner => SafetyUnavailable::ForeignOwner,
    }
}
impl ProcessSafety for ProcessGate {
    fn state(&self) -> SafetyState {
        match self.gate.fault() {
            None => SafetyState::Qualified,
            Some(fault) => SafetyState::Unavailable(unavailable(fault)),
        }
    }
    fn hold(&self) -> Result<SafetyHold, SafetyUnavailable> {
        match self.gate.fault() {
            Some(fault) => Err(unavailable(fault)),
            None => Ok(SafetyHold::new(self.gate.hold())),
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
