//! The pipe wakes the client's pending read (Phase 1 Code P2-4, step R2b part D): the task's `deliver` and `end` and
//! the client's `abort` each wake it once, and a read after `abort` fails rather than waiting for an end that will not
//! come.
use super::{HttpsExchange, exchange};
use crate::git::endpoint::ssh_adapter::Settle;
use gwz_connection::{AuthContext, AuthScope, ExchangeIo, ExchangeSpec, GitService};
use gwz_transport::pool::{Identity, Key};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

struct Count(AtomicUsize);
impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Count {
    fn wakes(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
fn advertisement() -> (HttpsExchange, super::Work) {
    let spec = ExchangeSpec::git(
        Key::https("localhost", 443),
        Identity::Https,
        GitService::UploadPackAdvertisement,
        "/repo",
    )
    .unwrap();
    let auth = AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(()));
    exchange(&spec, &auth, Arc::new(Settle::default()))
}
/// Polls a read of `io` that finds nothing: it is pending and has not woken its waker.
fn pending_read(io: &mut HttpsExchange) -> Arc<Count> {
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let polled = io.poll_read(&mut Context::from_waker(&waker), &mut [0; 8]);
    assert!(polled.is_pending());
    assert_eq!(count.wakes(), 0, "a pending read woke itself");
    count
}
fn read(io: &mut HttpsExchange, out: &mut [u8]) -> Poll<io::Result<usize>> {
    io.poll_read(&mut Context::from_waker(Waker::noop()), out)
}

#[test]
fn r2b_a_pending_https_read_is_woken_by_deliver_end_and_abort() {
    let (mut io, work) = advertisement();
    let mut out = [0u8; 8];
    let woken = pending_read(&mut io);
    work.pipe.deliver(b"0000");
    assert_eq!(woken.wakes(), 1, "deliver wakes the read");
    assert!(matches!(read(&mut io, &mut out), Poll::Ready(Ok(4))));
    let woken = pending_read(&mut io);
    work.pipe.end(Ok(()));
    assert_eq!(woken.wakes(), 1, "the end wakes the read");
    assert!(matches!(read(&mut io, &mut out), Poll::Ready(Ok(0))));

    let (mut io, _work) = advertisement();
    let woken = pending_read(&mut io);
    io.abort();
    assert_eq!(woken.wakes(), 1, "abort wakes the read");
    match read(&mut io, &mut out) {
        Poll::Ready(Err(error)) => assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted),
        other => panic!("a read after abort ends: {other:?}"),
    }
}
