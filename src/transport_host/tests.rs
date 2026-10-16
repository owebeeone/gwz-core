use super::*;
use crate::RequestMeta;
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

pub(super) fn config() -> SshEndpointConfig {
    SshEndpointConfig::fixture(std::env::temp_dir().join("nonexistent-endpoint-home"), None)
}
/// The endpoint session's bootstrap refusals (the in-process Local endpoint is the same `Session::endpoint`): a Bind
/// for a request nobody registered creates no authority, and registering after that is refused too. The Cli twin is
/// `cli_placement_tests`.
#[test]
fn unregistered_bind_cannot_create_endpoint_authority() {
    let (endpoint, port) = session::Session::endpoint(config()).unwrap();
    let mut bind =
        gwz_transport::binding::offer("session", gwz_transport::protocol::EndpointRole::Driver);
    bind.bind.as_mut().unwrap().versions = vec![2];
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        pin!(port.deliver(("unknown".into(), bind))).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert!(ClientRequest::new(endpoint.clone(), "later").is_err());
}
/// A registration retired before the driver's Bind cannot be revived by that Bind arriving late, which the Local
/// route races too: `open_request` admits the client before the driver binds. The Cli twin is `cli_placement_tests`.
#[test]
fn registration_dropped_before_bind_cannot_be_resurrected_by_late_message() {
    let (endpoint, port) = session::Session::endpoint(config()).unwrap();
    let owner = ClientRequest::new(endpoint.clone(), "cancelled").unwrap();
    drop(owner);
    let mut bind = gwz_transport::binding::offer(
        "late-session",
        gwz_transport::protocol::EndpointRole::Driver,
    );
    bind.bind.as_mut().unwrap().versions = vec![2];
    bind.bind.as_mut().unwrap().receive_limits = session::limits();
    let delivery = wait(port.deliver(("cancelled".into(), bind)));
    assert!(
        delivery.is_err(),
        "late Bind must not revive a retired request"
    );
    assert!(ClientRequest::new(endpoint.clone(), "cancelled").is_err());
    endpoint.close();
    assert_eq!(wait(endpoint.cleanup()).pending_local_work, 0);
}
#[test]
fn operation_policy_carries_max_retries_and_an_older_writer_leaves_it_absent() {
    use crate::cbor::Cbor;
    let policy = crate::OperationPolicy {
        max_retries: Some(0),
        ..Default::default()
    };
    let decoded = crate::OperationPolicy::from_cbor(&policy.to_cbor()).unwrap();
    assert_eq!(decoded.max_retries, Some(0));
    // A writer without the field, production's encoder among them, sends
    // keys 1 to 8 only; its policy decodes with the field absent.
    let Cbor::Map(mut entries) = policy.to_cbor() else {
        panic!("a policy encodes as a map");
    };
    entries.retain(|(key, _)| *key != 9);
    let older = crate::OperationPolicy::from_cbor(&Cbor::Map(entries)).unwrap();
    assert_eq!(older.max_retries, None);
}
#[test]
fn request_installs_its_resolved_pool_capacity_before_bind() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let mut custom = meta("capacity-custom");
    custom.policy = Some(crate::OperationPolicy {
        concurrency: Some(400),
        max_connections_per_host: Some(50),
        ..Default::default()
    });
    let request = wait(runtime.request(custom, "operation-one".into())).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    assert_eq!(
        endpoint.capacity_for_test(),
        Some(pool::Capacity {
            per_user_host: 50,
            per_host: 50,
            total: 400,
            max_requests: 1024,
        })
    );
    wait(request.finish());
    let request = wait(runtime.request(meta("capacity-default"), "operation-two".into())).unwrap();
    assert_eq!(
        endpoint.capacity_for_test(),
        Some(pool::Capacity {
            per_user_host: 32,
            per_host: 32,
            total: 256,
            max_requests: 1024,
        })
    );
    wait(request.finish());
    wait(runtime.shutdown());
}
#[test]
fn live_request_between_leases_refuses_a_different_physical_capacity() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let first = wait(runtime.request(meta("capacity-first"), "first".into())).unwrap();
    let mut different = meta("capacity-different");
    different.policy = Some(crate::OperationPolicy {
        max_connections_per_host: Some(16),
        ..Default::default()
    });
    let error = wait(runtime.request(different.clone(), "different".into()))
        .err()
        .expect("a live request blocks physical policy replacement even between leases");
    assert_eq!(
        error.code,
        crate::model::ErrorCode::TransportCapacityConflict
    );
    wait(first.finish());
    let retry = wait(runtime.request(different, "different-retry".into()))
        .expect("pre-registration capacity refusal leaves request_id retryable");
    wait(retry.finish());
    wait(runtime.shutdown());
}
#[test]
fn waiting_for_admission_leadership_times_out_before_registration() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    endpoint.hold_admission_for_test();
    let request = meta("waiting-capacity");
    let began = std::time::Instant::now();
    let error = wait(runtime.request(request.clone(), "waiting".into()))
        .err()
        .expect("held admission leadership must time out");
    assert!(error.message.contains("timed out"), "{error:?}");
    assert!(began.elapsed() < std::time::Duration::from_secs(6));
    endpoint.release_admission_for_test();
    let retry = wait(runtime.request(request, "waiting-retry".into()))
        .expect("timed-out wait must not consume its request ID");
    wait(retry.finish());
    wait(runtime.shutdown());
}
#[test]
fn capacity_wait_after_leadership_does_not_register_request_id() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    endpoint.hold_capacity_for_test();
    let request = meta("late-capacity");
    let began = std::time::Instant::now();
    let error = wait(runtime.request(request.clone(), "first".into()))
        .err()
        .expect("capacity gate must time out");
    assert!(error.message.contains("timed out"), "{error:?}");
    assert!(began.elapsed() < std::time::Duration::from_secs(6));
    endpoint.release_capacity_for_test();
    let retry = wait(runtime.request(request, "retry".into()))
        .expect("pre-registration timeout must leave the ID reusable");
    wait(retry.finish());
    wait(runtime.shutdown());
}

