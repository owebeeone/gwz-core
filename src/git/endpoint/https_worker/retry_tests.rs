//! What one HTTPS attempt tells its open's retry machine (gwz-core
//! dev-docs/GwzRemoteTransportRetryPlan.md §4): whether its first connect,
//! the open's setup, succeeded or failed, and a failure's origin.
use super::*;
use crate::git::endpoint::https_fixture::{Server, attach, response};
use std::net::TcpListener;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn advertisement(url: &str) -> Input {
    Input {
        destination: url.into(),
        service: GitService::UploadPackAdvertisement,
        policy: AuthPolicy::Anonymous,
        session: "session".into(),
        operation: "operation".into(),
    }
}
/// One attempt of an advertisement Open, with the endpoint's own budget.
async fn attempt(endpoint: &Endpoint, url: &str) -> (Result<Prepared, Failure>, FirstConnect) {
    let mut budget = endpoint.client.budget();
    endpoint
        .client
        .prepare_attempt(
            advertisement(url),
            &CancellationToken::new(),
            &mut budget,
            &mut None,
        )
        .await
}
/// Serves a prepared advertisement to its end, and returns how its
/// connection went back to the pool.
async fn serve(prepared: Prepared) -> Disposition {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut buffer = [0; 64];
    while stream.read(&mut buffer).await.unwrap() != 0 {}
    let disposition = stream.close().await.unwrap().disposition;
    task.await.unwrap();
    disposition
}
/// A port nothing listens on.
fn refused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}
/// Accepts each connection and, with `close`, closes it at once, before
/// any TLS; otherwise holds it unanswered. Counts what it accepted.
fn listener(close: bool) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            counter.fetch_add(1, Ordering::AcqRel);
            if !close {
                held.push(stream);
            }
        }
    });
    (port, accepted)
}

