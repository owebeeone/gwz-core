//! The HTTPS endpoint's setup retries through its message boundary (gwz-core
//! dev-docs/GwzRemoteTransportRetryPlan.md §4 and §5, with amendment 2's
//! §3.20 cold start). Its clock is stepped by hand and its jitter is fixed,
//! and local listeners count the connections each key opens.
use super::*;
use crate::git::endpoint::{https_connection, https_fixture as fixture, setup_retry::Jitter};
use std::{
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
    task::Waker,
};

#[test]
fn retry_allowance_and_challenge_keys_are_account_specific() {
    let mut envelope = open(1, 443, 1_000);
    let first = envelope.open.as_mut().unwrap();
    first.destination.https_username = Some("account-a".into());
    let a = retry_key(first);
    first.destination.https_username = Some("account-b".into());
    assert_ne!(a, retry_key(first));
    first.destination.https_username = None;
    assert_ne!(a, retry_key(first));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// An endpoint whose pool allows `per_host` connections to a host and whose
/// waits draw no jitter. Its operation "request" retries `max_retries` times.
fn endpoint(tls: https_connection::Config, per_host: usize, max_retries: u32) -> HttpsEndpoint {
    let config = pool::Config {
        per_host,
        per_user_host: per_host,
        ..Default::default()
    };
    let authority = Authority::new(config.total, config.per_host);
    let mut endpoint = HttpsEndpoint::new(
        HttpsEndpointConfig { tls, auth: None },
        config,
        3_000,
        authority,
        "endpoint".into(),
        HelperSlots::new(),
    )
    .unwrap();
    endpoint.retries.set_jitter(Jitter::fixed(0));
    endpoint.set_max_retries("request", max_retries);
    endpoint
}

/// Stream `stream`'s anonymous advertisement Open of 127.0.0.1:`port`.
fn open(stream: i64, port: u16, allocation_ms: i64) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: stream,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: Scheme::Https,
                host: "127.0.0.1".into(),
                port: port as i64,
                path: "/repo".into(),
                ssh_username: None,
                https_username: None,
            },
            service: GitService::UploadPackAdvertisement,
            identity: Identity {
                mode: IdentityMode::CredentialsDisabled,
                ..Default::default()
            },
            policy: AuthPolicy::Anonymous,
            deadlines: Deadlines {
                allocation_ms,
                connect_ms: 10_000,
                io_ms: 3_000,
                interaction_ms: 120_000,
                cleanup_ms: 5_000,
            },
            receive_limits: gwz_transport::binding::default_limits(),
        }),
        ..Default::default()
    }
}

fn port(server: &fixture::Server) -> u16 {
    crate::git::endpoint::https_destination::Destination::parse(&server.url)
        .unwrap()
        .port()
}

/// Accepts each connection and closes it at once, before TLS, so that each
/// setup fails with `Io`; counts the connections.
fn closing() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            counter.fetch_add(1, Ordering::AcqRel);
            drop(stream);
        }
    });
    (port, accepted)
}

