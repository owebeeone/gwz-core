//! In-process proof using existing operation envelopes; no new wire service.
use super::driver_tests::{block_on, common, fixture_url};
use super::*;
use crate::{InitFromSourcesRequest, InitFromSourcesResponse, ResponseEnvelope, ResponseMeta};
use gwz_transport::protocol::MessageKind;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[path = "../../tests/transport_backend/python_embedding.rs"]
mod python;

#[derive(Clone, Copy)]
pub(super) enum Consumer {
    Rust,
    Python,
}
impl Consumer {
    pub(super) fn bytes(self, name: &str, value: &crate::Cbor) -> Vec<u8> {
        let bytes = crate::encode(value);
        match self {
            Self::Rust => bytes,
            Self::Python => python::roundtrip(name, &bytes),
        }
    }
    pub(super) fn request(self, value: &InitFromSourcesRequest) -> InitFromSourcesRequest {
        let result = InitFromSourcesRequest::from_cbor(&crate::decode(
            &self.bytes("InitFromSourcesRequest", &value.to_cbor()),
        ))
        .unwrap();
        assert_eq!(&result, value);
        result
    }
    pub(super) fn response(self, value: &InitFromSourcesResponse) -> InitFromSourcesResponse {
        let result = InitFromSourcesResponse::from_cbor(&crate::decode(
            &self.bytes("InitFromSourcesResponse", &value.to_cbor()),
        ))
        .unwrap();
        assert_eq!(&result, value);
        result
    }
}
pub(super) fn extract_request(value: &mut InitFromSourcesRequest) -> Option<Attachment> {
    value
        .meta
        .transport_message
        .take()
        .map(|message| (value.meta.request_id.clone(), message))
}
pub(super) fn extract_response(value: &mut InitFromSourcesResponse) -> Option<Attachment> {
    value
        .response
        .meta
        .transport_message
        .take()
        .map(|message| (value.response.meta.request_id.clone(), message))
}
pub(super) fn embedded(
    consumer: Consumer,
    request: &InitFromSourcesRequest,
    from_core: bool,
    item: Attachment,
) -> Attachment {
    assert_eq!(item.0, request.meta.request_id);
    let recovered = if from_core {
        // Attachment-bearing wrappers are consumed here, never published as
        // operation results. The actual final response is sent separately.
        let wrapped = InitFromSourcesResponse {
            response: ResponseEnvelope {
                meta: ResponseMeta {
                    request_id: item.0.clone(),
                    schema_version: request.meta.schema_version.clone(),
                    transport_message: Some(item.1.clone()),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        extract_response(&mut consumer.response(&wrapped)).unwrap()
    } else {
        let mut wrapped = request.clone();
        wrapped.meta.transport_message = Some(item.1.clone());
        let mut decoded = consumer.request(&wrapped);
        let recovered = extract_request(&mut decoded).unwrap();
        assert_eq!(
            &decoded, request,
            "transport extraction changed operation arguments"
        );
        recovered
    };
    assert_eq!(recovered, item);
    recovered
}
pub(super) struct Link {
    pub(super) stop: Arc<AtomicBool>,
    pub(super) pause: Arc<AtomicBool>,
    pub(super) paused: Arc<AtomicBool>,
    pub(super) counts: Arc<Mutex<BTreeMap<String, usize>>>,
    pub(super) hold_first_data: Arc<AtomicBool>,
    pub(super) worker: Option<JoinHandle<()>>,
}
impl Link {
    pub(super) fn new(
        core: TransportPort,
        endpoint: TransportPort,
        consumer: Consumer,
        request: InitFromSourcesRequest,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let counts = Arc::new(Mutex::new(BTreeMap::new()));
        let hold_first_data = Arc::new(AtomicBool::new(true));
        let hold_data = hold_first_data.clone();
        let (stopped, hold, acknowledged, seen) =
            (stop.clone(), pause.clone(), paused.clone(), counts.clone());
        let worker = thread::spawn(move || {
            struct Disconnect(TransportPort, TransportPort);
            impl Drop for Disconnect {
                fn drop(&mut self) {
                    self.0.disconnect();
                    self.1.disconnect();
                }
            }
            let _disconnect = Disconnect(core.clone(), endpoint.clone());
            let mut a = None;
            let mut b = None;
            let mut cx = Context::from_waker(Waker::noop());
            while !stopped.load(Ordering::Acquire) {
                if hold.load(Ordering::Acquire) {
                    acknowledged.store(true, Ordering::Release);
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                acknowledged.store(false, Ordering::Release);
                for (from, to, pending, from_core) in [
                    (&core, &endpoint, &mut a, true),
                    (&endpoint, &core, &mut b, false),
                ] {
                    if pending.is_none() {
                        match pin!(from.next_message()).poll(&mut cx) {
                            Poll::Ready(Ok(Some(item))) => {
                                *seen
                                    .lock()
                                    .unwrap()
                                    .entry(format!("{}:{:?}", from_core, item.1.kind))
                                    .or_default() += 1;
                                if item.1.kind == MessageKind::Data
                                    && hold_data.swap(false, Ordering::AcqRel)
                                {
                                    hold.store(true, Ordering::Release);
                                }
                                *pending = Some(embedded(consumer, &request, from_core, item));
                            }
                            Poll::Pending => {}
                            _ => {
                                stopped.store(true, Ordering::Release);
                                break;
                            }
                        }
                    }
                    if hold.load(Ordering::Acquire) {
                        break;
                    }
                    if let Some(item) = pending.as_ref() {
                        match pin!(to.deliver(item.clone())).poll(&mut cx) {
                            Poll::Ready(Ok(())) => *pending = None,
                            Poll::Pending => {}
                            Poll::Ready(Err(_)) => {
                                stopped.store(true, Ordering::Release);
                                break;
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
            core.disconnect();
            endpoint.disconnect();
        });
        Self {
            stop,
            pause,
            paused,
            counts,
            hold_first_data,
            worker: Some(worker),
        }
    }
    pub(super) fn hold(&self) {
        self.pause.store(true, Ordering::Release);
        let until = Instant::now() + Duration::from_secs(3);
        while !self.paused.load(Ordering::Acquire) {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !thread::panicking() {
                joined.unwrap();
            }
        }
    }
}
pub(super) fn run(consumer: Consumer, cancel: bool, disconnect: bool) {
    let fixture = common::SshdFixture::new();
    let endpoint_home = fixture.temp.path().join("endpoint-home");
    std::fs::create_dir_all(endpoint_home.join(".ssh")).unwrap();
    std::fs::copy(&fixture.known_hosts, endpoint_home.join(".ssh/known_hosts")).unwrap();
    std::fs::copy(
        fixture.temp.path().join("client_ed25519"),
        endpoint_home.join("key"),
    )
    .unwrap();
    let core_home = fixture.temp.path().join("core-home");
    std::fs::create_dir_all(core_home.join(".ssh")).unwrap();
    std::fs::write(core_home.join(".ssh/known_hosts"), b"").unwrap();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    let mut seed = 713u64;
    let bytes: Vec<u8> = (0..262144)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    let blob = server.blob(&bytes).unwrap();
    let mut tree = server.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = server.find_tree(tree.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    server.set_head("refs/heads/main").unwrap();
    server
        .commit(Some("HEAD"), &signature, &signature, "data", &tree, &[])
        .unwrap();
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(core_home, None)).unwrap();
    let core_port = runtime.install_cli().unwrap();
    require_cli_ssh(
        &runtime
            .capabilities(crate::TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into(),
            })
            .unwrap(),
        gwz_transport::protocol::AuthPolicy::SshExplicit,
    )
    .unwrap();
    let (endpoint, endpoint_port) =
        CliEndpoint::new(SshEndpointConfig::fixture(endpoint_home.clone(), None)).unwrap();
    let root = fixture.temp.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let mut original = InitFromSourcesRequest {
        meta: RequestMeta {
            request_id: "embedded-init".into(),
            schema_version: "gwz.protocol/v0".into(),
            transport: Some(crate::TransportOptions {
                placement: Some(TransportPlacement::Cli),
                default_identity: Some("key".into()),
                endpoint_path_base: Some(endpoint_home.to_string_lossy().into_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
        workspace_root: root.to_string_lossy().into_owned(),
        sources: vec![crate::SourceUrl {
            url: fixture_url(&fixture),
            path: Some("member".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    original = consumer.request(&original);
    assert!(extract_request(&mut original.clone()).is_none());
    let client_request = endpoint
        .register_request(&original.meta.request_id)
        .unwrap();
    let link = Link::new(core_port.clone(), endpoint_port, consumer, original.clone());
    let scope = Arc::new(block_on(runtime.request(original.meta.clone(), "init".into())).unwrap());
    link.hold();
    let scope_worker = scope.clone();
    let (tx, rx) = mpsc::channel();
    let root_worker = root.clone();
    let dispatched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = dispatched.clone();
    let worker = thread::spawn(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        tx.send(crate::workspace_ops::handle_init_from_sources(
            scope_worker.backend(),
            &root_worker,
            original,
            "init",
            &crate::operation::NullSink,
        ))
        .unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(40)).is_err(),
        "operation should wait for endpoint preflight while delivery is paused"
    );
    if cancel {
        scope.cancel();
    }
    if disconnect {
        core_port.disconnect();
    }
    link.pause.store(false, Ordering::Release);
    if !cancel && !disconnect {
        let until = Instant::now() + Duration::from_secs(10);
        while link.hold_first_data.load(Ordering::Acquire) || !link.paused.load(Ordering::Acquire) {
            assert!(Instant::now() < until, "never reached carried stream data");
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            rx.recv_timeout(Duration::from_millis(40)).is_err(),
            "operation completed across paused data delivery"
        );
        link.pause.store(false, Ordering::Release);
    }
    let result = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("operation stranded");
    worker.join().unwrap();
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    if cancel || disconnect {
        assert!(result.is_err());
        assert!(!root.join("member").exists());
    } else {
        let mut response = consumer.response(&result.unwrap());
        assert!(extract_response(&mut response).is_none());
        assert_eq!(response.response.meta.request_id, "embedded-init");
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert_eq!(std::fs::read(root.join("member/payload")).unwrap(), bytes);
        let counts = link.counts.lock().unwrap();
        for kind in [
            MessageKind::Bind,
            MessageKind::Bound,
            MessageKind::Open,
            MessageKind::Opened,
            MessageKind::Data,
            MessageKind::Window,
        ] {
            assert!(
                counts
                    .iter()
                    .any(|(k, n)| k.ends_with(&format!(":{kind:?}")) && *n > 0),
                "missing {kind:?}: {counts:?}"
            );
        }
    }
    let scope_report = block_on(Arc::try_unwrap(scope).ok().unwrap().finish());
    let request_report = block_on(client_request.finish());
    let endpoint_report = block_on(endpoint.shutdown());
    drop(link);
    let runtime_report = block_on(runtime.shutdown());
    assert_eq!(scope_report.pending_local_work, 0);
    assert_eq!(request_report.pending_local_work, 0);
    assert_eq!(endpoint_report.pending_local_work, 0);
    assert_eq!(runtime_report.pending_local_work, 0);
}
#[test]
fn rust_messages_embed_live_git_exchange() {
    run(Consumer::Rust, false, false);
}
#[test]
fn python_messages_embed_live_git_exchange() {
    run(Consumer::Python, false, false);
}
#[test]
fn rust_messages_cancel_while_delivery_is_paused() {
    run(Consumer::Rust, true, false);
}
#[test]
fn python_messages_cancel_while_delivery_is_paused() {
    run(Consumer::Python, true, false);
}
#[test]
fn rust_messages_close_while_delivery_is_paused() {
    run(Consumer::Rust, false, true);
}
#[test]
fn python_messages_close_while_delivery_is_paused() {
    run(Consumer::Python, false, true);
}
#[test]
fn message_embedding_adapter_preserves_correlation_and_payload() {
    let request = InitFromSourcesRequest {
        meta: RequestMeta {
            request_id: "r".into(),
            schema_version: "gwz.protocol/v0".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    for from_core in [true, false] {
        let item = (
            "r".into(),
            gwz_transport::binding::offer("session", gwz_transport::protocol::EndpointRole::Driver),
        );
        assert_eq!(
            embedded(Consumer::Rust, &request, from_core, item.clone()),
            item
        );
    }
}

#[test]
fn python_rejects_malformed_present_attachment_instead_of_dispatching_it() {
    let request = InitFromSourcesRequest {
        meta: RequestMeta {
            request_id: "malformed".into(),
            schema_version: "gwz.protocol/v0".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut value = request.to_cbor();
    if let crate::Cbor::Map(fields) = &mut value {
        if let crate::Cbor::Map(meta) = &mut fields.iter_mut().find(|(tag, _)| *tag == 1).unwrap().1
        {
            *meta.iter_mut().find(|(tag, _)| *tag == 10).unwrap() =
                (10, crate::Cbor::Text("not an envelope".into()));
        }
    }
    let encoded = crate::encode(&value);
    assert!(
        std::panic::catch_unwind(|| python::roundtrip("InitFromSourcesRequest", &encoded)).is_err()
    );
}
