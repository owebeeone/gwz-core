//! Candidate-only in-memory lifecycle probes.
//!
//! These tests deliberately stop at the mux boundary.  The peer is an
//! in-memory endpoint owner, so a failure here identifies host admission or
//! wake-up ordering without involving SSH, a repository, or a wire carrier.
use super::*;
use crate::git::endpoint::{ssh_channel::GitService, stream_io::BlockingStream};
use gwz_transport::{
    binding,
    mux::{self, Owner, Port},
    protocol::*,
};
use std::{
    future::Future,
    io::{self, Read},
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(3);

struct Harness {
    session: Arc<session::Session>,
    host: TransportPort,
    peer_owner: Owner,
    peer: Port,
    endpoint_id: String,
    trust_owner: String,
}

impl Harness {
    fn new(requests: &[(&str, &str)]) -> Self {
        let (session, host) = session::Session::driver(3000).expect("driver session");
        for (request, operation) in requests {
            session
                .register(request, Some((*operation).into()))
                .expect("host registration");
        }
        session.begin(requests[0].0).expect("host bind");
        let bind = wait_future(host.next_message())
            .expect("host bind result")
            .expect("host bind");
        let session_id = bind.1.session_id.clone();
        let endpoint_id = "in-memory-endpoint".to_owned();
        let trust_owner = "in-memory-owner".to_owned();
        let endpoint = binding::EndpointConfig {
            endpoint_id: endpoint_id.clone(),
            role: EndpointRole::Driver,
            schemes: vec![Scheme::Ssh],
            policies: vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit],
            limits: session::limits(),
            trust_owner: trust_owner.clone(),
        };
        let config = mux::Config {
            limits: session::limits(),
            ..Default::default()
        };
        let (peer_owner, peer) =
            Owner::new(mux::Mux::endpoint(&session_id, endpoint, config).expect("peer mux"));
        for (request, _) in requests {
            peer_owner
                .register(request, None)
                .expect("peer registration");
        }
        wait_future(peer.deliver(bind)).expect("peer bind delivery");
        let bound = wait_future(peer.next_message())
            .expect("peer bound result")
            .expect("peer bound");
        wait_future(host.deliver(bound)).expect("host bound delivery");
        Self {
            session,
            host,
            peer_owner,
            peer,
            endpoint_id,
            trust_owner,
        }
    }

    fn open(
        &self,
        request: &str,
        operation: &str,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> thread::JoinHandle<io::Result<BlockingStream>> {
        let session = self.session.clone();
        let request = request.to_owned();
        let operation = operation.to_owned();
        thread::spawn(move || {
            session.open(
                &request,
                &operation,
                "ssh://git@example.invalid/repository.git",
                GitService::UploadPack,
                Identity::default(),
                Arc::new(|_, _| {}),
                facts,
            )
        })
    }

    fn opened(&self, open: &Attachment) -> Attachment {
        (
            open.0.clone(),
            Envelope {
                version: 2,
                session_id: open.1.session_id.clone(),
                stream_id: open.1.stream_id,
                kind: MessageKind::Opened,
                opened: Some(Opened {
                    connection_id: "connection-1".into(),
                    reused: false,
                    endpoint_id: self.endpoint_id.clone(),
                    trust_owner: self.trust_owner.clone(),
                    facts: Facts::default(),
                    receive_limits: session::limits(),
                }),
                ..Default::default()
            },
        )
    }

    fn failed(&self, open: &Attachment, facts: Option<Facts>) -> Attachment {
        (
            open.0.clone(),
            Envelope {
                version: 2,
                session_id: open.1.session_id.clone(),
                stream_id: open.1.stream_id,
                kind: MessageKind::Failed,
                failed: Some(Failure {
                    code: gwz_transport::protocol::ErrorCode::RepositoryRefused,
                    effect: Effect::None,
                    facts,
                }),
                ..Default::default()
            },
        )
    }

    fn open_failed(&self, open: &Attachment) -> Attachment {
        (
            open.0.clone(),
            Envelope {
                version: 2,
                session_id: open.1.session_id.clone(),
                stream_id: open.1.stream_id,
                kind: MessageKind::OpenFailed,
                open_failed: Some(Failure {
                    code: gwz_transport::protocol::ErrorCode::Cancelled,
                    effect: Effect::None,
                    facts: None,
                }),
                ..Default::default()
            },
        )
    }

    fn late_closed(&self, open: &Attachment) -> Attachment {
        (
            open.0.clone(),
            Envelope {
                version: 2,
                session_id: open.1.session_id.clone(),
                stream_id: open.1.stream_id,
                kind: MessageKind::Closed,
                closed: Some(Closed {
                    disposition: Disposition::Discarded,
                    unread_response_discarded: true,
                    facts: Facts::default(),
                    failure: None,
                }),
                ..Default::default()
            },
        )
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.host.disconnect();
    }
}

fn wait_future<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let deadline = Instant::now() + WAIT;
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        assert!(Instant::now() < deadline, "in-memory mux future stranded");
        thread::sleep(Duration::from_millis(1));
    }
}

