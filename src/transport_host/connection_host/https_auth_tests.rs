//! The connection host's HTTPS adapter answering challenges, against the fixture HTTPS servers (plan step 1.3c;
//! contract 7.2 and 7.3): a helper token and the logon session each mark the connection they authenticated
//! per-exchange scoped, and the logon-session route fails closed while a gate cannot demonstrate its scope. The
//! logon session here is the native module's fake provider; real SSPI runs on Windows (`https_sspi_tests.rs`).
use super::{
    https_tests::{ADVERTISEMENT, Case, Unavailable, body, challenging},
    tests::{block_on, operation},
};
use crate::git::{
    endpoint::{
        https_adapter::Binding,
        https_auth,
        https_fixture::Behaviour,
        https_worker::native::{self, NativeHost},
    },
    spawn_gate::{GateFault, SpawnGate},
};
use gwz_connection::HostPhase;
use gwz_transport::protocol::Disposition;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// A credential helper fixture that gives one Basic credential and counts its lookups.
fn helper(directory: &std::path::Path) -> https_auth::Config {
    let counter = directory.join("count");
    let mut config = crate::git::endpoint::helper_script::git_fixture(
        directory,
        "printf 'lookup\\n' >> \"$COUNTER\"\nprintf 'username=fixture\\npassword=private-fixture\\n\\n'",
    );
    config.environment.push((
        "COUNTER".into(),
        crate::git::endpoint::helper_script::script_path(&counter).into(),
    ));
    config
}

#[test]
fn a_helper_token_authenticated_connection_is_marked_per_exchange_scoped() {
    let case = Case::with(
        |seen| challenging(seen, "Basic realm=\"fixture\""),
        Behaviour::default(),
        |directory| (Some(helper(directory)), None),
        None,
    );
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    // The challenge and the helper's answer went on one connection.
    let seen = case.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [false, true]);
    assert_eq!(seen[0].1, seen[1].1);
    let scopes = case.scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].1.binding, Binding::HelperToken);
    assert!(scopes[0].1.binding.per_exchange());
    assert!(
        scopes[0].1.exchange.is_some(),
        "the exchange that bound it is named"
    );
    assert!(case.close());
}

#[test]
fn the_logon_session_route_marks_its_connection_per_exchange_scoped() {
    // A logon session whose provider completes at once (the native module's fake port), against a server that
    // challenges with NTLM: the connection the session authenticated is bound to it.
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        |_| (None, Some(native::publication_fixture())),
        None,
    );
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    let seen = case.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [false, true]);
    assert_eq!(
        seen[0].1, seen[1].1,
        "the token went on the challenged connection"
    );
    let scopes = case.scopes();
    assert_eq!(scopes[0].1.binding, Binding::LogonSession);
    assert!(scopes[0].1.binding.per_exchange());
    assert!(case.close());
}

/// The exchange is refused, the server saw no credential and the logon session took no step.
fn refused_before_any_credential(case: &Case, steps: &AtomicUsize) {
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    assert!(body(&mut exchange).is_err(), "the native route refuses");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(a.finish());
    let seen = case.seen.lock().unwrap().clone();
    assert!(
        seen.iter().all(|(authorized, _)| !authorized),
        "no token was sent: {seen:?}"
    );
    assert_eq!(steps.load(Ordering::SeqCst), 0, "no SSPI step was taken");
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(case.close());
}

#[test]
fn the_logon_session_route_refuses_while_the_process_gate_is_unavailable() {
    // The composition's gate cannot demonstrate its scope: the route fails closed (contract 7.3).
    let (caller, steps) = native::counting_fixture("localhost");
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        move |_| (None, Some(caller)),
        Some(Arc::new(Unavailable)),
    );
    refused_before_any_credential(&case, &steps);
    // The caller's own gate gave its supervisor no window (`NativeHost::on_gate`): no port, so no conversation.
    let host = NativeHost::on_gate(SpawnGate::faulted(GateFault::NotCreated), |_| {
        panic!("no supervisor is made on an unavailable gate")
    });
    let none = AtomicUsize::new(0);
    let case = Case::with(
        |seen| challenging(seen, "NTLM"),
        Behaviour::default(),
        move |_| (None, Some(host.caller())),
        None,
    );
    refused_before_any_credential(&case, &none);
}
