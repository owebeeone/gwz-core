//! A member that a server throttled to the end of its attempts reaches the user
//! with the `capacity` error code and the words of its cause (adaptive
//! concurrency design §5.3; operator decision 2026-10-13). The server is the
//! fixture's; the failure is the endpoint's own `OpenFailed`, worded as the
//! host words it.
use super::{
    requeue_tests::{collect, replies, server},
    retry_tests::{endpoint, open, port, runtime, shut},
    *,
};
use crate::git::endpoint::{https_fixture as fixture, setup_retry};
use gwz_transport::protocol::{CapacityCause, CapacitySignal};
use hyper::header::HeaderValue;

/// A server that answers every discovery with `status` and the `Retry-After`.
async fn throttling(status: u16, retry_after: Option<&'static str>) -> fixture::Server {
    fixture::Server::start(Arc::new(move |_| {
        Box::pin(async move {
            let service = GitService::UploadPackAdvertisement;
            let mut reply = fixture::response(status, service, "");
            if let Some(wait) = retry_after {
                reply
                    .headers_mut()
                    .insert("Retry-After", HeaderValue::from_static(wait));
            }
            reply
        })
    }))
    .await
}

/// What the user is told of an open that failed with `failure`, as the host
/// words it for the HTTPS route, with the `note` the endpoint left beside it.
pub(super) fn user_error(
    failure: &Failure,
    note: Option<setup_retry::EndNote>,
) -> crate::model::ModelError {
    crate::transport_host::HttpsOpenFailure {
        ended: note.map(|note| setup_retry::Ended::new(note, "127.0.0.1", 443)),
        attempts: setup_retry::reported_attempt(failure),
        failure: failure.clone(),
        anonymous: None,
        service: Some(GitService::UploadPackAdvertisement),
        helpers_disabled: false,
        cli_hint: true,
        windows: false,
        host: None,
        redirect: None,
        machine: Default::default(),
    }
    .model_error()
    .expect("a Capacity failure with its detail is a typed error")
}

async fn failed_open(
    server: &fixture::Server,
    max_retries: u32,
    until: Duration,
) -> (u16, Failure) {
    let (port, failure, _) = failed_open_noted(server, max_retries, until).await;
    (port, failure)
}

/// `failed_open`, and the note the endpoint left in its handoff for the open
/// (`EndNote`), which the driver of a real command takes.
async fn failed_open_noted(
    server: &fixture::Server,
    max_retries: u32,
    until: Duration,
) -> (u16, Failure, Option<setup_retry::EndNote>) {
    let port = port(server);
    let mut endpoint = endpoint(server.config(), 8, max_retries);
    let handoff = crate::git::endpoint::ssh_handoff::Handoff::default();
    endpoint.set_handoff(handoff.clone());
    endpoint
        .accept("request".into(), open(1, port, 30_000))
        .unwrap();
    let mut published = Vec::new();
    collect(&mut endpoint, until, &mut published, |p| {
        !replies(p).is_empty()
    })
    .await;
    let failure = replies(&published)[0]
        .open_failed
        .clone()
        .expect("the open failed");
    shut(&mut endpoint).await;
    (port, failure, handoff.take_end("session", 1))
}

