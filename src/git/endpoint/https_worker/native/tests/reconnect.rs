//! Closure rows of the D19 pre-activation review, round 2 (lane `auth2`), for the connection a native
//! authentication answers a challenge on. State P2-3: a credential is sent on a connection that can carry it, and a
//! challenge's connection that cannot is replaced by a fresh one, as the `gh` path replaces it. State P3-7: the
//! challenge's connection is given back when the hold lapses, not when the helper is done. Code P3-4: the
//! replacement's connect is the open's setup.
use super::closure::{Slow, run, slow};
use super::*;
use crate::git::endpoint::https_fixture::{Server, input, response};

/// A short lookup (2 s, under the 5 s hold) is no reason to send on a connection the server closed meanwhile.
#[test]
fn d19_state_p2_3_a_connection_the_server_closed_during_a_short_lookup_is_replaced() {
    run(async {
        let slowed = slow(Slow {
            idle: Some(Duration::from_secs(1)),
            ..Slow::helper(2)
        })
        .await;
        let facts = slowed.result.unwrap_or_else(|failed| {
            panic!(
                "a credential is not refused for a connection that could not carry it: {:?} (HTTP {:?})",
                failed.code,
                failed.facts.as_ref().and_then(|facts| facts.http_status)
            )
        });
        assert!(facts.credential_offered);
        assert_eq!(
            slowed.accepted, 2,
            "the credential is on a fresh connection"
        );
        assert_eq!(slowed.starts, 0, "Basic never starts SSPI work");
    });
}

/// A 401 marked `Connection: close` ends its connection whatever the lookup took.
#[test]
fn d19_state_p2_3_a_challenge_marked_connection_close_is_answered_on_a_fresh_connection() {
    run(async {
        let slowed = slow(Slow {
            close_after_challenge: true,
            ..Slow::helper(0)
        })
        .await;
        let facts = slowed.result.unwrap_or_else(|failed| {
            panic!(
                "a credential is not refused for a connection that could not carry it: {:?} (HTTP {:?})",
                failed.code,
                failed.facts.as_ref().and_then(|facts| facts.http_status)
            )
        });
        assert!(facts.credential_offered);
        assert_eq!(
            slowed.accepted, 2,
            "the credential is on a fresh connection"
        );
        assert_eq!(slowed.starts, 0, "Basic never starts SSPI work");
    });
}

