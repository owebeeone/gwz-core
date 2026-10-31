//! Real Windows SSPI through the connection host's HTTPS adapter (plan step 1.3c; contract 7.2 and 7.3): the product's
//! SSPI worker process and the logon session of the test, against the loopback SSPI acceptor fixture of plan step 4.8
//! (`native::tests::real_sspi`), which checks the connection's channel binding as an Extended Protection server does.
//! A Windows-only module, not a skip. No account is created and no password is used.
use super::{
    https_tests::{Case, Unavailable, body, context_of, operation_of},
    tests::block_on,
};
use crate::git::endpoint::{
    https_adapter::Binding,
    https_fixture::Behaviour,
    https_worker::native::tests::real_sspi::{
        Expects, acceptor_handler, real_caller, settle_worker,
    },
};
use gwz_transport::protocol::Disposition;
use std::sync::{Arc, Mutex};

fn settle(supervisor: Arc<gwz_sspi::Supervisor>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(settle_worker(supervisor));
}

#[test]
fn the_logon_session_authenticates_through_the_host_and_its_connection_is_per_exchange_scoped() {
    let (caller, supervisor) = real_caller();
    for scheme in ["NTLM", "Negotiate"] {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let (accepted, native) = (statuses.clone(), caller.clone());
        let case = Case::with(
            move |_| acceptor_handler(scheme, scheme, Expects::Right, accepted),
            Behaviour::default(),
            move |_| (None, Some(native)),
            None,
        );
        let a = case.operation("op-a");
        let mut exchange = case.exchange(&a, false);
        body(&mut exchange).unwrap_or_else(|error| panic!("{scheme}: {error:?}"));
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable,
            "{scheme}"
        );
        block_on(a.finish());
        // The enforcing server accepted the client's binding: its last status is success.
        assert_eq!(statuses.lock().unwrap().last(), Some(&0), "{scheme}");
        let scopes = case.scopes();
        assert_eq!(scopes.len(), 1, "{scheme}");
        assert_eq!(scopes[0].1.binding, Binding::LogonSession, "{scheme}");
        assert!(scopes[0].1.binding.per_exchange(), "{scheme}");
        assert!(case.close(), "{scheme}");
    }
    settle(supervisor);
}

#[test]
fn the_logon_session_route_refuses_on_windows_while_the_process_gate_is_unavailable() {
    let (caller, supervisor) = real_caller();
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let accepted = statuses.clone();
    let case = Case::with(
        move |_| acceptor_handler("NTLM", "NTLM", Expects::Right, accepted),
        Behaviour::default(),
        move |_| (None, Some(caller)),
        Some(Arc::new(Unavailable)),
    );
    let a = case.operation("op-a");
    let mut exchange = case.exchange(&a, false);
    assert!(body(&mut exchange).is_err(), "the native route refuses");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(a.finish());
    assert!(
        statuses.lock().unwrap().is_empty(),
        "the acceptor never saw a token"
    );
    assert!(case.close());
    settle(supervisor);
}

#[test]
fn the_logon_session_is_each_operations_own_through_the_host_on_windows() {
    // R3 (Code P2-2): one instance, two operations with equal profiles. A's context holds the real caller its entry
    // captured, and B's holds none: A's logon session authenticates, B's challenge goes unanswered, and the enforcing
    // server never sees a second token.
    let (caller, supervisor) = real_caller();
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let accepted = statuses.clone();
    let case = Case::with(
        move |_| acceptor_handler("NTLM", "NTLM", Expects::Right, accepted),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let a = operation_of(&case.handle, "op-a", context_of(None, Some(caller)));
    let mut exchange = case.exchange(&a, false);
    body(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    let tokens = statuses.lock().unwrap().len();
    assert_eq!(statuses.lock().unwrap().last(), Some(&0), "A authenticated");
    let b = operation_of(&case.handle, "op-b", context_of(None, None));
    let mut exchange = case.exchange(&b, false);
    assert!(body(&mut exchange).is_err(), "B had no caller to answer with");
    block_on(exchange.finish());
    block_on(b.finish());
    assert_eq!(
        statuses.lock().unwrap().len(),
        tokens,
        "the acceptor saw a token from B"
    );
    assert!(case.close());
    settle(supervisor);
}
