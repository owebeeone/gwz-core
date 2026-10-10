//! The HTTPS endpoint behind a hold (adaptive concurrency design §4.5 and
//! §5.2): after a 429 with `Retry-After` the site's opens wait, and the time
//! they wait is no allocation; they start when the hold ends.
use super::{
    retry_tests::{code, endpoint, open, port, runtime, settle, shut},
    *,
};
use crate::git::endpoint::https_fixture as fixture;
use hyper::header::HeaderValue;
use std::{sync::atomic::AtomicUsize, task::Waker};

#[test]
fn an_open_behind_a_hold_waits_with_its_allocation_stopped_and_starts_when_it_ends() {
    runtime().block_on(async {
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        let server = fixture::Server::start(Arc::new(move |_| {
            let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                let service = GitService::UploadPackAdvertisement;
                if first {
                    let mut reply = fixture::response(429, service, "");
                    reply
                        .headers_mut()
                        .insert("Retry-After", HeaderValue::from_static("2"));
                    reply
                } else {
                    fixture::response(200, service, "ok")
                }
            })
        }))
        .await;
        let port = port(&server);
        let mut endpoint = endpoint(server.config(), 8, 3);
        // The first open meets the 429 and fails as it does without a hold.
        endpoint
            .accept("request".into(), open(1, port, 30_000))
            .unwrap();
        let first = settle(&mut endpoint, 0).await;
        assert_eq!(first.len(), 1);
        assert_eq!(code(&first[0]), ErrorCode::Io);
        let held_at = std::time::Instant::now();
        // A second open, with an allocation of 500 ms, is held while the
        // endpoint's own clock runs 5 s past it.
        endpoint
            .accept("request".into(), open(2, port, 500))
            .unwrap();
        assert!(settle(&mut endpoint, 0).await.is_empty());
        assert!(settle(&mut endpoint, 5_000).await.is_empty());
        assert_eq!(server.connections.load(Ordering::SeqCst), 1, "no start");
        // The hold ends in the pool's time; the open then starts and succeeds,
        // with what was left of its allocation.
        let mut cx = Context::from_waker(Waker::noop());
        let mut published = Vec::new();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !published
            .iter()
            .any(|message: &Envelope| message.kind == MessageKind::Opened)
        {
            assert!(std::time::Instant::now() < until, "the open never started");
            endpoint.step(5_000, &mut cx).unwrap();
            while let Some(outbound) = endpoint.take_outbound(&mut cx) {
                published.push(outbound.envelope);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(held_at.elapsed() >= Duration::from_millis(1_900));
        assert!(
            published
                .iter()
                .all(|message| message.kind != MessageKind::OpenFailed),
            "the held open failed"
        );
        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
        shut(&mut endpoint).await;
    });
}
