//! Closure rows of the D19 pre-activation review for the native HTTPS authentication (lane `auth`): the
//! reviewers' tests, kept as they were specified. Code P2-1: a final answer that is not a credential rejection gets
//! the classification of the shared policy on Windows as on every platform. State P2-2: the Basic helper lookup
//! is on the helper's own allowance, and the native network deadline D does not run during it.
use super::*;
use crate::git::endpoint::https_fixture::{Server, input, response};
use crate::transport_host::{HttpsOpenFailure, policy_words};
use hyper::header::RETRY_AFTER;

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

/// What an open ended with, and what its attempt's response told the limit machines.
struct Ended {
    failure: Failure,
    refusal: Option<super::super::super::throttle::Rejection>,
    steps: usize,
}

/// An open under `policy` whose credentialed request is answered `status`: the server challenges an anonymous
/// request (Basic, or NTLM for the logon session) and answers every request that carries a credential with `status`
/// and no body, and a 429 with a `Retry-After`.
async fn answered(policy: AuthPolicy, status: u16) -> Ended {
    let challenge = if policy == AuthPolicy::WindowsDefault {
        "NTLM"
    } else {
        "Basic realm=x"
    };
    let server = Server::start(Arc::new(move |request| {
        let authorized = request.headers().contains_key(AUTHORIZATION);
        Box::pin(async move {
            if !authorized {
                let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, challenge.parse().unwrap());
                return result;
            }
            let mut result = response(status, GitService::UploadPackAdvertisement, Bytes::new());
            if status == 429 {
                result
                    .headers_mut()
                    .insert(RETRY_AFTER, "7".parse().unwrap());
            }
            result
        })
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let helper = (policy != AuthPolicy::WindowsDefault).then(|| {
        crate::git::endpoint::helper_script::git_fixture(
            dir.path(),
            "printf 'username=fixture\\npassword=fixture\\n\\n'",
        )
    });
    let mut endpoint = Endpoint::new(server.config(), helper, pool::Config::default()).unwrap();
    let (caller, port) = fake(false, 2);
    endpoint.client.set_native(caller);
    let mut request = input(&server, GitService::UploadPackAdvertisement);
    request.policy = policy;
    let (result, _, refusal) = endpoint
        .client
        .prepare_attempt_judged(
            request,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    let failure = result.err().expect("the credentialed answer is a refusal");
    let steps = port.steps.load(Ordering::Acquire);
    endpoint.client.finish_operation("operation");
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    Ended {
        failure,
        refusal,
        steps,
    }
}

/// The words the open's failure is shown in under `policy`.
fn words(failure: &Failure, policy: AuthPolicy) -> String {
    let (helpers_disabled, windows) = policy_words(Some(policy));
    HttpsOpenFailure {
        failure: failure.clone(),
        anonymous: None,
        attempts: None,
        service: Some(GitService::UploadPackAdvertisement),
        helpers_disabled,
        cli_hint: true,
        windows,
        host: Some("localhost".into()),
        redirect: None,
        machine: Default::default(),
        ended: None,
    }
    .reason()
}

#[test]
fn d19_code_p2_1_a_non_401_answer_to_a_windows_credential_is_classified_as_the_gh_policy_does() {
    run(async {
        for status in [403u16, 404, 429, 503] {
            let gh = answered(AuthPolicy::Gh, status).await;
            // The reference: the shared classification (403 and 404 refuse the repository, the rest are `Io`), and
            // a 429, or a 503 with Retry-After, is the limit machine's to requeue.
            let expected = if status == 403 || status == 404 {
                ErrorCode::RepositoryRefused
            } else {
                ErrorCode::Io
            };
            assert_eq!(gh.failure.code, expected, "Gh {status}");
            let gh_words = words(&gh.failure, AuthPolicy::Gh);
            for policy in [AuthPolicy::WindowsConfigured, AuthPolicy::WindowsDefault] {
                let windows = answered(policy, status).await;
                let row = format!("{policy:?} {status}");
                assert_eq!(windows.failure.code, gh.failure.code, "{row}");
                let facts = windows.failure.facts.as_ref().unwrap();
                assert_ne!(facts.authenticated, Some(false), "{row}");
                assert_eq!(facts.http_status, Some(i64::from(status)), "{row}");
                assert!(facts.credential_offered, "{row}");
                // The limit machine is told what the response said, as on the Unix path: a throttle requeues.
                assert_eq!(windows.refusal, gh.refusal, "{row}");
                let text = words(&windows.failure, policy);
                assert!(!text.contains("(HTTP 401)"), "{row}: {text}");
                assert!(!text.contains("rejected"), "{row}: {text}");
                if policy == AuthPolicy::WindowsConfigured || status != 403 {
                    assert_eq!(text, gh_words, "{row}");
                } else {
                    // The same refusal, worded for the credential that was sent: a Windows sign-in, not a helper's.
                    assert!(
                        text.starts_with("The server refused this account (HTTP 403) after gwz sent your Windows sign-in."),
                        "{row}: {text}"
                    );
                    assert!(!text.contains("credential helper"), "{row}: {text}");
                }
                if policy == AuthPolicy::WindowsDefault {
                    assert_eq!(windows.steps, 1, "{row}");
                }
            }
        }
    });
}

#[test]
fn d19_code_p2_1_a_401_answering_a_windows_basic_credential_is_still_its_rejection() {
    run(async {
        let rejected = answered(AuthPolicy::WindowsConfigured, 401).await;
        assert_eq!(rejected.failure.code, ErrorCode::Authentication);
        let facts = rejected.failure.facts.as_ref().unwrap();
        assert_eq!(facts.authenticated, Some(false));
        assert_eq!(facts.http_status, Some(401));
        assert!(words(&rejected.failure, AuthPolicy::WindowsConfigured).contains("(HTTP 401)"));
    });
}

/// An open under the configured policy against a server that challenges Basic and accepts the helper's credential,
/// whose helper takes `helper_seconds` to answer, with `connect` as the native network deadline D and `interaction`
/// as the helper's interaction allowance. The server closes a connection that has had no request for `idle`. It
/// returns what the open ended with, the logon session's starts and the connections the server accepted.
async fn slow_helper(
    helper_seconds: u32,
    connect: Duration,
    interaction: Duration,
    idle: Option<Duration>,
) -> (Result<Facts, Failure>, usize, usize) {
    let server = Server::start_closing_idle(
        Arc::new(|request| {
            let authorized = request.headers().contains_key(AUTHORIZATION);
            Box::pin(async move {
                let mut result = response(
                    if authorized { 200 } else { 401 },
                    GitService::UploadPackAdvertisement,
                    Bytes::new(),
                );
                if !authorized {
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, "Basic realm=x".parse().unwrap());
                }
                result
            })
        }),
        idle,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let script =
        format!("sleep {helper_seconds}; printf 'username=fixture\\npassword=fixture\\n\\n'");
    let helper = crate::git::endpoint::helper_script::git_fixture(dir.path(), &script);
    let mut endpoint =
        Endpoint::new(server.config(), Some(helper), pool::Config::default()).unwrap();
    let (caller, port) = fake(true, 2);
    endpoint.client.set_native(caller);
    let mut request = input(&server, GitService::UploadPackAdvertisement);
    request.policy = AuthPolicy::WindowsConfigured;
    let mut budget = endpoint.client.budget();
    budget.connect = Some(connect);
    budget.helper = interaction;
    let (result, _) = endpoint
        .client
        .prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None)
        .await;
    let starts = port.starts.load(Ordering::Acquire);
    let accepted = server.connections.load(Ordering::Acquire);
    let result = result.map(|prepared| prepared.opened.facts.clone());
    endpoint.client.finish_operation("operation");
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
    (result, starts, accepted)
}

#[test]
fn d19_state_p2_2_a_helper_outlasting_d_but_inside_its_allowance_authenticates() {
    run(async {
        // D is 1.5 s and the helper answers after 3 s: D is the network setup clock and does not run while the
        // helper works (GwzTransportWindowsParityDesign: local helper work is excluded from network clocks).
        let (result, starts, _) = slow_helper(
            3,
            Duration::from_millis(1_500),
            Duration::from_secs(60),
            None,
        )
        .await;
        let facts = result.unwrap_or_else(|failed| {
            panic!(
                "a helper inside its allowance authenticates, not {:?} ({:?})",
                failed.code, failed.setup_cause
            )
        });
        assert_eq!(facts.method, AuthMethod::Gh);
        assert!(facts.credential_offered);
        assert_eq!(starts, 0, "Basic never starts SSPI work");
    });
}

#[test]
fn d19_state_p2_2_a_helper_outlasting_its_allowance_fails_timeout_with_the_m4_report() {
    run(async {
        // The allowance (3 s) is shorter than the helper (30 s) and longer than D (1.5 s): the lookup ends at its
        // own allowance, and the failure says so (M4), not as a bare timeout of D.
        let (result, starts, _) = slow_helper(
            30,
            Duration::from_millis(1_500),
            Duration::from_secs(3),
            None,
        )
        .await;
        let failure = result.err().expect("a helper past its allowance times out");
        assert_eq!(failure.code, ErrorCode::Timeout);
        assert_eq!(failure.setup_cause, Some(SetupFailureCause::Interaction));
        assert_eq!(
            failure.detail.as_ref().and_then(|d| d.helper_budget_ms),
            Some(3_000)
        );
        assert_eq!(starts, 0, "a helper timeout never starts the logon session");
    });
}

#[test]
fn d19_state_p2_2_a_connection_the_server_closes_while_the_helper_works_is_replaced() {
    run(async {
        // The server closes a connection idle for 6 s and the helper takes 10 s: the connection that carried the
        // challenge is gone when the credential arrives. As on the `gh` path, which holds a challenge's connection
        // for 5 s, the open gives it back and takes a fresh one once the credential is in hand.
        let (result, starts, accepted) = slow_helper(
            10,
            Duration::from_secs(20),
            Duration::from_secs(60),
            Some(Duration::from_secs(6)),
        )
        .await;
        let facts = result.unwrap_or_else(|failed| {
            panic!(
                "a connection closed while the helper works is replaced, not {:?} (HTTP {:?})",
                failed.code,
                failed.facts.as_ref().and_then(|facts| facts.http_status)
            )
        });
        assert!(facts.credential_offered);
        assert_eq!(
            accepted, 2,
            "the authenticated request is on a fresh connection"
        );
        assert_eq!(starts, 0);
    });
}

#[test]
fn d19_state_p2_2_a_helper_inside_the_hold_keeps_the_challenges_connection() {
    run(async {
        // A helper that answers inside the 5 s hold sends the credential on the connection that carried the challenge.
        let (result, _, accepted) = slow_helper(
            1,
            Duration::from_secs(20),
            Duration::from_secs(60),
            Some(Duration::from_secs(6)),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(accepted, 1);
    });
}
