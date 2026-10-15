//! The eight request rounds of a native (Windows sign-in) exchange are the operation's, not one attempt's, and a
//! retry that the endpoint's own retry machine makes after an exchange resumes the count (design 8; D19 Code P3-2
//! and State follow-up 1). The attempts are the endpoint's, not a `Budget` handed from one to the next by the test.
use super::{
    requeue_tests::{collect, replies},
    retry_tests::{code, endpoint, open, port, runtime, shut},
    *,
};
use crate::git::endpoint::https_fixture as fixture;
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn d19_code_p3_2_a_retry_after_an_exchange_resumes_the_eight_rounds_of_the_operation() {
    runtime().block_on(async {
        // The server challenges NTLM and then continues the exchange for as long as the client answers, except
        // that its fourth authenticated answer is a 429: the first attempt ends after four rounds with a throttle,
        // which the endpoint requeues, and the retry meets the same server.
        let authenticated = Arc::new(AtomicUsize::new(0));
        let seen = authenticated.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let number = if request.headers().contains_key(AUTHORIZATION) {
                seen.fetch_add(1, Ordering::SeqCst) + 1
            } else {
                0
            };
            Box::pin(async move {
                let service = GitService::UploadPackAdvertisement;
                let status = if number == 4 { 429 } else { 401 };
                let mut result = fixture::response(status, service, "");
                if status == 401 {
                    let offer = if number == 0 { "NTLM" } else { "NTLM YQ==" };
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                }
                result
            })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        let (caller, steps) =
            crate::git::endpoint::https_worker::native::counting_fixture("127.0.0.1");
        endpoint.client.set_native(caller);
        let mut envelope = open(1, port, 30_000);
        envelope.open.as_mut().unwrap().policy = AuthPolicy::WindowsDefault;
        endpoint.accept("request".into(), envelope).unwrap();
        let mut published = Vec::new();
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| !replies(p).is_empty(),
        )
        .await;
        // The retry ran: the server saw more than the first attempt's four answers...
        assert!(
            authenticated.load(Ordering::SeqCst) > 4,
            "the throttled attempt was not retried"
        );
        // ...and the rounds spent by the two attempts together are the operation's eight.
        assert_eq!(
            steps.load(Ordering::SeqCst),
            8,
            "a retry of the open got rounds beyond the operation's eight"
        );
        assert_eq!(code(replies(&published)[0]), ErrorCode::Protocol);
        shut(&mut endpoint).await;
    });
}
