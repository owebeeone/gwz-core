//! The HTTPS endpoint over a Down key (adaptive concurrency design §5.5,
//! cases 29 and 44): the opens held when the budget is spent finish with its
//! failure, a later open parks for the next retest, and after two retests in
//! a row have failed an arrival finishes at once.
use super::{
    retry_tests::{closing, code, endpoint, open, port, runtime, settle, shut},
    *,
};
use crate::git::endpoint::{https_connection, https_fixture as fixture};
use std::{
    net::{Shutdown, TcpListener, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
};

/// A listener that closes its first `refuse` connections and forwards the
/// rest to the fixture server on `target`, as a host that comes back; it
/// counts what it accepts.
fn returning(target: u16, refuse: usize) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    std::thread::spawn(move || {
        while let Ok((client, _)) = listener.accept() {
            if counter.fetch_add(1, Ordering::AcqRel) < refuse {
                continue;
            }
            let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else {
                continue;
            };
            let (mut up, mut down) = (client.try_clone().unwrap(), server.try_clone().unwrap());
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut up, &mut down);
                let _ = down.shutdown(Shutdown::Write);
            });
            std::thread::spawn(move || {
                let (mut client, mut server) = (client, server);
                let _ = std::io::copy(&mut server, &mut client);
                let _ = client.shutdown(Shutdown::Write);
            });
        }
    });
    (port, accepted)
}

fn counted(message: &Envelope) -> bool {
    message
        .open_failed
        .as_ref()
        .and_then(|failure| failure.detail.as_ref())
        .and_then(|detail| detail.retry_attempt.as_ref())
        .is_some()
}

#[test]
fn held_opens_finish_when_the_budget_is_spent_and_later_ones_park_for_the_retests() {
    runtime().block_on(async {
        let (port, accepted) = closing();
        // One connection at a time, one retry: attempts 1 and 2 spend the
        // budget, which finishes the opens held behind them.
        let mut endpoint = endpoint(https_connection::Config::default(), 1, 1);
        for stream in 1..=4 {
            endpoint
                .accept("request".into(), open(stream, port, 30_000))
                .unwrap();
        }
        assert!(settle(&mut endpoint, 0).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 1);
        let finished = settle(&mut endpoint, 1_000).await;
        assert_eq!(accepted.load(Ordering::Acquire), 2);
        assert_eq!(finished.len(), 4, "every open held on the key shares it");
        assert!(
            finished
                .iter()
                .all(|message| code(message) == ErrorCode::Io)
        );
        assert!(
            finished.iter().any(counted),
            "the spent open reports its count"
        );
        // The key is Down. An open that arrives parks: no handshake until the
        // retest is due, 30 s after the budget was spent.
        endpoint
            .accept("request".into(), open(5, port, 30_000))
            .unwrap();
        assert!(settle(&mut endpoint, 30_999).await.is_empty());
        assert_eq!(accepted.load(Ordering::Acquire), 2);
        // It carries the retest and shares its failure, with no attempt count.
        let retest = settle(&mut endpoint, 31_000).await;
        assert_eq!(accepted.load(Ordering::Acquire), 3);
        assert_eq!(retest.len(), 1);
        assert!(!counted(&retest[0]));
        // The second retest, 30 s on, also fails.
        endpoint
            .accept("request".into(), open(6, port, 30_000))
            .unwrap();
        assert!(settle(&mut endpoint, 60_999).await.is_empty());
        let second = settle(&mut endpoint, 61_000).await;
        assert_eq!(accepted.load(Ordering::Acquire), 4);
        assert_eq!(second.len(), 1);
        // Two in a row: an arrival before the next is due finishes at once.
        endpoint
            .accept("request".into(), open(7, port, 30_000))
            .unwrap();
        let at_once = settle(&mut endpoint, 61_001).await;
        assert_eq!(accepted.load(Ordering::Acquire), 4, "no handshake");
        assert_eq!(at_once.len(), 1);
        assert_eq!(code(&at_once[0]), ErrorCode::Io);
        assert!(!counted(&at_once[0]));
        shut(&mut endpoint).await;
    });
}

#[test]
fn a_retest_that_succeeds_heals_the_key_and_the_parked_open_is_served() {
    runtime().block_on(async {
        let server = fixture::Server::start(Arc::new(|_| {
            Box::pin(async { fixture::response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        // The host refuses its first two connections, then answers.
        let (front, accepted) = returning(port(&server), 2);
        let mut endpoint = endpoint(server.config(), 1, 1);
        let view = |endpoint: &HttpsEndpoint| {
            let name = endpoint.operations.get("request")?.name.clone();
            endpoint.client.governor().scoped(&name).view(
                &pool::Key::https("127.0.0.1", front),
                endpoint.client.pool_now(),
            )
        };
        for stream in 1..=2 {
            endpoint
                .accept("request".into(), open(stream, front, 30_000))
                .unwrap();
        }
        assert!(settle(&mut endpoint, 0).await.is_empty());
        let finished = settle(&mut endpoint, 1_000).await;
        assert_eq!(finished.len(), 2);
        assert_eq!(accepted.load(Ordering::Acquire), 2);
        // Down: the limit machine was told the key left Healthy.
        assert_eq!(view(&endpoint).map(|v| v.outage), Some(true));
        endpoint
            .accept("request".into(), open(3, front, 30_000))
            .unwrap();
        assert!(settle(&mut endpoint, 30_999).await.is_empty());
        // The retest connects, the host has returned: the open is served and
        // the limit machine is told the key is Healthy again.
        let served = settle(&mut endpoint, 31_000).await;
        let opened: Vec<_> = served
            .iter()
            .filter(|message| message.kind == MessageKind::Opened)
            .collect();
        assert_eq!(opened.len(), 1, "{served:?}");
        assert_eq!(accepted.load(Ordering::Acquire), 3);
        assert_eq!(view(&endpoint).map(|v| v.outage), Some(false));
        shut(&mut endpoint).await;
    });
}