fn next_host(harness: &Harness) -> Attachment {
    wait_future(harness.host.next_message())
        .expect("host port")
        .expect("host message")
}

#[test]
fn carrier_drop_wakes_blocking_open() {
    let harness = Harness::new(&[("open", "operation")]);
    let facts_seen = Arc::new(AtomicBool::new(false));
    let facts_for_callback = facts_seen.clone();
    let pending = harness.open(
        "open",
        "operation",
        Arc::new(move |_| {
            facts_for_callback.store(true, Ordering::Release);
        }),
    );
    let open = next_host(&harness);
    wait_future(harness.peer.deliver(open)).expect("peer open delivery");

    harness.host.disconnect();
    let result = pending.join().expect("open worker");
    let error = match result {
        Ok(_) => panic!("carrier drop unexpectedly opened a stream"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(matches!(
        error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<gwz_transport::stream::Error>()),
        Some(gwz_transport::stream::Error::PeerFailed {
            code: gwz_transport::protocol::ErrorCode::CarrierLost,
            ..
        })
    ));
}

#[test]
fn carrier_drop_wakes_blocking_reader() {
    let harness = Harness::new(&[("open", "operation")]);
    let pending = harness.open("open", "operation", Arc::new(|_| {}));
    let open = next_host(&harness);
    wait_future(harness.peer.deliver(open.clone())).expect("peer open delivery");
    harness
        .peer_owner
        .send("open", &harness.opened(&open).1)
        .expect("peer opened");
    let opened = wait_future(harness.peer.next_message())
        .expect("peer opened result")
        .expect("peer opened message");
    wait_future(harness.host.deliver(opened)).expect("host opened delivery");
    let mut stream = pending.join().expect("open worker").expect("opened stream");
    let (woke, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buffer = [0_u8; 1];
        let result = stream.read(&mut buffer);
        let kind = match result {
            Ok(_) => io::ErrorKind::Other,
            Err(error) => error.kind(),
        };
        woke.send(kind).expect("reader result receiver");
    });
    harness.host.disconnect();
    assert_eq!(
        receiver.recv_timeout(WAIT).expect("carrier reader wake"),
        io::ErrorKind::BrokenPipe
    );
    reader.join().expect("reader worker");
}

#[test]
fn terminal_facts_are_recorded_before_blocking_reader_wakes() {
    let harness = Harness::new(&[("open", "operation")]);
    let facts_seen = Arc::new(AtomicBool::new(false));
    let facts_for_callback = facts_seen.clone();
    let pending = harness.open(
        "open",
        "operation",
        Arc::new(move |_| {
            facts_for_callback.store(true, Ordering::Release);
        }),
    );
    let open = next_host(&harness);
    wait_future(harness.peer.deliver(open.clone())).expect("peer open delivery");
    harness
        .peer_owner
        .send("open", &harness.opened(&open).1)
        .expect("peer opened");
    let opened = wait_future(harness.peer.next_message())
        .expect("peer opened result")
        .expect("peer opened message");
    wait_future(harness.host.deliver(opened)).expect("host opened delivery");
    let mut stream = pending.join().expect("open worker").expect("opened stream");

    let facts_for_reader = facts_seen.clone();
    let (woke, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buffer = [0_u8; 1];
        let result = stream.read(&mut buffer);
        let kind = match result {
            Ok(_) => io::ErrorKind::Other,
            Err(error) => error.kind(),
        };
        woke.send((facts_for_reader.load(Ordering::Acquire), kind))
            .expect("reader result receiver");
    });

    let facts = Facts {
        method: AuthMethod::SshAgent,
        credential_offered: true,
        authenticated: Some(false),
        ..Default::default()
    };
    let failure = harness.failed(&open, Some(facts.clone()));
    let failure_facts = failure.1.failed.as_ref().unwrap().facts.clone().unwrap();
    wait_future(harness.host.deliver(failure)).expect("host failed delivery");
    let (seen, kind) = receiver
        .recv_timeout(WAIT)
        .expect("reader wake after terminal failure");
    reader.join().expect("reader worker");
    assert!(seen, "terminal facts must precede reader wake");
    assert_eq!(kind, io::ErrorKind::PermissionDenied);
    assert_eq!(failure_facts, facts);
}