#[test]
fn a_429_storm_with_a_short_retry_after_reaches_the_user_as_a_server_throttle() {
    runtime().block_on(async {
        let server = throttling(429, Some("1")).await;
        let (port, failure, note) = failed_open_noted(&server, 1, Duration::from_secs(20)).await;
        assert_eq!(failure.code, ErrorCode::Capacity);
        // Both attempts were refused: the endpoint says so beside the failure.
        assert_eq!(note, Some(setup_retry::EndNote::Refused { count: 2 }));
        let detail = failure.detail.as_ref().expect("a detail");
        let capacity = detail.capacity.as_ref().expect("the capacity detail");
        assert_eq!(capacity.cause, CapacityCause::ServerThrottle);
        assert_eq!(capacity.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(capacity.port, Some(i64::from(port)));
        assert_eq!(capacity.max_per_host, Some(8));
        assert_eq!(capacity.last_signal, Some(CapacitySignal::Http429));
        assert_eq!(capacity.retry_after_ms, Some(1_000));
        assert!(capacity.limit_applied.is_some_and(|limit| limit <= 8));
        let count = detail.retry_attempt.as_ref().expect("the attempt count");
        assert_eq!((count.attempt, count.attempts), (2, 2));
        // The words the user reads.
        let error = user_error(&failure, note);
        assert_eq!(error.code, crate::model::ErrorCode::Capacity);
        let text = error.message;
        assert!(
            text.starts_with(&format!("127.0.0.1:{port} kept refusing connections")),
            "{text}"
        );
        assert!(
            text.ends_with("retry later, or lower --max-per-host (attempt 2 of 2)"),
            "{text}"
        );
        assert!(text.contains("; gave up after "), "{text}");
    });
}

#[test]
fn one_refusal_alone_says_the_last_attempt_was_refused_and_not_that_the_host_kept_refusing() {
    runtime().block_on(async {
        // `--max-retries 0`: the first refusal is the member's only attempt.
        let (server, _) = server(Arc::new(|_, _| 429)).await;
        let (port, failure, note) = failed_open_noted(&server, 0, Duration::from_secs(10)).await;
        assert_eq!(note, Some(setup_retry::EndNote::Refused { count: 1 }));
        let text = user_error(&failure, note).message;
        assert!(
            text.starts_with(&format!("127.0.0.1:{port} refused the last attempt")),
            "{text}"
        );
        assert!(!text.contains("kept"), "{text}");
    });
}

#[test]
fn reset_reset_reset_then_a_429_says_the_last_attempt_was_refused_not_that_the_host_kept_refusing()
{
    runtime().block_on(async {
        // The first three connections are reset; the fourth reaches a server
        // that throttles. The member's last attempt is the only one refused.
        let (server, _) = server(Arc::new(|_, _| 429)).await;
        let (front, accepted) = super::down_tests::scripted(port(&server), vec![0, 1, 2]);
        let mut endpoint = endpoint(server.config(), 8, 3);
        let handoff = crate::git::endpoint::ssh_handoff::Handoff::default();
        endpoint.set_handoff(handoff.clone());
        endpoint
            .accept("request".into(), open(1, front, 30_000))
            .unwrap();
        let mut published = Vec::new();
        let mut now = 0;
        while replies(&published).is_empty() {
            assert!(
                now < 120_000,
                "never failed: {} connections",
                accepted.load(std::sync::atomic::Ordering::SeqCst)
            );
            published.extend(super::retry_tests::settle(&mut endpoint, now).await);
            now += 500;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let failure = replies(&published)[0]
            .open_failed
            .clone()
            .expect("the open failed");
        shut(&mut endpoint).await;
        assert_eq!(failure.code, ErrorCode::Capacity);
        let count = failure
            .detail
            .as_ref()
            .and_then(|d| d.retry_attempt.as_ref())
            .expect("the attempt count");
        assert_eq!((count.attempt, count.attempts), (4, 4));
        let note = handoff.take_end("session", 1);
        assert_eq!(note, Some(setup_retry::EndNote::Refused { count: 1 }));
        let text = user_error(&failure, note).message;
        assert!(
            text.starts_with(&format!("127.0.0.1:{front} refused the last attempt")),
            "{text}"
        );
        assert!(!text.contains("kept refusing"), "{text}");
        assert!(text.ends_with("(attempt 4 of 4)"), "{text}");
    });
}

#[test]
fn a_bare_429_storm_is_a_server_throttle_with_no_retry_after() {
    runtime().block_on(async {
        let (server, _) = server(Arc::new(|_, _| 429)).await;
        let (_, failure) = failed_open(&server, 1, Duration::from_secs(10)).await;
        let capacity = failure
            .detail
            .as_ref()
            .and_then(|detail| detail.capacity.as_ref())
            .expect("the capacity detail");
        assert_eq!(capacity.cause, CapacityCause::ServerThrottle);
        assert_eq!(capacity.retry_after_ms, None);
        assert_eq!(capacity.last_signal, Some(CapacitySignal::Http429));
    });
}

#[test]
fn a_retry_after_beyond_what_a_command_holds_reaches_the_user_as_its_own_cause() {
    runtime().block_on(async {
        // At `--max-retries 0` the first refusal is the member's last, so the
        // test does not wait out the hold of 30 s a longer `Retry-After` sets.
        let server = throttling(429, Some("120")).await;
        let (port, failure) = failed_open(&server, 0, Duration::from_secs(10)).await;
        assert_eq!(failure.code, ErrorCode::Capacity);
        let capacity = failure
            .detail
            .as_ref()
            .and_then(|detail| detail.capacity.as_ref())
            .expect("the capacity detail");
        assert_eq!(capacity.cause, CapacityCause::ServerRetryAfter);
        assert_eq!(capacity.retry_after_ms, Some(120_000));
        let error = user_error(&failure, None);
        assert_eq!(error.code, crate::model::ErrorCode::Capacity);
        assert_eq!(
            error.message,
            format!(
                "127.0.0.1:{port} asked to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min (attempt 1 of 1)"
            )
        );
    });
}

#[test]
fn a_503_with_a_retry_after_names_its_status_and_a_403_names_none() {
    runtime().block_on(async {
        for (status, signal) in [(503, Some(CapacitySignal::Http503)), (403, None)] {
            let server = throttling(status, Some("2")).await;
            let (_, failure) = failed_open(&server, 0, Duration::from_secs(10)).await;
            assert_eq!(failure.code, ErrorCode::Capacity, "{status}");
            let capacity = failure
                .detail
                .as_ref()
                .and_then(|detail| detail.capacity.as_ref())
                .expect("the capacity detail");
            assert_eq!(capacity.cause, CapacityCause::ServerThrottle, "{status}");
            assert_eq!(capacity.last_signal, signal, "{status}");
            assert_eq!(capacity.retry_after_ms, Some(2_000), "{status}");
        }
    });
}

#[test]
fn a_bare_503_stays_what_it_was_and_carries_no_capacity_detail() {
    runtime().block_on(async {
        let server = throttling(503, None).await;
        let (_, failure) = failed_open(&server, 3, Duration::from_secs(10)).await;
        assert_eq!(failure.code, ErrorCode::Io);
        assert!(
            failure
                .detail
                .as_ref()
                .is_none_or(|detail| detail.capacity.is_none())
        );
    });
}