/// Steps the endpoint at `now` until no attempt is in flight, and returns
/// the messages it published meanwhile.
async fn settle(endpoint: &mut HttpsEndpoint, now: u64) -> Vec<Envelope> {
    let mut cx = Context::from_waker(Waker::noop());
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut published = Vec::new();
    loop {
        endpoint.step(now, &mut cx).unwrap();
        while let Some(outbound) = endpoint.take_outbound(&mut cx) {
            published.push(outbound.envelope);
        }
        if endpoint
            .entries
            .values()
            .all(|entry| entry.preparing.is_none())
        {
            return published;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "an attempt never ended"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

fn code(message: &Envelope) -> ErrorCode {
    assert_eq!(message.kind, MessageKind::OpenFailed);
    message.open_failed.as_ref().expect("a failure").code
}

async fn shut(endpoint: &mut HttpsEndpoint) {
    endpoint.shutdown();
    let mut cx = Context::from_waker(Waker::noop());
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while endpoint.pending() != 0 {
        endpoint.step(endpoint.now_ms, &mut cx).unwrap();
        while endpoint.take_outbound(&mut cx).is_some() {}
        assert!(
            tokio::time::Instant::now() < until,
            "cleanup retains a live owner"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[test]
fn a_dead_keys_first_wave_is_its_per_host_limit_and_then_one_probe_at_a_time() {
    runtime().block_on(async {
        let (port, accepted) = closing();
        let mut endpoint = endpoint(https_connection::Config::default(), 2, 2);
        // Each open's allocation outlasts a wait only because a wait for the
        // key stops that clock.
        for stream in 1..=6 {
            endpoint
                .accept("request".into(), open(stream, port, 900))
                .unwrap();
        }
        // Cold: the first wave is the per-host limit, and no open finishes on
        // a retriable failure.
        assert!(settle(&mut endpoint, 0).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 2);
        // Nothing connects before the wake, 1 s after attempt 1.
        assert!(settle(&mut endpoint, 999).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 2);
        // One probe at the wake, attempt 2, and then a 2 s wait.
        assert!(settle(&mut endpoint, 1_000).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 3);
        assert!(settle(&mut endpoint, 2_999).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 3);
        // Attempt 3 is R + 1: its failure finishes every open on the key.
        let finished = settle(&mut endpoint, 3_000).await;
        assert_eq!(accepted.load(Ordering::Acquire), 4);
        assert_eq!(finished.len(), 6);
        for message in &finished {
            let count = message
                .open_failed
                .as_ref()
                .unwrap()
                .detail
                .as_ref()
                .and_then(|detail| detail.retry_attempt.as_ref())
                .unwrap();
            assert_eq!((count.attempt, count.attempts), (3, 3));
        }
        assert!(
            finished
                .iter()
                .all(|message| code(message) == ErrorCode::Io)
        );
        shut(&mut endpoint).await;
    });
}

#[test]
fn an_untrusted_certificate_closes_its_key_for_the_rest_of_the_operation() {
    runtime().block_on(async {
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async { fixture::response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(https_connection::Config::default(), 8, 3);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let first = settle(&mut endpoint, 0).await;
        assert_eq!(first.len(), 1);
        assert_eq!(code(&first[0]), ErrorCode::Trust);
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        // The operation's next open on that key finishes with it at once.
        endpoint
            .accept("request".into(), open(2, port, 30_000))
            .unwrap();
        let next = settle(&mut endpoint, 0).await;
        assert_eq!(next.len(), 1);
        assert_eq!(code(&next[0]), ErrorCode::Trust);
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        // The next operation starts the key Cold.
        endpoint
            .accept("other".into(), open(1, port, 30_000))
            .unwrap();
        assert_eq!(code(&settle(&mut endpoint, 0).await[0]), ErrorCode::Trust);
        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_failure_after_the_first_request_byte_is_returned_once() {
    runtime().block_on(async {
        let refused = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = fixture::Server::start(Arc::new(move |request| {
            Box::pin(async move {
                let service = GitService::UploadPackAdvertisement;
                match request.uri().path() {
                    "/server-error/info/refs" => fixture::response(500, service, ""),
                    "/denied/info/refs" => fixture::response(401, service, ""),
                    _ => {
                        let mut reply = fixture::response(302, service, "");
                        let location = format!(
                            "https://127.0.0.1:{refused}/moved/info/refs?service=git-upload-pack"
                        );
                        reply
                            .headers_mut()
                            .insert("Location", location.parse().unwrap());
                        reply
                    }
                }
            })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        // A server error and an HTTP authentication failure come after the
        // request's first byte, and so does a redirect's refused connect:
        // each is returned at once, with no wait and no second attempt.
        for (stream, path, expected) in [
            (1, "/server-error", ErrorCode::Io),
            (2, "/denied", ErrorCode::Authentication),
            (3, "/redirect", ErrorCode::Unavailable),
        ] {
            let mut message = open(stream, port, 30_000);
            message.open.as_mut().unwrap().destination.path = path.into();
            endpoint.accept("request".into(), message).unwrap();
            let published = settle(&mut endpoint, 0).await;
            assert_eq!(published.len(), 1, "{path}");
            assert_eq!(code(&published[0]), expected, "{path}");
            assert_eq!(server.connections.load(Ordering::SeqCst), stream as usize);
        }
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_cancelled_open_its_key_holds_starts_no_attempt() {
    runtime().block_on(async {
        let (port, accepted) = closing();
        let mut endpoint = endpoint(https_connection::Config::default(), 8, 1);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        assert!(settle(&mut endpoint, 0).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 1);
        let cancel = Envelope {
            version: 2,
            session_id: "session".into(),
            stream_id: 1,
            kind: MessageKind::Cancel,
            ..Default::default()
        };
        endpoint.accept("request".into(), cancel).unwrap();
        let cancelled = settle(&mut endpoint, 0).await;
        assert_eq!(cancelled.len(), 1);
        assert_eq!(code(&cancelled[0]), ErrorCode::Cancelled);
        // Past the wake, nothing connects for it.
        assert!(settle(&mut endpoint, 5_000).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 1);
        shut(&mut endpoint).await;
    });
}

#[test]
fn an_open_the_per_host_limit_holds_runs_out_of_allocation() {
    runtime().block_on(async {
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                fixture::response(200, GitService::UploadPackAdvertisement, "ok")
            })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 1, 3);
        endpoint
            .accept("request".into(), open(1, port, 100))
            .unwrap();
        endpoint
            .accept("request".into(), open(2, port, 100))
            .unwrap();
        // The first open's attempt holds the host's one place; the second
        // waits for it, and its allocation clock runs meanwhile.
        let mut cx = Context::from_waker(Waker::noop());
        endpoint.step(200, &mut cx).unwrap();
        let held = endpoint
            .take_outbound(&mut cx)
            .expect("the held open's allocation ran out");
        assert_eq!(held.envelope.stream_id, 2);
        let failure = held.envelope.open_failed.expect("a failure");
        assert_eq!(
            (failure.code, failure.setup_cause),
            (ErrorCode::Timeout, Some(SetupFailureCause::Allocation))
        );
        shut(&mut endpoint).await;
    });
}