#[test]
fn cancellation_wakes_open_and_late_terminals_cannot_close_sibling_binding() {
    let harness = Harness::new(&[("first", "operation-1"), ("sibling", "operation-2")]);
    let pending = harness.open("first", "operation-1", Arc::new(|_| {}));
    let first_open = next_host(&harness);
    wait_future(harness.peer.deliver(first_open.clone())).expect("peer first open");
    harness.session.cancel("first");
    let cancel = next_host(&harness);
    assert_eq!(cancel.1.kind, MessageKind::Cancel);
    wait_future(harness.peer.deliver(cancel)).expect("peer cancellation");

    // The open result is the authoritative cancellation outcome.  This
    // bounded check catches a stranded Wait rather than allowing a later
    // endpoint terminal to win by timing.
    let deadline = Instant::now() + Duration::from_millis(250);
    while !pending.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    if !pending.is_finished() {
        harness.host.disconnect();
        let _ = pending.join();
        panic!("cancelling an active open did not wake its waiter");
    }
    let outcome = pending.join().expect("cancelled open worker");
    let error = match outcome {
        Ok(_) => panic!("cancelled open unexpectedly opened a stream"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(matches!(
        error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<gwz_transport::stream::Error>()),
        Some(gwz_transport::stream::Error::PeerFailed {
            code: gwz_transport::protocol::ErrorCode::Cancelled,
            ..
        })
    ));

    // Deliver stale endpoint terminals directly at the host port.  The mux
    // accepts the already-retired stream id as a tombstone and must not let
    // either terminal mutate the completed cancellation outcome.
    wait_future(harness.host.deliver(harness.open_failed(&first_open)))
        .expect("late failure admission");
    wait_future(harness.host.deliver(harness.late_closed(&first_open)))
        .expect("late closed admission");

    let sibling = harness.open("sibling", "operation-2", Arc::new(|_| {}));
    let sibling_open = next_host(&harness);
    wait_future(harness.peer.deliver(sibling_open.clone())).expect("peer sibling open");
    harness
        .peer_owner
        .send("sibling", &harness.opened(&sibling_open).1)
        .expect("peer sibling opened");
    let sibling_opened = wait_future(harness.peer.next_message())
        .expect("peer sibling opened result")
        .expect("peer sibling opened message");
    wait_future(harness.host.deliver(sibling_opened)).expect("host sibling opened");
    let _sibling_stream = sibling
        .join()
        .expect("sibling worker")
        .expect("sibling stream");
}

#[test]
fn oversized_open_deadlines_fail_through_bound_session_without_stopping_progress() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".ssh")).unwrap();
    std::fs::write(home.path().join(".ssh/known_hosts"), b"").unwrap();
    let (driver, host) = session::Session::driver(3000).unwrap();
    let (endpoint, client) =
        session::Session::endpoint(SshEndpointConfig::fixture(home.path().into(), None)).unwrap();
    driver
        .register("oversize", Some("operation".into()))
        .unwrap();
    endpoint.register("oversize", None).unwrap();
    driver.begin("oversize").unwrap();
    wait_future(client.deliver(wait_future(host.next_message()).unwrap().unwrap())).unwrap();
    wait_future(host.deliver(wait_future(client.next_message()).unwrap().unwrap())).unwrap();
    let worker = driver.clone();
    let (tx, rx) = mpsc::channel();
    let open = thread::spawn(move || {
        tx.send(worker.open(
            "oversize",
            "operation",
            "ssh://git@example.invalid/repo",
            GitService::UploadPack,
            Identity::default(),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
        ))
        .unwrap()
    });
    let mut attachment = wait_future(host.next_message()).unwrap().unwrap();
    let deadlines = &mut attachment.1.open.as_mut().unwrap().deadlines;
    deadlines.allocation_ms = i64::MAX;
    deadlines.interaction_ms = i64::MAX;
    deadlines.connect_ms = 1;
    gwz_transport::codec::admit(&attachment.1).unwrap();
    wait_future(client.deliver(attachment)).unwrap();
    let response = {
        let mut next = pin!(client.next_message());
        let mut cx = Context::from_waker(Waker::noop());
        let until = Instant::now() + Duration::from_millis(500);
        loop {
            if let Poll::Ready(result) = next.as_mut().poll(&mut cx) {
                break result.ok().flatten();
            }
            if Instant::now() >= until {
                break None;
            }
            thread::sleep(Duration::from_millis(1));
        }
    };
    if let Some(response) = &response {
        wait_future(host.deliver(response.clone())).unwrap();
    }
    // Always wake/join the caller in a red run, even if the endpoint supervisor died.
    if response.is_none() {
        host.disconnect();
    }
    let result = rx
        .recv_timeout(WAIT)
        .expect("blocking open waiter was stranded");
    open.join().unwrap();
    assert_eq!(
        response
            .expect("endpoint supervisor lost progress")
            .1
            .open_failed
            .unwrap()
            .code,
        gwz_transport::protocol::ErrorCode::InvalidRequest
    );
    assert!(result.is_err());
    // Another request is serviced on the same binding after policy rejection.
    driver.register("after", Some("operation".into())).unwrap();
    endpoint.register("after", None).unwrap();
    let worker = driver.clone();
    let check = thread::spawn(move || {
        worker.check(
            "after",
            Identity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some("/missing-after-policy-rejection".into()),
                path_base: None,
            },
        )
    });
    wait_future(client.deliver(wait_future(host.next_message()).unwrap().unwrap())).unwrap();
    wait_future(host.deliver(wait_future(client.next_message()).unwrap().unwrap())).unwrap();
    assert!(check.join().unwrap().is_err());
    host.disconnect();
    client.disconnect();
}