/// What an open under the logon session ended with against a server that challenges `scheme`, marks its first 401
/// `Connection: close` when `close_first`, and answers a request that carries a token from `answers`. The server's
/// connections, and the logon session's starts, are counted.
async fn sspi_open(
    scheme: &'static str,
    close_first: bool,
    complete: bool,
    answers: u16,
) -> (Result<Facts, Failure>, usize, usize) {
    let server = Server::start(Arc::new(move |request| {
        let authorized = request.headers().contains_key(AUTHORIZATION);
        Box::pin(async move {
            let mut result = response(
                if authorized { answers } else { 401 },
                GitService::UploadPackAdvertisement,
                Bytes::new(),
            );
            if !authorized || answers == 401 {
                let offer = if authorized {
                    format!("{scheme} YQ==")
                } else {
                    scheme.to_string()
                };
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                if close_first || authorized {
                    result
                        .headers_mut()
                        .insert(hyper::header::CONNECTION, "close".parse().unwrap());
                }
            }
            result
        })
    }))
    .await;
    let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
    let (caller, port) = fake(complete, 2);
    endpoint.client.set_native(caller);
    let mut request = input(&server, GitService::UploadPackAdvertisement);
    request.policy = AuthPolicy::WindowsDefault;
    let (result, _) = endpoint
        .client
        .prepare_attempt(
            request,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    let starts = port.starts.load(Ordering::Acquire);
    let accepted = server.connections.load(Ordering::Acquire);
    let result = result.map(|prepared| prepared.opened.facts.clone());
    endpoint.client.finish_operation("operation");
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
    (result, starts, accepted)
}

/// A `Negotiate` 401 marked `Connection: close` ends its connection before the first token: the token goes on a
/// fresh one, with the binding of that connection.
#[test]
fn d19_state_p2_3_a_negotiate_challenge_marked_connection_close_takes_a_fresh_connection_for_its_first_token()
 {
    run(async {
        for scheme in ["Negotiate", "NTLM"] {
            let (result, starts, accepted) = sspi_open(scheme, true, true, 200).await;
            let facts = result.unwrap_or_else(|failed| {
                panic!(
                    "{scheme}: a first token is not refused for a connection that could not carry it: {:?}",
                    failed.code
                )
            });
            assert_eq!(facts.authenticated, Some(true), "{scheme}");
            assert_eq!(accepted, 2, "{scheme}: the token is on a fresh connection");
            assert_eq!(starts, 1, "{scheme}");
        }
    });
}

/// A close in the middle of an exchange ends it: the exchange belongs to its connection, so there is no fresh one
/// to continue it on, and the refusal is the open's (unchanged by the first-token rule above).
#[test]
fn d19_state_p2_3_a_close_in_the_middle_of_an_ntlm_exchange_still_ends_it() {
    run(async {
        let (result, starts, accepted) = sspi_open("NTLM", false, false, 401).await;
        let failed = result.err().expect("an exchange cannot move connections");
        assert_eq!(failed.code, ErrorCode::Authentication);
        assert_eq!(accepted, 1);
        assert_eq!(starts, 1);
    });
}

/// The challenge's connection is kept for the `gh` path's hold (5 s) and no longer: a helper that is still working
/// at the hold's end no longer has a connection of the open's idle against its site's room.
#[test]
fn d19_state_p3_7_the_challenge_connection_is_given_back_when_the_hold_lapses_during_the_lookup() {
    run(async {
        let slowed = slow(Slow {
            sample_open_at: Some(Duration::from_secs(7)),
            ..Slow::helper(10)
        })
        .await;
        assert!(slowed.result.is_ok());
        assert_eq!(slowed.accepted, 2);
        assert_eq!(
            slowed.open,
            Some(0),
            "the challenge's connection was still open and idle 7 s into a 10 s lookup"
        );
    });
}

/// The replacement's connect is the open's setup: its failure is the first connect's, with its origin, and the limit
/// machine is told, so the endpoint retries it as it retries the `gh` path's.
#[test]
fn d19_code_p3_4_a_re_lease_whose_connect_fails_is_the_opens_setup_failure() {
    run(async {
        // The server stops listening once it has accepted the challenge's connection, and the helper outlasts the
        // 5 s hold: the credential's connection is refused by the host.
        let slowed = slow(Slow {
            stop_after: Some(1),
            ..Slow::helper(6)
        })
        .await;
        let failed = slowed.result.err().expect("the reconnect is refused");
        assert_eq!(failed.code, ErrorCode::Unavailable);
        assert_eq!(
            failed.setup_cause,
            Some(SetupFailureCause::ConnectionRefused),
            "the connect's origin is kept"
        );
        assert!(
            matches!(slowed.connect, FirstConnect::Failed),
            "the connect failure is the attempt's setup failure"
        );
        assert!(
            slowed.refusal.is_some(),
            "the limit machine is told of the refused connect"
        );
    });
}

/// Step 1.4: one request is one pool owner, so the connection a native challenge's re-lease takes is counted with the
/// request's other checkouts, not with an owner of its own. At a limit of two, an open whose first 401 is marked
/// `Connection: close` (so it re-leases) and one that does not hold both of the request's two connections, and a third
/// open of the request is refused.
#[test]
fn a_re_lease_is_counted_with_its_request_s_other_checkouts_under_the_request_s_limits() {
    run(async {
        let first = Arc::new(AtomicUsize::new(0));
        let server = Server::start(Arc::new(move |request| {
            let authorized = request.headers().contains_key(AUTHORIZATION);
            let first = first.clone();
            Box::pin(async move {
                let mut result = response(
                    if authorized { 200 } else { 401 },
                    GitService::UploadPackAdvertisement,
                    Bytes::new(),
                );
                if !authorized {
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap());
                    // Only the first challenge ends its connection, so only the first open re-leases.
                    if first.fetch_add(1, Ordering::SeqCst) == 0 {
                        result
                            .headers_mut()
                            .insert(hyper::header::CONNECTION, "close".parse().unwrap());
                    }
                }
                result
            })
        }))
        .await;
        let config = pool::Config {
            per_user_host: 64,
            per_host: 64,
            total: 64,
            allocation_timeout_ms: 300,
            ..pool::Config::default()
        };
        let mut endpoint = Endpoint::new(server.config(), None, config).unwrap();
        let (caller, _) = fake(true, 2);
        endpoint.client.set_native(caller);
        endpoint.client.set_operation_limits(
            "operation",
            pool::Capacity {
                per_user_host: 2,
                per_host: 2,
                total: 256,
                max_requests: 1024,
            },
        );
        let mut held = Vec::new();
        let mut outcomes = Vec::new();
        for _ in 0..3 {
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::WindowsDefault;
            let (result, _) = endpoint
                .client
                .prepare_attempt(
                    request,
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await;
            outcomes.push(result.is_ok());
            held.extend(result.ok());
        }
        assert_eq!(
            outcomes,
            [true, true, false],
            "the re-leased connection is one of the request's two"
        );
        drop(held);
        endpoint.client.finish_operation("operation");
        let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
        assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
    });
}
