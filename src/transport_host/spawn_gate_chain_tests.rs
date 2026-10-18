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

// The same chain from the host's native route (operator decision D31): the gate the supervisor's launch guard takes
// is the gate the transport's helpers, request context and backends hold, because `NativeHost` made them together.
// On Unix the supervisor is unavailable, so the guard is taken from the builder it was handed; the gate excludes
// because the rows make it so.

/// A host over an excluding `gate`, and the guard its supervisor's builder was handed.
fn native_host(gate: &SpawnGate) -> (NativeHost, gwz_sspi::LaunchGuard) {
    let handed = std::cell::RefCell::new(None);
    let host = NativeHost::on_gate(gate.clone(), |guard| {
        *handed.borrow_mut() = Some(guard);
        Err(gwz_sspi::ErrorKind::UnsupportedPlatform)
    });
    let guard = handed
        .take()
        .expect("the supervisor is built with the host's launch guard");
    (host, guard)
}

#[test]
fn the_supervisors_launch_guard_holds_the_hosts_gate_for_as_long_as_it_is_held() {
    let gate = SpawnGate::with_exclusion(true);
    let (host, guard) = native_host(&gate);
    assert!(gate.try_hold().is_some(), "building the host takes no hold");
    let launch = guard.hold();
    assert!(
        gate.try_hold().is_none(),
        "a worker's creation window holds the gate"
    );
    assert!(
        host.gate().try_hold().is_none(),
        "the host's gate is the guard's"
    );
    drop(launch);
    assert!(
        gate.try_hold().is_some(),
        "dropping the guard's value releases it"
    );
}

#[test]
fn a_native_hosts_callers_enter_a_transport_whose_spawns_wait_for_a_worker_launch() {
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
        let (host, guard) = native_host(&gate);
        let runtime = TransportRuntime::with_https_native(
            SshEndpointConfig::fixture(endpoint_home(root.path()), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            host.caller(),
        )
        .unwrap();
        let request_meta = meta("native-gate-chain");
        let request = runtime
            .request(request_meta.clone(), "fetch".into())
            .await
            .unwrap();
        let derived = request
            .backend()
            .with_transport(root.path(), request_meta.transport.as_ref())
            .unwrap()
            .expect("a backend derived for the request's transport options");

        // A worker's creation window, as the supervisor's launch takes it.
        let launch = guard.hold();
        assert!(
            request.backend().spawn_gate.try_hold().is_none(),
            "the request's backend does not wait for a worker launch"
        );
        assert!(
            derived.spawn_gate.try_hold().is_none(),
            "a derived backend does not wait for a worker launch"
        );
        assert!(
            request.context.spawn_gate().try_hold().is_none(),
            "the request context does not wait for a worker launch"
        );
        drop(launch);
        assert!(request.backend().spawn_gate.try_hold().is_some());
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

// A host's own backends (operator decision D47): the operations a host runs outside the transport (gwz-py's
// non-network requests, and those under the transport off switch) get their backends from the host, so their `git
// commit`, `git tag` and libgit2 credential helper take the gate the supervisor's worker launches take.

#[test]
fn a_native_hosts_own_backend_holds_the_hosts_gate_and_no_other() {
    let gate = SpawnGate::with_exclusion(true);
    let (host, _guard) = native_host(&gate);
    let backend = host.backend();
    {
        let hold = gate.hold();
        assert!(
            backend.spawn_gate.try_hold().is_none(),
            "the host's own backend has a gate of its own, not the host's"
        );
        drop(hold);
    }
    assert!(backend.spawn_gate.try_hold().is_some());
    assert!(host.gates(&backend));
    assert!(
        host.clone().gates(&backend),
        "a clone of a host is the same host"
    );
    assert!(
        host.gates(&host.backend()),
        "each backend of one host shares its gate"
    );
    assert!(
        !host.gates(&crate::git::Git2Backend::new()),
        "a plain backend is on a gate of its own"
    );
    let other = SpawnGate::with_exclusion(true);
    let (other_host, _other_guard) = native_host(&other);
    assert!(
        !host.gates(&other_host.backend()),
        "another host's backend is on another gate"
    );
}
