//! The worker completes the close the initiator asks for when the stream says
//! it can, not on a timer: until the initiator's Close arrives the worker
//! waits to be told of a change, and a wait that is not told ends at its
//! deadline. The tests count how often the wait is polled, which a fixed
//! sleep inflates (TR8.1: a 2 ms retry sleep gated every command's exit).
use super::serve::finish_close;
use super::*;
use gwz_transport::stream::{Config as StreamConfig, Side};
use std::{future::poll_fn, sync::atomic::AtomicUsize};
use tokio::task::JoinHandle;

/// An initiator and an endpoint stream, and the host's loop between them.
/// The loop tells the worker's wait of every change it makes, as the host's
/// `accept`, `step` and `take_outbound` do.
struct Wired {
    initiator: Stream,
    endpoint: Stream,
    peer: Arc<MessageEndpoint>,
    ready: CloseWake,
    pump: JoinHandle<()>,
}
impl Drop for Wired {
    fn drop(&mut self) {
        self.pump.abort();
    }
}
fn wire() -> Wired {
    let mut config = StreamConfig::new("session", 1, Side::Initiator);
    config.profile_version = 2;
    config.io_timeout_ms = 30_000;
    let (initiator, left) = Stream::new(config.clone()).unwrap();
    config.side = Side::Endpoint;
    let (endpoint, right) = Stream::new(config).unwrap();
    let (left, peer) = (Arc::new(left), Arc::new(right));
    let ready = CloseWake::default();
    let pump = {
        let (peer, ready) = (peer.clone(), ready.clone());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = left.next_message() => match message {
                        Ok(Some(message)) => {
                            if peer.deliver(message).is_err() { break; }
                            ready.notify();
                        }
                        _ => break,
                    },
                    message = peer.next_message() => match message {
                        Ok(Some(message)) => {
                            let _ = left.deliver(message);
                            ready.notify();
                        }
                        _ => break,
                    },
                }
            }
        })
    };
    Wired {
        initiator,
        endpoint,
        peer,
        ready,
        pump,
    }
}

/// Both sides have ended their writes: what is left is the initiator's Close.
async fn both_ended(wired: &Wired) {
    wired.initiator.end_write().await.unwrap();
    assert_eq!(wired.endpoint.read(&mut [0]).await.unwrap(), 0);
    wired.endpoint.end_write().await.unwrap();
}

/// `finish_close` as a task that counts its polls.
fn closing(
    wired: &Wired,
    within: Duration,
) -> (JoinHandle<Result<(), ErrorCode>>, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let (peer, ready, counted) = (wired.peer.clone(), wired.ready.clone(), polls.clone());
    let task = tokio::spawn(async move {
        let facts = Mutex::new(Facts::default());
        let work = finish_close(
            &peer,
            &ready,
            Disposition::Reusable,
            &facts,
            Instant::now() + within,
        );
        tokio::pin!(work);
        poll_fn(|cx| {
            counted.fetch_add(1, Ordering::SeqCst);
            work.as_mut().poll(cx)
        })
        .await
    });
    (task, polls)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_close_that_has_not_arrived_is_waited_for_without_polling_and_completes_when_it_does() {
    runtime().block_on(async {
        let wired = wire();
        both_ended(&wired).await;
        let (task, polls) = closing(&wired, Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            !task.is_finished(),
            "the close completed before it was asked for"
        );
        let waiting = polls.load(Ordering::SeqCst);
        assert!(
            waiting <= 3,
            "the wait for the initiator's Close was polled {waiting} times in 80 ms; \
             it must wait to be told, not retry on a timer"
        );
        let (closed, finished) = tokio::join!(wired.initiator.close(), task);
        closed.expect("the initiator's close result");
        finished.unwrap().expect("the worker completes the close");
        assert!(polls.load(Ordering::SeqCst) <= waiting + 3);
    });
}

#[test]
fn a_close_that_never_arrives_ends_the_wait_at_its_deadline_without_polling() {
    runtime().block_on(async {
        let wired = wire();
        both_ended(&wired).await;
        let began = Instant::now();
        let (task, polls) = closing(&wired, Duration::from_millis(80));
        let ended = task.await.unwrap();
        assert_eq!(ended, Err(ErrorCode::Timeout));
        assert!(began.elapsed() >= Duration::from_millis(80));
        let count = polls.load(Ordering::SeqCst);
        assert!(count <= 3, "an 80 ms wait was polled {count} times");
    });
}

#[test]
fn a_close_that_has_already_arrived_completes_on_the_first_poll() {
    runtime().block_on(async {
        let wired = wire();
        both_ended(&wired).await;
        let initiator = wired.initiator.clone();
        let asked = tokio::spawn(async move { initiator.close().await });
        // Let the Close travel before the worker asks.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let (task, polls) = closing(&wired, Duration::from_secs(10));
        task.await.unwrap().unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        asked.await.unwrap().unwrap();
    });
}