#[test]
fn a_dropped_tls_handshake_is_io_and_an_untrusted_certificate_is_trust() {
    runtime().block_on(async {
        let mut endpoint = Endpoint::new(
            https_connection::Config::default(),
            None,
            pool::Config::default(),
        )
        .unwrap();
        // A server that drops a connection before its handshake, as one with
        // no room for another does: Io, which a setup retries.
        let (port, accepted) = listener(true);
        let (result, connect) = attempt(&endpoint, &format!("https://127.0.0.1:{port}/repo")).await;
        let failure = result.err().expect("a dropped handshake fails");
        assert_eq!(
            (failure.code, connect),
            (ErrorCode::Io, FirstConnect::Failed)
        );
        assert_eq!(accepted.load(Ordering::Acquire), 1);
        // A certificate the endpoint does not trust: Trust, which closes the key.
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let (result, connect) = attempt(&endpoint, &server.url).await;
        let failure = result.err().expect("an untrusted certificate fails");
        assert_eq!(
            (failure.code, connect),
            (ErrorCode::Trust, FirstConnect::Failed)
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn only_the_first_hops_fresh_connect_is_the_opens_setup_and_keeps_its_origin() {
    runtime().block_on(async {
        let refused = refused_port();
        let server = Server::start(Arc::new(move |request| {
            Box::pin(async move {
                if request.uri().path() == "/redirect/info/refs" {
                    let mut reply = response(302, GitService::UploadPackAdvertisement, "");
                    let location = format!(
                        "https://127.0.0.1:{refused}/moved/info/refs?service=git-upload-pack"
                    );
                    reply
                        .headers_mut()
                        .insert("Location", location.parse().unwrap());
                    reply
                } else {
                    response(200, GitService::UploadPackAdvertisement, "ok")
                }
            })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        // A fresh connect that succeeds is the open's setup.
        let (result, connect) = attempt(&endpoint, &server.url).await;
        assert_eq!(connect, FirstConnect::Connected);
        assert_eq!(serve(result.unwrap()).await, Disposition::Reusable);
        // An idle connection is no setup.
        let (result, connect) = attempt(&endpoint, &server.url).await;
        let prepared = result.unwrap();
        assert!(prepared.opened.reused);
        assert_eq!(connect, FirstConnect::None);
        serve(prepared).await;
        // A refused first connect is the setup's failure, with its origin.
        let (result, connect) =
            attempt(&endpoint, &format!("https://127.0.0.1:{refused}/repo")).await;
        let failure = result.err().expect("a refused connect fails");
        assert_eq!(
            (failure.code, failure.setup_cause, connect),
            (
                ErrorCode::Unavailable,
                Some(SetupFailureCause::ConnectionRefused),
                FirstConnect::Failed
            )
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        // A redirect's connect comes after the open's first request byte: the
        // open's setup succeeded, and the hop's failure keeps no origin that
        // a retry could be decided by.
        let mut fresh = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let redirect = server.url.replace("/repo", "/redirect");
        let (result, connect) = attempt(&fresh, &redirect).await;
        let failure = result.err().expect("the redirect's refused connect fails");
        assert_eq!(
            (failure.code, failure.setup_cause, connect),
            (ErrorCode::Unavailable, None, FirstConnect::Connected)
        );
        assert_eq!(fresh.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_stalled_handshake_is_an_aggregate_timeout_and_a_full_pool_an_allocation_one() {
    runtime().block_on(async {
        // A handshake that never answers runs out of the connect's
        // aggregate budget: a setup timeout whose origin is aggregate.
        let (port, accepted) = listener(false);
        let config = pool::Config {
            connect_timeout_ms: 200,
            ..Default::default()
        };
        let mut stalled = Endpoint::new(https_connection::Config::default(), None, config).unwrap();
        let (result, connect) = attempt(&stalled, &format!("https://127.0.0.1:{port}/repo")).await;
        let failure = result.err().expect("a stalled handshake fails");
        assert_eq!(
            (failure.code, failure.setup_cause, connect),
            (
                ErrorCode::Timeout,
                Some(SetupFailureCause::Aggregate),
                FirstConnect::Failed
            )
        );
        assert_eq!(accepted.load(Ordering::Acquire), 1);
        assert_eq!(stalled.shutdown(Duration::from_secs(2)).await, 0);
        // A first hop that waits out its allocation for a full pool never set
        // up: no setup, and its origin is allocation.
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let config = pool::Config {
            total: 1,
            per_host: 1,
            per_user_host: 1,
            allocation_timeout_ms: 50,
            ..Default::default()
        };
        let mut narrow = Endpoint::new(server.config(), None, config).unwrap();
        let (held, _) = attempt(&narrow, &server.url).await;
        let held = held.unwrap();
        let (result, connect) = attempt(&narrow, &server.url).await;
        let failure = result.err().expect("a full pool's wait runs out");
        assert_eq!(
            (failure.code, failure.setup_cause, connect),
            (
                ErrorCode::Timeout,
                Some(SetupFailureCause::Allocation),
                FirstConnect::None
            )
        );
        drop(held);
        assert_eq!(narrow.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_helper_that_runs_out_of_the_interaction_allowance_has_that_origin() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("gh");
        crate::git::endpoint::helper_script::write_helper_script(&executable, "sleep 5\n");
        let config = pool::Config {
            interaction_timeout_ms: 100,
            ..Default::default()
        };
        let auth = https_auth::Config {
            executable,
            environment: Vec::new(),
        };
        let mut endpoint =
            Endpoint::new(https_connection::Config::default(), Some(auth), config).unwrap();
        let mut input = advertisement(&format!("https://127.0.0.1:{}/repo", refused_port()));
        input.policy = AuthPolicy::Gh;
        let mut budget = endpoint.client.budget();
        let (result, connect) = endpoint
            .client
            .prepare_attempt(input, &CancellationToken::new(), &mut budget, &mut None)
            .await;
        let failure = result.err().expect("the helper runs out of time");
        assert_eq!(
            (failure.code, failure.setup_cause, connect),
            (
                ErrorCode::Timeout,
                Some(SetupFailureCause::Interaction),
                FirstConnect::None
            )
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
    });
}

#[test]
fn an_exchange_whose_setup_its_key_does_not_admit_is_discarded_after_use() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (result, connect) = attempt(&endpoint, &server.url).await;
        assert_eq!(connect, FirstConnect::Connected);
        let mut prepared = result.unwrap();
        prepared.discard_after_use();
        assert_eq!(serve(prepared).await, Disposition::Discarded);
        // So the next open sets up afresh, and keeps that connection.
        let (result, connect) = attempt(&endpoint, &server.url).await;
        assert_eq!(connect, FirstConnect::Connected);
        assert_eq!(serve(result.unwrap()).await, Disposition::Reusable);
        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
