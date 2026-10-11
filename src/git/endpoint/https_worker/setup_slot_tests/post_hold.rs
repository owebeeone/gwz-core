//! A 429 with `Retry-After` on a POST holds the key's other POSTs before their
//! first byte (adaptive concurrency design §4.5 and §10.2 case 55): the server
//! has throttled exactly those exchanges, and the idle connections it may have
//! closed meanwhile are discarded when the hold ends.
use super::*;
use crate::git::endpoint::https_fixture::{Server, attach, input, response};
use hyper::header::HeaderValue;
use std::time::Instant as StdInstant;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A fixture whose first POST is throttled with `Retry-After: 2` and whose
/// every other request is answered; it records when each POST arrived.
async fn throttling_server() -> (Server, Arc<Mutex<Vec<StdInstant>>>) {
    let posts = Arc::new(Mutex::new(Vec::new()));
    let seen = posts.clone();
    let server = Server::start(Arc::new(move |request| {
        let seen = seen.clone();
        Box::pin(async move {
            if request.method() == "GET" {
                return response(200, GitService::UploadPackAdvertisement, "ok");
            }
            let first = {
                let mut seen = seen.lock().unwrap();
                seen.push(StdInstant::now());
                seen.len() == 1
            };
            let _ = request.into_body().collect().await;
            if first {
                let mut reply = response(429, GitService::UploadPackExchange, "");
                reply
                    .headers_mut()
                    .insert("Retry-After", HeaderValue::from_static("2"));
                return reply;
            }
            response(200, GitService::UploadPackExchange, "done")
        })
    }))
    .await;
    (server, posts)
}

async fn prepare(endpoint: &Endpoint, server: &Server, service: GitService) -> Prepared {
    let mut budget = endpoint.client.budget();
    endpoint
        .client
        .prepare_attempt(
            input(server, service),
            &CancellationToken::new(),
            &mut budget,
            &mut None,
        )
        .await
        .0
        .unwrap()
}

/// Serves a prepared exchange to its end, whatever its end is.
async fn run_to_end(prepared: Prepared, body: &[u8]) -> Vec<u8> {
    let (stream, task) = attach(prepared);
    if !body.is_empty() {
        let _ = stream.write_all(body).await;
    }
    let _ = stream.end_write().await;
    let mut output = Vec::new();
    let mut buffer = [0; 64];
    while let Ok(n) = stream.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..n]);
    }
    let _ = stream.close().await;
    task.await.unwrap();
    output
}

/// Four members finish their discovery: four idle connections, each Connected.
async fn four_idle(endpoint: &Endpoint, server: &Server) {
    let (a, b, c, d) = tokio::join!(
        prepare(endpoint, server, GitService::UploadPackAdvertisement),
        prepare(endpoint, server, GitService::UploadPackAdvertisement),
        prepare(endpoint, server, GitService::UploadPackAdvertisement),
        prepare(endpoint, server, GitService::UploadPackAdvertisement),
    );
    for prepared in [a, b, c, d] {
        run_to_end(prepared, b"").await;
    }
    assert_eq!(server.connections.load(Ordering::SeqCst), 4);
}

#[test]
fn a_post_429_holds_the_posts_of_the_members_between_discovery_and_their_post() {
    runtime().block_on(async {
        let (server, posts) = throttling_server().await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        endpoint
            .client
            .governor()
            .begin_operation("operation", 32, true, 0);
        four_idle(&endpoint, &server).await;
        // Member A's POST is throttled: it fails once, and the key is held
        // for POSTs.
        let a = prepare(&endpoint, &server, GitService::UploadPackExchange).await;
        run_to_end(a, b"want").await;
        let held_at = StdInstant::now();
        assert_eq!(posts.lock().unwrap().len(), 1);
        // The other three prepare their POSTs inside the hold: none is sent
        // until it has ended, and the idle connections are discarded, so each
        // resumes on a fresh connection.
        let (b, c, d) = tokio::join!(
            prepare(&endpoint, &server, GitService::UploadPackExchange),
            prepare(&endpoint, &server, GitService::UploadPackExchange),
            prepare(&endpoint, &server, GitService::UploadPackExchange),
        );
        for prepared in [&b, &c, &d] {
            assert!(!prepared.opened.reused, "a discarded connection was leased");
        }
        assert!(
            held_at.elapsed() >= Duration::from_millis(1_900),
            "prepared {:?} into a 2 s hold",
            held_at.elapsed()
        );
        for prepared in [b, c, d] {
            assert_eq!(run_to_end(prepared, b"want").await, b"done");
        }
        let posts = posts.lock().unwrap();
        assert_eq!(posts.len(), 4);
        for later in &posts[1..] {
            assert!(
                later.duration_since(posts[0]) >= Duration::from_millis(1_900),
                "a POST went out {:?} into the hold",
                later.duration_since(posts[0])
            );
        }
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_post_prepared_before_the_hold_waits_for_it_before_its_first_byte() {
    runtime().block_on(async {
        let (server, posts) = throttling_server().await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        endpoint
            .client
            .governor()
            .begin_operation("operation", 32, true, 0);
        four_idle(&endpoint, &server).await;
        // Member B has its POST prepared (it holds a leased connection) when
        // member A's POST is throttled.
        let b = prepare(&endpoint, &server, GitService::UploadPackExchange).await;
        let a = prepare(&endpoint, &server, GitService::UploadPackExchange).await;
        run_to_end(a, b"want").await;
        let held_at = StdInstant::now();
        assert_eq!(run_to_end(b, b"want").await, b"done");
        assert!(
            held_at.elapsed() >= Duration::from_millis(1_900),
            "sent {:?} into a 2 s hold",
            held_at.elapsed()
        );
        let posts = posts.lock().unwrap();
        assert!(posts[1].duration_since(posts[0]) >= Duration::from_millis(1_900));
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_hold_set_by_a_discovery_does_not_hold_the_posts_of_members_past_it() {
    runtime().block_on(async {
        let (server, posts) = throttling_server().await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        endpoint
            .client
            .governor()
            .begin_operation("operation", 32, true, 0);
        four_idle(&endpoint, &server).await;
        let key = Key::https("localhost", Destination::parse(&server.url).unwrap().port());
        let now = endpoint.client.pool_now();
        // A discovery-origin hold on the key (a request-level 429).
        endpoint.client.governor().scoped("operation").refused(
            &key,
            crate::git::endpoint::setup_retry::Conn(u64::MAX),
            crate::git::endpoint::setup_retry::Signal::Throttle,
            Some(5_000),
            false,
            now,
        );
        let begun = StdInstant::now();
        let b = prepare(&endpoint, &server, GitService::UploadPackExchange).await;
        run_to_end(b, b"want").await;
        assert!(
            begun.elapsed() < Duration::from_millis(1_000),
            "a POST waited {:?} for a discovery's hold",
            begun.elapsed()
        );
        assert_eq!(posts.lock().unwrap().len(), 1);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
