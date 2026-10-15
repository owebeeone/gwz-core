//! How an SSPI exchange with the logon session ends (TR1.8 sections 7 and 8, TD1): a server's rejection is terminal
//! with no replay, a redirect ends the exchange and sends nothing to its new location, and the eight rounds are the
//! operation's, not one exchange's. The SSPI side is the fake port, so the rows run on every platform.
use super::*;
use crate::git::endpoint::https_worker::native::exchange::channel_binding;

/// A server that answers 401 and offers `first` to a request with no `Authorization` header and `after` to one that
/// has it, and counts the requests.
async fn server(
    first: &'static str,
    after: &'static str,
) -> (
    crate::git::endpoint::https_fixture::Server,
    Arc<AtomicUsize>,
) {
    use crate::git::endpoint::https_fixture::{Server, response};
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let server = Server::start(Arc::new(move |request| {
        seen.fetch_add(1, Ordering::Relaxed);
        let offer = if request.headers().contains_key(AUTHORIZATION) {
            after
        } else {
            first
        };
        Box::pin(async move {
            let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
            result
                .headers_mut()
                .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
            result
        })
    }))
    .await;
    (server, requests)
}

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

/// One attempt at an open under the logon-session policy, on `budget`.
async fn attempt(
    endpoint: &Endpoint,
    server: &crate::git::endpoint::https_fixture::Server,
    budget: &mut Budget,
) -> Result<Prepared, Failure> {
    use crate::git::endpoint::https_fixture::input;
    let mut request = input(server, GitService::UploadPackAdvertisement);
    request.policy = AuthPolicy::WindowsDefault;
    endpoint
        .client
        .prepare_attempt(request, &CancellationToken::new(), budget, &mut None)
        .await
        .0
}

async fn finish(mut endpoint: Endpoint) {
    endpoint.client.finish_operation("operation");
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
}

#[test]
fn a_401_with_no_continuation_token_is_the_servers_rejection_and_ends_the_exchange() {
    run(async {
        // After the client's token the server answers 401 and offers the scheme again with no token: the logon
        // identity is refused. The exchange does not go on to a second round and nothing replays the identity.
        for offer in ["NTLM", "Negotiate"] {
            let (server, requests) = server(offer, offer).await;
            let mut endpoint =
                Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, port) = fake(false, 2);
            endpoint.client.set_native(caller);
            let result = attempt(&endpoint, &server, &mut endpoint.client.budget()).await;
            let failure = result.as_ref().err().unwrap();
            assert_eq!(failure.code, ErrorCode::Authentication, "{offer}");
            let facts = failure.facts.as_ref().unwrap();
            assert_eq!(facts.authenticated, Some(false), "{offer}");
            assert_eq!(facts.http_status, Some(401), "{offer}");
            assert_eq!(port.steps.load(Ordering::Acquire), 1, "{offer}");
            assert_eq!(port.starts.load(Ordering::Acquire), 1, "{offer}");
            assert_eq!(
                requests.load(Ordering::Acquire),
                2,
                "a challenge and one answer: {offer}"
            );
            drop(result);
            finish(endpoint).await;
        }
    });
}

#[test]
fn a_redirect_during_the_exchange_ends_it_and_nothing_reaches_the_new_location() {
    run(async {
        let (target, arrivals) = server("NTLM", "NTLM").await;
        // The first server challenges, then answers the client's token with a redirect to the other.
        let location = format!("{}/info/refs", target.url);
        use crate::git::endpoint::https_fixture::{Server, response};
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let source = Server::start(Arc::new(move |request| {
            seen.fetch_add(1, Ordering::Relaxed);
            let location = location.clone();
            let answered = request.headers().contains_key(AUTHORIZATION);
            Box::pin(async move {
                let mut result = response(
                    if answered { 302 } else { 401 },
                    GitService::UploadPackAdvertisement,
                    Bytes::new(),
                );
                if answered {
                    result
                        .headers_mut()
                        .insert(hyper::header::LOCATION, location.parse().unwrap());
                } else {
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
                }
                result
            })
        }))
        .await;
        let mut endpoint = Endpoint::new(source.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(false, 2);
        endpoint.client.set_native(caller);
        let result = attempt(&endpoint, &source, &mut endpoint.client.budget()).await;
        let failure = result.as_ref().err().unwrap();
        assert_eq!(failure.code, ErrorCode::Authentication);
        assert_eq!(failure.facts.as_ref().unwrap().http_status, Some(302));
        assert_eq!(port.steps.load(Ordering::Acquire), 1);
        assert_eq!(requests.load(Ordering::Acquire), 2);
        assert_eq!(
            arrivals.load(Ordering::Acquire),
            0,
            "the new location saw a request"
        );
        drop(result);
        finish(endpoint).await;
    });
}

#[test]
fn the_eight_rounds_are_the_operations_and_a_retry_does_not_get_eight_more() {
    run(async {
        // The server always continues the exchange, so an exchange runs until the cap. The client's cap is on the
        // budget it is given: a second attempt on the first one's budget has no rounds left, starts no SSPI work,
        // and does not reset the count. That the endpoint's own retry machine hands the second attempt the first
        // one's count is pinned apart, through the endpoint (`d19_code_p3_2_*` in `https_endpoint`).
        let (server, requests) = server("NTLM", "NTLM YQ==").await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(false, 2);
        endpoint.client.set_native(caller);
        let mut budget = endpoint.client.budget();
        let first = attempt(&endpoint, &server, &mut budget).await;
        assert_eq!(first.err().unwrap().code, ErrorCode::Protocol);
        assert_eq!(port.steps.load(Ordering::Acquire), 8);
        assert_eq!(port.starts.load(Ordering::Acquire), 1);
        let seen = requests.load(Ordering::Acquire);
        let second = attempt(&endpoint, &server, &mut budget).await;
        assert_eq!(second.err().unwrap().code, ErrorCode::Protocol);
        assert_eq!(
            port.steps.load(Ordering::Acquire),
            8,
            "rounds beyond the eighth"
        );
        assert_eq!(
            port.starts.load(Ordering::Acquire),
            1,
            "a second exchange started"
        );
        // The retry's discovery request is the one the challenge answers; no authenticated request follows it.
        assert!(requests.load(Ordering::Acquire) <= seen + 1);
        finish(endpoint).await;
        // A new operation has its own eight.
        let (server, _) = self::server("NTLM", "NTLM YQ==").await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let (caller, port) = fake(false, 2);
        endpoint.client.set_native(caller);
        let fresh = attempt(&endpoint, &server, &mut endpoint.client.budget()).await;
        assert_eq!(fresh.err().unwrap().code, ErrorCode::Protocol);
        assert_eq!(port.steps.load(Ordering::Acquire), 8);
        finish(endpoint).await;
    });
}

#[test]
fn a_connection_with_no_channel_binding_cannot_start_an_sspi_attempt() {
    // None, and an empty one, are refused; the binding of a connection is passed on as it is.
    assert!(channel_binding(None).is_none());
    assert!(channel_binding(Some(&gwz_sspi::SecretBytes::new(b""))).is_none());
    let mut digest = vec![0x41; 32];
    let held = https_auth::SecretHeader::channel_binding_digest(&mut digest).unwrap();
    let passed = channel_binding(Some(&held)).unwrap();
    assert_eq!(passed.as_bytes(), held.as_bytes());
    assert!(passed.as_bytes().starts_with(b"tls-server-end-point:"));
}