#[test]
fn dropping_staged_capacity_installation_closes_the_generation() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    endpoint.hold_retirement_for_test();
    let mut changed = meta("staged-capacity");
    changed.policy = Some(crate::OperationPolicy {
        max_connections_per_host: Some(16),
        ..Default::default()
    });
    let mut request = Box::pin(runtime.request(changed, "first".into()));
    let mut context = Context::from_waker(Waker::noop());
    let began = std::time::Instant::now();
    while !endpoint.retirement_waiting_for_test() {
        match request.as_mut().poll(&mut context) {
            Poll::Pending => {}
            Poll::Ready(Err(error)) => panic!("capacity installation failed early: {error:?}"),
            Poll::Ready(Ok(_)) => panic!("capacity installation unexpectedly completed"),
        }
        assert!(began.elapsed() < std::time::Duration::from_secs(2));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    drop(request);
    assert!(
        endpoint.is_closed(),
        "dropped transaction closes mutated endpoint"
    );
    assert!(wait(runtime.request(meta("later"), "later".into())).is_err());
    wait(runtime.shutdown());
}
fn meta(id: &str) -> RequestMeta {
    RequestMeta {
        request_id: id.into(),
        schema_version: "gwz.protocol/v0".into(),
        ..Default::default()
    }
}

pub(super) fn wait<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        assert!(std::time::Instant::now() < deadline, "host future stranded");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn local_scope_clones_share_retirement_and_complete_without_a_network_exchange() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let meta = meta("local");
    let scope = wait(runtime.request(meta.clone(), "op".into())).unwrap();
    let context = scope.context.clone();
    context.validate(&meta, "op").unwrap();
    let mut other = meta.clone();
    other.request_id = "other".into();
    assert!(context.validate(&other, "op").is_err());
    assert!(context.validate(&meta, "other").is_err());
    assert_eq!(wait(scope.finish()).pending_local_work, 0);
    assert!(context.validate(&meta, "op").is_err());
    assert_eq!(wait(runtime.shutdown()).pending_local_work, 0);
}

#[test]
fn cancelling_one_bound_request_preserves_its_sibling_and_future_requests() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let first = wait(runtime.request(meta("first"), "op1".into())).unwrap();
    let second_meta = meta("second");
    let second = wait(runtime.request(second_meta.clone(), "op2".into())).unwrap();
    first.cancel();
    assert_eq!(wait(first.finish()).pending_local_work, 0);
    second.context.validate(&second_meta, "op2").unwrap();
    assert_eq!(wait(second.finish()).pending_local_work, 0);
    let third = wait(runtime.request(meta("third"), "op3".into())).unwrap();
    assert_eq!(wait(third.finish()).pending_local_work, 0);
    wait(runtime.shutdown());
}

#[test]
fn cloned_cancellation_handle_only_cancels_its_live_request() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<super::TransportCancellation>();
    let runtime = TransportRuntime::new(config()).unwrap();
    let first_meta = meta("handle-first");
    let first = wait(runtime.request(first_meta.clone(), "op1".into())).unwrap();
    let second_meta = meta("handle-second");
    let second = wait(runtime.request(second_meta.clone(), "op2".into())).unwrap();
    let handle = first.cancellation_handle();
    let clone = handle.clone();
    std::thread::spawn(move || clone.cancel()).join().unwrap();
    assert!(first.context.validate(&first_meta, "op1").is_err());
    second.context.validate(&second_meta, "op2").unwrap();
    wait(first.finish());
    handle.cancel();
    second.context.validate(&second_meta, "op2").unwrap();
    wait(second.finish());
    wait(runtime.shutdown());
}

#[test]
fn finish_after_supervisor_shutdown_does_not_strand_request_owner() {
    let runtime = TransportRuntime::new(config()).unwrap();
    let scope = wait(runtime.request(meta("r"), "op".into())).unwrap();
    wait(runtime.shutdown());
    std::thread::sleep(Duration::from_millis(20));
    let report = wait(scope.finish());
    assert_eq!(report.pending_local_work, 0);
    assert!(!report.peer_cleanup_confirmed);
}
