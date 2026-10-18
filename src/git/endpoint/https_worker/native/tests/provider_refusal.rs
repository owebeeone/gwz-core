//! A sign-in that the Windows provider refuses (the first `InitializeSecurityContextW` failing, or a token larger than
//! the limit) is told from a worker that broke its contract by the failure's facts (decision D48): the code stays
//! `Protocol`, so nothing else that reads it changes. The SSPI side is a fake port, so the rows run on every platform.
use super::*;

struct RefusingPort {
    provider_refused: bool,
    steps: Arc<AtomicUsize>,
}
struct RefusingSession {
    provider_refused: bool,
    steps: Arc<AtomicUsize>,
}
impl Port for RefusingPort {
    fn start(
        &self,
        _: gwz_sspi::AuthRequest,
        _: Instant,
        _: gwz_sspi::Cancellation,
    ) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
        let session = RefusingSession {
            provider_refused: self.provider_refused,
            steps: self.steps.clone(),
        };
        Box::pin(async move { Ok(Box::new(session) as Box<dyn Session>) })
    }
}
impl Session for RefusingSession {
    fn step(
        &mut self,
        _: Option<gwz_sspi::SecretBytes>,
    ) -> Work<'_, Result<gwz_sspi::TokenStep, BridgeError>> {
        self.steps.fetch_add(1, Ordering::Relaxed);
        let provider_refused = self.provider_refused;
        Box::pin(async move {
            Err(BridgeError {
                code: ErrorCode::Protocol,
                pending: None,
                provider_refused,
            })
        })
    }
    fn finish(self: Box<Self>) -> Work<'static, Result<(), BridgeError>> {
        Box::pin(async { Ok(()) })
    }
    fn cancel(self: Box<Self>) -> Option<Box<dyn Probe>> {
        None
    }
}

/// The failure of one open against a server that challenges with `offer`, whose first step the port fails.
fn refused_open(offer: &'static str, provider_refused: bool) -> Failure {
    let mut failure = None;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let server = Server::start(Arc::new(move |_| {
                Box::pin(async move {
                    let mut result =
                        response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                    result
                })
            }))
            .await;
            let mut endpoint =
                Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let steps = Arc::new(AtomicUsize::new(0));
            let port = Arc::new(RefusingPort {
                provider_refused,
                steps: steps.clone(),
            });
            endpoint.client.set_native(NativeCaller {
                port: Ok(port),
                machine_proxy: None,
            });
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::WindowsDefault;
            let (prepared, _) = endpoint
                .client
                .prepare_attempt(
                    request,
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            assert_eq!(steps.load(Ordering::Acquire), 1, "{offer}");
            failure = Some(prepared.err().expect("the sign-in is refused"));
            endpoint.client.finish_operation("operation");
            let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
    failure.unwrap()
}

#[test]
fn a_negotiate_sign_in_the_provider_refused_says_so_in_its_facts_and_keeps_its_code() {
    use gwz_transport::protocol::NativeObservation;
    let failure = refused_open("Negotiate", true);
    assert_eq!(failure.code, ErrorCode::Protocol);
    let facts = failure.facts.as_ref().unwrap();
    // The sign-in began and no mechanism was ever seen; nothing was sent.
    let native = facts.native.as_ref().unwrap();
    assert_eq!(native.observation, NativeObservation::Unresolved);
    assert_eq!(native.mechanism, None);
    assert!(!native.authoritative);
    assert!(!facts.credential_offered);
    assert_eq!(facts.http_status, None);
    assert_eq!(facts.authenticated, None);
}

#[test]
fn a_worker_that_broke_its_contract_leaves_the_facts_as_they_were() {
    use gwz_transport::protocol::NativeObservation;
    let failure = refused_open("Negotiate", false);
    assert_eq!(failure.code, ErrorCode::Protocol);
    let native = failure.facts.as_ref().unwrap().native.as_ref().unwrap();
    assert_eq!(native.observation, NativeObservation::NotStarted);
}

#[test]
fn an_ntlm_refusal_cannot_be_told_in_the_facts_and_leaves_them_as_they_were() {
    // The wire accepts Unresolved for Negotiate only (an NTLM exchange has its mechanism from the start).
    use gwz_transport::protocol::NativeObservation;
    let failure = refused_open("NTLM", true);
    assert_eq!(failure.code, ErrorCode::Protocol);
    let native = failure.facts.as_ref().unwrap().native.as_ref().unwrap();
    assert_eq!(native.observation, NativeObservation::NotStarted);
}
