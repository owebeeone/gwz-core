//! A throttle requeues its member and does not fail it, and the believed limit
//! is tested for the whole command (adaptive concurrency design §4.5, §4.7,
//! §5.1, §5.3 and §10.2 cases 1, 9 and 43). The server is the fixture's, with
//! a rule for which connection it answers.
use super::{
    retry_tests::{code, endpoint, open, port, runtime, settle, shut},
    *,
};
use crate::git::endpoint::{
    https_fixture as fixture,
    setup_retry::{State, View},
};
use std::sync::atomic::{AtomicUsize, Ordering};

type Answer = Arc<dyn Fn(usize, usize) -> u16 + Send + Sync>;

/// A server whose answer to a discovery is `answer(connection, request)`,
/// numbered from 1; and the count of requests it has seen.
async fn server(answer: Answer) -> (fixture::Server, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let server = fixture::Server::start(Arc::new(move |request| {
        let answer = answer.clone();
        let number = seen.fetch_add(1, Ordering::SeqCst) + 1;
        let connection = request
            .extensions()
            .get::<fixture::ConnectionId>()
            .map_or(0, |id| id.0);
        Box::pin(async move {
            let service = GitService::UploadPackAdvertisement;
            fixture::response(answer(connection, number), service, "ok")
        })
    }))
    .await;
    (server, requests)
}

/// The open's own replies: `Opened` and `OpenFailed`, not the stream's data.
fn replies(published: &[Envelope]) -> Vec<&Envelope> {
    published
        .iter()
        .filter(|m| matches!(m.kind, MessageKind::Opened | MessageKind::OpenFailed))
        .collect()
}

/// Steps at `now` until `done` holds of what has been published, for at most
/// `limit`.
async fn collect(
    endpoint: &mut HttpsEndpoint,
    limit: Duration,
    published: &mut Vec<Envelope>,
    done: impl Fn(&[Envelope]) -> bool,
) {
    let begun = tokio::time::Instant::now();
    while !done(published) {
        assert!(
            begun.elapsed() < limit,
            "never happened: {} replies",
            replies(published).len()
        );
        published.extend(settle(endpoint, 0).await);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn operation_view(endpoint: &HttpsEndpoint, port: u16) -> Option<View> {
    let name = endpoint.operations.get("request")?.name.clone();
    endpoint.client.governor().scoped(&name).view(
        &pool::Key::https("127.0.0.1", port),
        endpoint.client.pool_now(),
    )
}

#[test]
fn a_throttled_discovery_is_requeued_and_succeeds_when_the_server_relents() {
    runtime().block_on(async {
        let (server, requests) =
            server(Arc::new(|_, request| if request <= 3 { 429 } else { 200 })).await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let mut published = Vec::new();
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| !replies(p).is_empty(),
        )
        .await;
        assert_eq!(replies(&published)[0].kind, MessageKind::Opened);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            4,
            "three 429s, then the answer"
        );
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_member_that_is_throttled_to_the_end_of_its_attempts_finishes_with_capacity() {
    runtime().block_on(async {
        let (server, requests) = server(Arc::new(|_, _| 429)).await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 2);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let mut published = Vec::new();
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| !replies(p).is_empty(),
        )
        .await;
        assert_eq!(code(replies(&published)[0]), ErrorCode::Capacity);
        let count = replies(&published)[0]
            .open_failed
            .as_ref()
            .unwrap()
            .detail
            .as_ref()
            .and_then(|detail| detail.retry_attempt.as_ref())
            .unwrap();
        assert_eq!((count.attempt, count.attempts), (3, 3));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            3,
            "no attempt above 1 + --max-retries"
        );
        shut(&mut endpoint).await;
    });
}

#[test]
fn at_max_retries_zero_a_throttle_fails_its_member_at_once_with_capacity() {
    runtime().block_on(async {
        let (server, requests) = server(Arc::new(|_, _| 429)).await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 0);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let mut published = Vec::new();
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| !replies(p).is_empty(),
        )
        .await;
        assert_eq!(code(replies(&published)[0]), ErrorCode::Capacity);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_bare_503_with_nothing_else_counted_fails_as_it_did() {
    runtime().block_on(async {
        let (server, requests) = server(Arc::new(|_, _| 503)).await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let mut published = Vec::new();
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| !replies(p).is_empty(),
        )
        .await;
        // hi = 0: the retry machine's, which returns a failure after the
        // first request byte once; and it is no throttle, so it is `Io`.
        assert_eq!(code(replies(&published)[0]), ErrorCode::Io);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_limit_is_found_tested_for_the_command_and_climbed_when_it_lifts() {
    runtime().block_on(async {
        // The server answers its first two connections and throttles the rest,
        // until the test lifts it.
        let allowed = Arc::new(AtomicUsize::new(2));
        let rule = allowed.clone();
        let (server, requests) = server(Arc::new(move |connection, _| {
            if connection <= rule.load(Ordering::SeqCst) {
                200
            } else {
                429
            }
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 4, 6);
        for stream in 1..=4 {
            endpoint
                .accept("request".into(), open(stream, port, 60_000))
                .unwrap();
        }
        let mut published = Vec::new();
        // The first wave: two are answered, two are throttled and requeued.
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |p| replies(p).len() == 2,
        )
        .await;
        assert!(
            replies(&published)
                .iter()
                .all(|m| m.kind == MessageKind::Opened)
        );
        let view = operation_view(&endpoint, port).unwrap();
        assert_eq!((view.state, view.n), (State::Stable, 2), "the limit is 2");
        // The probe timer, with no one's help, tests N + 1 on a requeued member:
        // refused while the limit stands, and the others stay waiting.
        let before = requests.load(Ordering::SeqCst);
        collect(
            &mut endpoint,
            Duration::from_secs(10),
            &mut published,
            |_| requests.load(Ordering::SeqCst) > before,
        )
        .await;
        assert_eq!(operation_view(&endpoint, port).unwrap().n, 2);
        assert_eq!(
            replies(&published).len(),
            2,
            "no member failed: a test is never a final attempt"
        );
        // The limit lifts: the next tests succeed and N climbs until both are served.
        allowed.store(10, Ordering::SeqCst);
        collect(
            &mut endpoint,
            Duration::from_secs(20),
            &mut published,
            |p| replies(p).len() == 4,
        )
        .await;
        assert!(
            replies(&published)
                .iter()
                .all(|m| m.kind == MessageKind::Opened)
        );
        assert_eq!(replies(&published).len(), 4);
        shut(&mut endpoint).await;
    });
}
