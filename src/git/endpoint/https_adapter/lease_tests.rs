//! A start that fails after the host's lease (findings F8 follow-up, item 3): the HTTPS adapter claims the connection
//! (`Shared::lease`) before it queues the exchange's start, as the SSH adapter does. A start no worker takes is refused
//! (its client reads the failure), and its lease is released: none is left standing to keep the worker from retiring
//! that connection on an idle loss.
use super::{Command, HttpsAdapter, HttpsInputs, HttpsSettings};
use crate::git::endpoint::{
    https_auth::HelperSlots, https_connection, shared_reservation::Authority, ssh_adapter::Shared,
};
use gwz_connection::{AuthContext, AuthScope, EndpointAdapter, ExchangeSpec, GitService};
use gwz_transport::pool::{
    Action, Config, ConnectionId, Identity, Key, Owner, PoolMachine, Request,
};
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
    thread,
};
use tokio::sync::mpsc::UnboundedReceiver;

/// A connection id as the host's pool names one.
fn connection() -> ConnectionId {
    let mut pool = PoolMachine::new(Config::default()).unwrap();
    pool.request(Request::new(
        Key::https("localhost", 443),
        Identity::Https,
        Owner::new("s", "o"),
    ))
    .unwrap();
    let Some(Action::Connect { connection, .. }) = pool.next_action() else {
        panic!("the pool asks for a connection");
    };
    connection
}

#[test]
fn a_start_no_worker_takes_releases_the_lease() {
    let mut adapter = HttpsAdapter::new(
        HttpsInputs {
            tls: https_connection::Config::default(),
            helper_slots: HelperSlots::new(),
            authority: Authority::new(256, 256),
            settings: HttpsSettings {
                allocation_timeout_ms: 20_000,
                connect_timeout_ms: 20_000,
                io_timeout_ms: 20_000,
                interaction_timeout_ms: 20_000,
                cleanup_timeout_ms: 2_000,
            },
        },
        Arc::new(crate::transport_host::composition::process_safety()),
    );
    // The worker's thread starts, but its command channel is already closed, as when it has ended after a disposal.
    let (_, receiver) = adapter.start.take().unwrap();
    adapter.start = Some((
        Box::new(|receiver: UnboundedReceiver<Command>, _: Arc<Shared>| {
            drop(receiver);
            thread::Builder::new().spawn(|| {})
        }),
        receiver,
    ));
    let connection = connection();
    // Any command starts the worker; this one changes nothing.
    adapter.cancel_connect(connection);
    let spec = ExchangeSpec::git(
        Key::https("localhost", 443),
        Identity::Https,
        GitService::UploadPackAdvertisement,
        "/repo",
    )
    .unwrap();
    let auth = AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(()));
    let mut exchange = adapter
        .start_exchange(connection, &spec, &auth)
        .expect("refused work is the exchange's failure, as before");
    let mut buffer = [0u8; 16];
    // The refusal ended the work before start_exchange returned: one poll reads the failure.
    assert!(
        matches!(
            exchange.poll_read(&mut Context::from_waker(Waker::noop()), &mut buffer),
            Poll::Ready(Err(_))
        ),
        "the work was refused"
    );
    assert!(!adapter.shared.leased(connection), "the lease was released");
    // The connection is as it was before the lease: the worker may retire it, and a retired one is refused.
    assert!(adapter.shared.retire(connection));
    assert_eq!(
        adapter
            .start_exchange(connection, &spec, &auth)
            .map(drop)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotConnected
    );
}
