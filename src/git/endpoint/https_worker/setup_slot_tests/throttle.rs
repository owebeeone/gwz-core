//! What a response tells the pool's limit machines, through the worker's own
//! entry (adaptive concurrency design §4.5, §10.2 case 51): a 429 with
//! `Retry-After` on a reused connection holds its site, and when the hold ends
//! the site's idle connections are discarded in one step, so that no member
//! leases one the server's keep-alive may have closed.
use super::*;
use crate::git::endpoint::https_fixture::{Server, attach, response};
use hyper::header::HeaderValue;

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
async fn attempt(endpoint: &Endpoint, url: &str) -> Result<Prepared, Failure> {
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
        .0
}
/// Serves a prepared advertisement to its end: its connection goes idle.
async fn serve(prepared: Prepared) {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut buffer = [0; 64];
    while stream.read(&mut buffer).await.unwrap() != 0 {}
    stream.close().await.unwrap();
    task.await.unwrap();
}

#[test]
fn a_429_with_retry_after_holds_the_site_and_its_end_discards_the_idle_connections() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|request| {
            Box::pin(async move {
                let service = GitService::UploadPackAdvertisement;
                if request.uri().path().starts_with("/throttle") {
                    let mut reply = response(429, service, "");
                    reply
                        .headers_mut()
                        .insert("Retry-After", HeaderValue::from_static("2"));
                    reply
                } else {
                    response(200, service, "ok")
                }
            })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let key = Key::https("localhost", Destination::parse(&server.url).unwrap().port());
        let pool_governor = endpoint.client.governor().clone();
        pool_governor.begin_operation("operation", 32, false, 0);
        let governor = pool_governor.scoped("operation");
        let ok = server.url.clone();
        let throttled = server.url.replace("/repo", "/throttle");
        // Three connections at once, each served to its end: three idle, and
        // each Connected once its first exchange was answered.
        let (a, b, c) = tokio::join!(
            attempt(&endpoint, &ok),
            attempt(&endpoint, &ok),
            attempt(&endpoint, &ok)
        );
        for prepared in [a, b, c] {
            serve(prepared.unwrap()).await;
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 3);
        let now = endpoint.client.pool_now();
        assert_eq!(governor.view(&key, now).unwrap().connected, 3);
        // An open that leases one of them is throttled with Retry-After: 2.
        let refused = attempt(&endpoint, &throttled).await;
        assert!(refused.is_err());
        let held_at = Instant::now();
        let now = endpoint.client.pool_now();
        assert!(!governor.admission(&key, now).gate_open, "the site is held");
        assert!(!governor.exchange_may_begin(&key, now), "and so is a lease");
        // Nothing but time lifts it, and its end discards the idle connections.
        tokio::time::sleep(Duration::from_millis(1_900)).await;
        pool_governor.tick(endpoint.client.pool_now());
        assert!(
            !governor
                .admission(&key, endpoint.client.pool_now())
                .gate_open
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
        pool_governor.tick(endpoint.client.pool_now());
        assert!(held_at.elapsed() >= Duration::from_millis(2_000));
        assert!(
            governor
                .admission(&key, endpoint.client.pool_now())
                .gate_open
        );
        // The two idle connections are gone: the next open is on a new one.
        let prepared = attempt(&endpoint, &ok).await.unwrap();
        assert!(!prepared.opened.reused, "a discarded connection was leased");
        assert_eq!(server.connections.load(Ordering::SeqCst), 4);
        serve(prepared).await;
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_hold_set_between_admission_and_lease_keeps_the_exchange_from_the_server() {
    // The attempt was admitted before the hold; it leases an idle connection
    // after. Its discovery must wait for the hold, not go out inside it.
    runtime().block_on(async {
        let arrivals = Arc::new(Mutex::new(Vec::<std::time::Instant>::new()));
        let seen = arrivals.clone();
        let server = Server::start(Arc::new(move |_| {
            seen.lock().unwrap().push(std::time::Instant::now());
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let key = Key::https("localhost", Destination::parse(&server.url).unwrap().port());
        let ok = server.url.clone();
        endpoint
            .client
            .governor()
            .begin_operation("operation", 32, false, 0);
        serve(attempt(&endpoint, &ok).await.unwrap()).await;
        assert_eq!(arrivals.lock().unwrap().len(), 1);
        // Another member's 429 on a connection with nothing in flight: the
        // hold, and nothing else.
        let held_at = std::time::Instant::now();
        let now = endpoint.client.pool_now();
        endpoint.client.governor().scoped("operation").refused(
            &key,
            crate::git::endpoint::setup_retry::Conn(u64::MAX),
            crate::git::endpoint::setup_retry::Signal::Throttle,
            Some(1_500),
            false,
            now,
        );
        let prepared = attempt(&endpoint, &ok).await.unwrap();
        let second = arrivals.lock().unwrap()[1];
        assert!(
            second.duration_since(held_at) >= Duration::from_millis(1_400),
            "the discovery arrived {:?} into a 1.5 s hold",
            second.duration_since(held_at)
        );
        serve(prepared).await;
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
