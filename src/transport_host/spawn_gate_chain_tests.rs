//! The host's spawn gate reaches every spawn through the real chain (WH2 contract C9): the helper slots a driver
//! entry makes, the runtime that takes them, the request's context, and the backend (and the backends derived from
//! it). A gate each link made for itself would pass every other row and leave the window open on Windows.
use super::https_tests::{endpoint_home, fixture, meta};
use super::*;
use crate::git::{GitBackend, spawn_gate::SpawnGate};
use gwz_transport::protocol::GitService;

#[test]
fn a_requests_backend_and_the_backends_derived_from_it_hold_the_hosts_gate() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async { fixture::response(200, GitService::UploadPackAdvertisement, "small") })
        }))
        .await;
        let root = tempfile::tempdir().unwrap();
        let gate = SpawnGate::with_exclusion(true);
        let slots = HelperSlots::with_gate(gate.clone());
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            slots.clone(),
        )
        .unwrap();
        let request_meta = meta("gate-chain");
        let request = runtime
            .request(request_meta.clone(), "fetch".into())
            .await
            .unwrap();
        let derived = request
            .backend()
            .with_transport(root.path(), request_meta.transport.as_ref())
            .unwrap()
            .expect("a backend derived for the request's transport options");

        // The helpers' launches take this gate; whoever holds it holds it against the backends too.
        let hold = slots.gate().hold();
        assert!(
            request.backend().spawn_gate.try_hold().is_none(),
            "the request's backend has a gate of its own, not the host's"
        );
        assert!(
            derived.spawn_gate.try_hold().is_none(),
            "a derived backend has a gate of its own, not the host's"
        );
        assert!(
            request.context.spawn_gate().try_hold().is_none(),
            "the request context has a gate of its own, not the host's"
        );
        drop(hold);
        assert!(request.backend().spawn_gate.try_hold().is_some());
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
