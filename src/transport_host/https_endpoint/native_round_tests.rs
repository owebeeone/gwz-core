//! The eight request rounds of a native (Windows sign-in) exchange are the operation's, not one attempt's, and a
//! retry that the endpoint's own retry machine makes after an exchange resumes the count (design 8; D19 Code P3-2
//! and State follow-up 1). The attempts are the endpoint's, not a `Budget` handed from one to the next by the test.
use super::{
    requeue_tests::{collect, replies},
    retry_tests::{code, endpoint, endpoint_with, open, port, runtime, settle, shut},
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

/// What an open under `policy` ended with against a server that answers every authenticated request with a 429, and
/// how many authenticated requests and logon-session steps it took, and the port of its server. A native exchange here is two rounds: the
/// server answers the first token with a continuation and the second with the 429. `max_retries` is the request's.
async fn throttled_to_the_end(
    policy: AuthPolicy,
    max_retries: u32,
) -> (Failure, usize, usize, u16) {
    let authenticated = Arc::new(AtomicUsize::new(0));
    let seen = authenticated.clone();
    let native = policy != AuthPolicy::Gh;
    let server = fixture::Server::start(Arc::new(move |request| {
        let number = if request.headers().contains_key(AUTHORIZATION) {
            seen.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            0
        };
        Box::pin(async move {
            let service = GitService::UploadPackAdvertisement;
            // Odd authenticated requests of a native exchange are its first token: the server continues it.
            let status = if number == 0 || (native && number % 2 == 1) {
                401
            } else {
                429
            };
            let mut result = fixture::response(status, service, "");
            if status == 401 {
                let offer = match (native, number) {
                    (false, _) => "Basic realm=x",
                    (true, 0) => "NTLM",
                    (true, _) => "NTLM YQ==",
                };
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
            }
            result
        })
    }))
    .await;
    let port = port(&server);
    let dir = tempfile::tempdir().unwrap();
    let helper = (!native).then(|| {
        crate::git::endpoint::helper_script::git_fixture(
            dir.path(),
            "cat >/dev/null\nprintf 'username=fixture\\npassword=fixture\\n\\n'\n",
        )
    });
    let mut endpoint = endpoint_with(server.config(), helper, 8, max_retries);
    let (caller, steps) = crate::git::endpoint::https_worker::native::counting_fixture("127.0.0.1");
    endpoint.client.set_native(caller);
    let mut envelope = open(1, port, 30_000);
    let request = envelope.open.as_mut().unwrap();
    request.policy = policy;
    request.identity.mode = IdentityMode::Ambient;
    endpoint.accept("request".into(), envelope).unwrap();
    let mut published = Vec::new();
    collect(
        &mut endpoint,
        Duration::from_secs(20),
        &mut published,
        |p| !replies(p).is_empty(),
    )
    .await;
    let reply = replies(&published)[0];
    assert_eq!(reply.kind, MessageKind::OpenFailed, "{policy:?}");
    let failure = reply.open_failed.clone().unwrap();
    let (taken, rounds) = (
        authenticated.load(Ordering::SeqCst),
        steps.load(Ordering::SeqCst),
    );
    shut(&mut endpoint).await;
    (failure, taken, rounds, port)
}

/// The words a failure is shown in, without the count of the attempts that led to it and without the server's port.
fn words_without_attempts(failure: &Failure, port: u16) -> String {
    let mut failure = failure.clone();
    if let Some(detail) = failure.detail.as_mut() {
        detail.retry_attempt = None;
    }
    super::capacity_tests::user_error(&failure, None)
        .to_string()
        .replace(&format!("127.0.0.1:{port}"), "the server")
}

#[test]
fn d19_code_p3_3_an_open_whose_native_rounds_are_spent_ends_with_the_throttle_not_protocol() {
    runtime().block_on(async {
        // `--max-retries 7` allows eight attempts, and an exchange of two rounds spends the operation's eight in
        // four: the fifth attempt would meet the up-front refusal and end `Protocol`, naming nothing of the throttle.
        let (gh, _, _, gh_port) = throttled_to_the_end(AuthPolicy::Gh, 7).await;
        assert_eq!(gh.code, ErrorCode::Capacity, "the reference outcome");
        let (windows, authenticated, steps, port) =
            throttled_to_the_end(AuthPolicy::WindowsDefault, 7).await;
        assert!(steps <= 8, "{steps} steps beyond the operation's eight");
        assert_eq!(authenticated, steps, "every step was sent");
        assert_ne!(windows.code, ErrorCode::Protocol, "{windows:?}");
        assert_eq!(windows.code, gh.code);
        assert_eq!(
            words_without_attempts(&windows, port),
            words_without_attempts(&gh, gh_port)
        );
    });
}

#[test]
fn d19_state_p3_6_a_requeued_attempt_after_an_exchange_has_a_deadline_of_its_own() {
    runtime().block_on(async {
        // The first attempt's exchange ends in a 429 that asks for 5 s; the endpoint's clock is stepped by hand, so
        // the retry starts exactly when the clock is stepped to the end of that wait, as a new exchange under a D
        // anchored there (the parity design's amended section 9): not the first attempt's D, spent or not.
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
                let status = if number == 2 { 429 } else { 401 };
                let mut result = fixture::response(status, service, "");
                if status == 401 {
                    let offer = if number == 0 { "NTLM" } else { "NTLM YQ==" };
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                } else {
                    result
                        .headers_mut()
                        .insert("Retry-After", "5".parse().unwrap());
                }
                result
            })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        let (caller, _, deadlines) =
            crate::git::endpoint::https_worker::native::recording_fixture("127.0.0.1");
        endpoint.client.set_native(caller);
        let mut envelope = open(1, port, 30_000);
        envelope.open.as_mut().unwrap().policy = AuthPolicy::WindowsDefault;
        endpoint.accept("request".into(), envelope).unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        while authenticated.load(Ordering::SeqCst) < 2 {
            assert!(
                tokio::time::Instant::now() < until,
                "the first attempt never ended"
            );
            settle(&mut endpoint, 0).await;
        }
        // The first attempt's exchange has ended in the throttle; no new exchange starts while the wait runs.
        settle(&mut endpoint, 4_999).await;
        let first = *deadlines
            .lock()
            .unwrap()
            .first()
            .expect("the first exchange");
        assert_eq!(
            deadlines.lock().unwrap().len(),
            1,
            "a retry started inside the wait"
        );
        // The wait is over: the retry is a new exchange.
        while deadlines.lock().unwrap().len() < 2 {
            assert!(
                tokio::time::Instant::now() < until,
                "the retry never started"
            );
            settle(&mut endpoint, 5_000).await;
        }
        let second = deadlines.lock().unwrap()[1];
        assert!(second > first, "the retry carried the first attempt's D");
        // Its D is anchored when it started: the open's connect allowance (10 s) from there, not from the open.
        assert!(
            second <= tokio::time::Instant::now() + Duration::from_secs(10),
            "the retry's D is anchored later than it started"
        );
        shut(&mut endpoint).await;
    });
}
