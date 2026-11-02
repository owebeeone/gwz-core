//! An idle HTTPS connection's loss against the host's lease of it (findings F8 follow-up 2): the HTTPS adapter's worker
//! retired its idle connections on a loss with no claim, as the SSH worker did before F8. The host counts a connection
//! idle until it has read its `IdleClosed`, so it could lease one the worker had already disposed of, and that
//! exchange failed `Io` before its request was sent. The worker now retires through the adapter's claims
//! (`ssh_adapter/claims.rs`), whose retirement stands until the host has read the report. A start the host makes after
//! that read (a reuse verdict it read in the same pass) is refused by `SchemeRouter`, the production adapter shape
//! these rows run (fix round, CODE-P2-1).
use super::{
    https_tests::{ADVERTISEMENT, Case, body, served},
    idle_lease_tests::Gate,
    tests::{block_on, operation, wait_for},
};
use gwz_transport::protocol::Disposition;
use std::{io, sync::Arc};

#[test]
fn an_https_connection_retired_on_an_idle_loss_is_refused_until_the_host_reads_its_report() {
    let gate = Arc::new(Gate::default());
    let case = Case::held(served, gate.clone());
    let operation = operation(&case.handle, "op-https-idle-unread");
    let mut exchange = case.exchange(&operation, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    case.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    // The host reads no report now. The server's side of the idle connection goes: Hyper's driver ends, and the
    // worker disposes of the connection and reports it.
    gate.set(|state| state.hold_reports = true);
    case.proxy.cut_all();
    wait_for("the worker disposed of it and reported it", || {
        gate.idle_closed_untaken() == 1
    });
    assert_eq!(
        case.handle.snapshot().connections.idle,
        1,
        "the host has not read it"
    );
    // The host leases what it still counts idle. The start that releases the reports to the host is the one the
    // retirement refuses, and the opening asks again.
    let mut exchange = case.exchange(&operation, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    gate.read(|state| {
        assert_eq!(
            state.starts,
            [None, Some(io::ErrorKind::NotConnected), None],
            "the retired connection was refused, and a new one started"
        );
    });
    assert_eq!(case.accepted(), 2, "a fresh connection served it");
    block_on(operation.finish());
    case.wait("the retired connection is gone", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(case.close());
}

#[test]
fn a_reuse_verdict_read_with_the_connections_idle_closed_is_refused_and_the_opening_asks_again() {
    // Fix round, CODE-P2-1. Binding B's opening leases the idle connection binding A used, and the adapter authorizes
    // it at once (nothing authenticated it). The host reads no report now, so the verdict waits; the server's side of
    // the connection goes, and the worker retires it and reports `IdleClosed`. The host then reads both in one pass,
    // which forgets the adapter's claims of the connection, and serves the verdict. The router took the connection's
    // route away with the report, so it refuses the start (`NotConnected`) and the opening asks again; a start that
    // reached the adapter would be queued for a connection its worker no longer has, and fail `Io` before its request
    // was sent.
    let gate = Arc::new(Gate::default());
    let case = Case::held(served, gate.clone());
    let a = operation(&case.handle, "op-a-authorized");
    let mut exchange = case.exchange(&a, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    case.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    gate.set(|state| state.hold_reports = true);
    let b = operation(&case.handle, "op-b-authorized");
    let opening = b.open(case.spec(true)).unwrap().wait();
    wait_for("the verdict waits for the host", || {
        gate.authorized_untaken() == 1
    });
    case.proxy.cut_all();
    wait_for("the worker retired it and reported it", || {
        gate.idle_closed_untaken() == 1
    });
    // Both reports wait in the outbox: the host reads them in one pass, then serves the verdict.
    gate.release_reports();
    let mut exchange = block_on(opening).unwrap();
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    gate.read(|state| {
        assert_eq!(
            state.starts,
            [None, Some(io::ErrorKind::NotConnected), None],
            "the verdict's start was refused, and a new connection started"
        );
    });
    assert_eq!(case.accepted(), 2, "a fresh connection served B");
    block_on(b.finish());
    case.wait("the retired connection is gone", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(gate.claims_empty(), "nothing is left in the claims");
    assert!(case.close());
}
