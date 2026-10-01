//! Opens streams through the worker the way the placement endpoint does, with
//! `start_endpoint_open`, and plays the driver's part for each attachment: an
//! initiator stream whose messages a thread carries to and from the worker,
//! read and written through a [`BlockingStream`] as Git does.
use crate::git::endpoint::{
    ssh_channel::GitService,
    ssh_worker::{BridgeContext, Endpoint, EndpointAttachment, PendingOpen, ThreadWake},
    stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{Deadlines, Envelope, MessageKind, Opened},
    stream::{Config as StreamConfig, Error, MessageEndpoint, Side, Stream},
};
use std::{
    future::Future,
    io,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

/// The deadlines of an open that asks for the endpoint's whole policy: its
/// pool's configured allocation, connect, interaction and cleanup budgets
/// and its I/O stall budget.
pub(super) fn deadlines(config: &Config, io_timeout_ms: u64) -> Deadlines {
    Deadlines {
        allocation_ms: config.allocation_timeout_ms as i64,
        connect_ms: config.connect_timeout_ms as i64,
        io_ms: io_timeout_ms as i64,
        interaction_ms: config.interaction_timeout_ms as i64,
        cleanup_ms: config.cleanup_timeout_ms as i64,
    }
}

/// The bridge of an open on one placement session's first stream.
pub(super) fn context(deadlines: Deadlines) -> BridgeContext {
    BridgeContext {
        session_id: "session".into(),
        stream_id: 1,
        version: 2,
        limits: gwz_transport::binding::default_limits(),
        deadlines,
        waker: None,
    }
}

/// Submits an open, with the identity file `selected` or else ambient
/// authority, and returns it without waiting for the worker's reply. The
/// worker refuses it here if it is stopped or its admission is full.
pub(super) fn start(
    endpoint: &Endpoint,
    key: Key,
    selected: Option<PathBuf>,
    service: GitService,
    path: &str,
    deadlines: Deadlines,
) -> io::Result<PendingOpen> {
    endpoint.start_endpoint_open(
        key,
        selected,
        service,
        path,
        context(deadlines),
        Arc::new(AtomicBool::new(false)),
    )
}

/// Waits for the worker's reply to an open.
pub(super) fn finish(open: &PendingOpen) -> io::Result<(EndpointAttachment, Opened)> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        if let Poll::Ready(result) = open.poll() {
            return result;
        }
        assert!(
            Instant::now() < until,
            "the worker never replied to the open"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// Opens a stream and drives it: the worker's reply, as a Git-facing stream.
pub(super) fn open(
    endpoint: &Endpoint,
    key: Key,
    selected: Option<PathBuf>,
    service: GitService,
    path: &str,
    deadlines: Deadlines,
) -> io::Result<(BlockingStream, Opened)> {
    let context = context(deadlines.clone());
    let pending = start(endpoint, key, selected, service, path, deadlines)?;
    let (attachment, opened) = finish(&pending)?;
    Ok((drive(attachment, &context), opened))
}

/// Carries an attachment's messages from a thread of its own, as the driver
/// session and the mux do, and returns the initiator's end as a stream.
pub(super) fn drive(attachment: EndpointAttachment, context: &BridgeContext) -> BlockingStream {
    // The same profile the worker gives the endpoint's end.
    let limits = &context.limits;
    let mut config = StreamConfig::new(&context.session_id, context.stream_id, Side::Initiator);
    config.profile_version = 2;
    config.receive_limits = limits.clone();
    config.peer_limits = limits.clone();
    config.receive_window = (limits.receive_window as usize).min(65_536);
    config.peer_receive_window = (limits.receive_window as usize).min(65_536);
    config.max_payload = (limits.data_payload as usize).min(16_384);
    let (stream, peer) = Stream::new(config).expect("initiator stream configuration");
    let peer = Arc::new(peer);
    thread::Builder::new()
        .name("gwz-test-attachment".into())
        .spawn(move || carry(attachment, peer))
        .expect("attachment thread");
    BlockingStream::new(stream)
}

type NextMessage = Pin<Box<dyn Future<Output = Result<Option<Envelope>, Error>> + Send>>;

/// Moves messages both ways until the worker ends the exchange or drops the
/// bridge, or the initiator fails.
fn carry(attachment: EndpointAttachment, peer: Arc<MessageEndpoint>) {
    let origin = Instant::now();
    let until = origin + Duration::from_secs(120);
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut next: Option<NextMessage> = None;
    let mut pending: Option<Envelope> = None;
    let mut outgoing_done = false;
    loop {
        peer.advance(origin.elapsed().as_millis() as u64);
        let mut moved = false;
        loop {
            match attachment.try_receive() {
                Ok(Some(message)) => {
                    let terminal =
                        matches!(message.kind, MessageKind::Closed | MessageKind::Failed);
                    let _ = peer.deliver(message);
                    if terminal {
                        // The worker released the exchange when it handed this over.
                        return;
                    }
                    moved = true;
                }
                Ok(None) => break,
                Err(_) => {
                    peer.disconnect();
                    return;
                }
            }
        }
        while !outgoing_done {
            if pending.is_none() {
                let future = next.get_or_insert_with(|| {
                    let peer = peer.clone();
                    Box::pin(async move { peer.next_message().await })
                });
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(Ok(Some(message))) if message.kind == MessageKind::Cancel => {
                        // As the placement endpoint does, the initiator's
                        // Cancel abandons the exchange; the worker never sees it.
                        attachment.cancel();
                        return;
                    }
                    Poll::Ready(Ok(Some(message))) => {
                        next = None;
                        pending = Some(message);
                    }
                    Poll::Ready(Ok(None)) => {
                        next = None;
                        outgoing_done = true;
                        break;
                    }
                    Poll::Ready(Err(_)) => {
                        attachment.cancel();
                        return;
                    }
                    Poll::Pending => break,
                }
            }
            let Some(message) = pending.take() else {
                break;
            };
            match attachment.send(message.clone()) {
                Ok(()) => moved = true,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    pending = Some(message);
                    break;
                }
                Err(_) => {
                    peer.disconnect();
                    return;
                }
            }
        }
        if Instant::now() >= until {
            attachment.cancel();
            peer.disconnect();
            return;
        }
        if !moved {
            thread::park_timeout(Duration::from_millis(1));
        }
    }
}
