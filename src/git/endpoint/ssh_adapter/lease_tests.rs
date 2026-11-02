//! A start that fails after the host's lease (findings F8 follow-up, item 3): `start_exchange` claims the connection
//! (`Claims::lease`) and only then makes the exchange's streams and queues its start. Every return after the claim that
//! queues no start releases it; a lease left standing would keep the worker from ever retiring that connection on an
//! idle loss or a failed reuse proof. Each row injects a failure at that point through the adapter's real path.
use super::{AdapterSettings, Command, Shared, SshAdapter};
use crate::git::endpoint::ssh_setup::SetupConnector;
use gwz_connection::{AuthContext, AuthScope, EndpointAdapter, ExchangeSpec, GitService};
use gwz_transport::pool::{
    Action, Config, ConnectionId, Identity, Key, Owner, PoolMachine, Request,
};
use std::{
    io,
    sync::{Arc, mpsc::Receiver},
    thread,
    time::Duration,
};

const SETTINGS: AdapterSettings = AdapterSettings {
    connect_timeout_ms: 20_000,
    io_timeout_ms: 20_000,
    interaction_timeout_ms: 20_000,
    cleanup_timeout_ms: 2_000,
};

/// A connection id as the host's pool names one.
fn connection() -> ConnectionId {
    let mut pool = PoolMachine::new(Config::default()).unwrap();
    pool.request(Request::new(
        Key::ssh("user", "host", 22),
        Identity::Ambient,
        Owner::new("s", "o"),
    ))
    .unwrap();
    let Some(Action::Connect { connection, .. }) = pool.next_action() else {
        panic!("the pool asks for a connection");
    };
    connection
}
/// An adapter whose setups never connect (nothing in these rows dials).
fn adapter(settings: AdapterSettings) -> SshAdapter {
    SshAdapter::new(
        |origin| {
            SetupConnector::isolated(origin, Duration::from_millis(500), |_, _, _| {
                Err(io::Error::other("the rows dial nothing"))
            })
        },
        settings,
    )
}
fn start(adapter: &mut SshAdapter, connection: ConnectionId) -> io::Result<()> {
    let spec = ExchangeSpec::git(
        Key::ssh("user", "host", 22),
        Identity::Ambient,
        GitService::UploadPackExchange,
        "/repo",
    )
    .unwrap();
    let auth = AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(()));
    adapter.start_exchange(connection, &spec, &auth).map(drop)
}

#[test]
fn a_start_whose_streams_cannot_be_made_releases_the_lease() {
    // An I/O bound past what a stream accepts (`i32::MAX` ms): the streams fail after the claim, before any start.
    let mut adapter = adapter(AdapterSettings {
        io_timeout_ms: i32::MAX as u64 + 1,
        ..SETTINGS
    });
    let connection = connection();
    // Any command starts the worker; this one changes nothing.
    adapter.cancel_connect(connection);
    let failed = start(&mut adapter, connection).unwrap_err();
    assert_ne!(
        failed.kind(),
        io::ErrorKind::NotConnected,
        "not a lost lease: {failed}"
    );
    assert!(!adapter.shared.leased(connection), "the lease was released");
    // The connection is as it was before the lease: the worker may retire it, and a retired one is refused.
    assert!(adapter.shared.retire(connection));
    let refused = start(&mut adapter, connection).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::NotConnected);
}

#[test]
fn a_start_no_worker_takes_releases_the_lease() {
    let mut adapter = adapter(SETTINGS);
    // The worker's thread starts, but its command channel is already closed, as when it has ended after a disposal.
    let (_, receiver) = adapter.start.take().unwrap();
    adapter.start = Some((
        Box::new(|receiver: Receiver<Command>, _: Arc<Shared>| {
            drop(receiver);
            thread::Builder::new().spawn(|| {})
        }),
        receiver,
    ));
    let connection = connection();
    adapter.cancel_connect(connection);
    let failed = start(&mut adapter, connection).unwrap_err();
    assert_eq!(failed.kind(), io::ErrorKind::BrokenPipe, "{failed}");
    assert!(!adapter.shared.leased(connection), "the lease was released");
    assert!(adapter.shared.retire(connection));
}
