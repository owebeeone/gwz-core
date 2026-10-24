//! The composition's gate (step 1.3a): every host of the process takes the same one, an unavailable one fails closed, and
//! the crate's `ProcessSafety` is the same gate.
use super::*;
use crate::git::{Git2Backend, spawn_gate::SpawnGate};
use crate::transport_host::NativeHost;
use std::sync::Arc;

fn host() -> NativeHost {
    NativeHost::new(Err(gwz_sspi::ErrorKind::WorkerUnavailable))
}

#[test]
fn two_native_hosts_in_one_process_share_the_gate() {
    let (a, b) = (host(), host());
    assert!(a.gates(&b.backend()), "b's backend is on a's gate");
    assert!(b.gates(&a.backend()), "a's backend is on b's gate");
}

#[test]
fn a_host_on_a_gate_of_its_own_and_a_hostless_backend_share_nothing() {
    let shared = host();
    let own = NativeHost::on_gate(SpawnGate::new(), |_| {
        Err(gwz_sspi::ErrorKind::WorkerUnavailable)
    });
    assert!(!shared.gates(&own.backend()) && !own.gates(&shared.backend()));
    assert!(
        !shared.gates(&Git2Backend::new()),
        "a backend with no host is on no host's gate"
    );
}

#[test]
fn the_entries_and_the_composition_hand_out_the_one_gate() {
    let safety = process_safety();
    assert!(spawn_gate().same_gate(safety.gate()));
    assert!(
        crate::git::endpoint::https_auth::HelperSlots::with_gate(spawn_gate())
            .gate()
            .same_gate(safety.gate())
    );
}

#[test]
fn the_gate_is_a_process_safety_and_excludes_where_a_window_needs_it() {
    let safety: Arc<dyn ProcessSafety> = Arc::new(process_safety());
    assert_eq!(safety.state(), SafetyState::Qualified);
    let held = safety.hold().expect("a qualified gate holds");
    assert_eq!(
        spawn_gate().try_hold().is_none(),
        SpawnGate::PROCESS_EXCLUDES,
        "a hold through the capability is a hold of the process gate"
    );
    drop(held);
    assert!(spawn_gate().try_hold().is_some());
}

/// Contract 7.3, fail closed: an unavailable gate says so, refuses to be held through the capability, and gives native
/// authentication no route, before any credential is read.
#[test]
fn an_unavailable_gate_fails_closed() {
    for (fault, reason) in [
        (GateFault::NotCreated, SafetyUnavailable::ObjectNotCreated),
        (GateFault::ForeignOwner, SafetyUnavailable::ForeignOwner),
    ] {
        let gate = ProcessGate {
            gate: SpawnGate::faulted(fault),
        };
        assert_eq!(gate.state(), SafetyState::Unavailable(reason));
        assert!(matches!(gate.hold(), Err(got) if got == reason));
        let host = NativeHost::on_gate(gate.gate().clone(), |_| {
            panic!("no supervisor is made on an unavailable gate")
        });
        assert!(matches!(
            host.supervisor(),
            Err(gwz_sspi::ErrorKind::ContainmentFailed)
        ));
    }
}

/// The dabeest row: two hosts built independently take one object, so the window held for one host's launch (the guard
/// the supervisor takes holds `gate.hold()`) excludes the other host's spawn.
#[test]
fn the_gate_one_host_holds_for_a_launch_excludes_the_spawn_of_another() {
    let (a, b) = (host(), host());
    let held = a.gate().hold();
    assert_eq!(b.gate().try_hold().is_none(), SpawnGate::PROCESS_EXCLUDES);
    drop(held);
    assert!(b.gate().try_hold().is_some());
}
