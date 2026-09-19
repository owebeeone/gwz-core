#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;

use gwz_transport::{
    protocol::{Disposition, Facts},
    protocol::{EndWrite, Envelope, MessageKind},
    stream::{Config, Error as StreamError, IoState, MessageEndpoint, Side, Stream},
};
use std::{
    future::Future,
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use stream_io::BlockingStream;

fn config(side: Side) -> Config {
    let mut config = Config::new("blocking-consumer", 1, side);
    config.send_buffer = 4;
    config.receive_window = 4;
    config.peer_receive_window = 4;
    config.max_payload = 2;
    config.coalesce_delay_ms = 10;
    config.io_timeout_ms = 3;
    config
}

fn pair() -> (Stream, MessageEndpoint, Stream, MessageEndpoint) {
    let (left, left_endpoint) = Stream::new(config(Side::Initiator)).unwrap();
    let (right, right_endpoint) = Stream::new(config(Side::Endpoint)).unwrap();
    (left, left_endpoint, right, right_endpoint)
}

fn poll_next(endpoint: &MessageEndpoint, context: &mut Context<'_>) -> bool {
    let mut next = Box::pin(endpoint.next_message());
    match next.as_mut().poll(context) {
        Poll::Ready(Ok(Some(_))) => true,
        Poll::Ready(Ok(None)) | Poll::Pending => false,
        Poll::Ready(Err(error)) => panic!("host message polling failed: {error:?}"),
    }
}

fn wait_for_waiter(endpoint: &MessageEndpoint) {
    for _ in 0..1_000 {
        if endpoint.waiter_count() > 0 {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("blocking operation did not register a stream waiter");
}

fn move_one(from: &MessageEndpoint, to: &MessageEndpoint, context: &mut Context<'_>) -> bool {
    let mut next = Box::pin(from.next_message());
    match next.as_mut().poll(context) {
        Poll::Ready(Ok(Some(message))) => {
            to.deliver(message).unwrap();
            true
        }
        Poll::Ready(Ok(None)) | Poll::Pending => false,
        Poll::Ready(Err(error)) => panic!("host message polling failed: {error:?}"),
    }
}

fn poll_peer_read(
    stream: &Stream,
    output: &mut [u8],
    context: &mut Context<'_>,
) -> Option<Result<usize, StreamError>> {
    let mut read = Box::pin(stream.read(output));
    match read.as_mut().poll(context) {
        Poll::Ready(result) => Some(result),
        Poll::Pending => None,
    }
}

fn source(error: &io::Error) -> Option<StreamError> {
    error
        .get_ref()
        .and_then(|value| value.downcast_ref::<StreamError>())
        .copied()
}

#[test]
fn small_write_waits_for_host_timer_and_exchange_handles_irregular_reads() {
    let (stream, endpoint, _peer, _peer_endpoint) = pair();
    let mut blocking = BlockingStream::new(stream);
    assert_eq!(blocking.write(b"x").unwrap(), 1);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(!poll_next(&endpoint, &mut context));
    endpoint.advance(9);
    assert!(!poll_next(&endpoint, &mut context));
    endpoint.advance(10);
    let mut next = Box::pin(endpoint.next_message());
    let message = match next.as_mut().poll(&mut context) {
        Poll::Ready(Ok(Some(message))) => message,
        other => panic!("timer did not emit the small write: {other:?}"),
    };
    assert_eq!(message.kind, MessageKind::Data);
    assert_eq!(message.data.unwrap().payload, b"x");

    let (stream, endpoint, peer, peer_endpoint) = pair();
    let host_done = Arc::new(AtomicBool::new(false));
    let expected = b"0123456789abcdef".to_vec();
    let expected_for_host = expected.clone();
    let host_done_for_host = host_done.clone();
    let close_done = Arc::new(AtomicBool::new(false));
    let close_done_for_host = close_done.clone();
    let host = thread::spawn(move || {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut received = Vec::new();
        let mut peer_end_write = Box::pin(peer.end_write());
        let mut peer_end_done = false;
        for tick in 0..5000 {
            move_one(&endpoint, &peer_endpoint, &mut context);
            move_one(&peer_endpoint, &endpoint, &mut context);
            let mut chunk = [0; 3];
            if let Some(Ok(count)) = poll_peer_read(&peer, &mut chunk, &mut context) {
                received.extend_from_slice(&chunk[..count]);
            }
            if !peer_end_done {
                if let Poll::Ready(result) = peer_end_write.as_mut().poll(&mut context) {
                    result.unwrap();
                    peer_end_done = true;
                }
            }
            if !close_done_for_host.load(Ordering::Acquire) {
                match peer_endpoint.complete_close(Disposition::Reusable, Facts::default()) {
                    Ok(()) => {
                        close_done_for_host.store(true, Ordering::Release);
                    }
                    Err(StreamError::WouldBlock) | Err(StreamError::WrongState) => {}
                    Err(error) => panic!("host close cleanup failed: {error:?}"),
                }
            }
            move_one(&peer_endpoint, &endpoint, &mut context);
            endpoint.advance(tick);
            peer_endpoint.advance(tick);
            if host_done_for_host.load(Ordering::Acquire)
                && close_done_for_host.load(Ordering::Acquire)
                && received == expected_for_host
            {
                return received;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("host did not drain bounded exchange");
    });
    let (operation_sender, operation_receiver) = mpsc::channel();
    let operation_expected = expected.clone();
    let operation = thread::spawn(move || {
        let mut blocking = BlockingStream::new(stream);
        let result = blocking
            .write_all(&operation_expected)
            .and_then(|()| blocking.end_write())
            .and_then(|()| blocking.close());
        operation_sender.send(result).unwrap();
    });
    let close_result = operation_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("blocking exchange did not complete within its bound")
        .unwrap();
    assert_eq!(close_result.disposition, Disposition::Reusable);
    host_done.store(true, Ordering::Release);
    operation.join().unwrap();
    assert_eq!(host.join().unwrap(), expected);
}

#[test]
fn cancellation_and_endpoint_loss_wake_blocked_std_io() {
    let (stream, endpoint, _peer, _peer_endpoint) = pair();
    let control = BlockingStream::new(stream.clone());
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut blocking = BlockingStream::new(stream);
        sender.send(blocking.write_all(&[7; 128])).unwrap();
    });
    wait_for_waiter(&endpoint);
    control.cancel();
    let error = receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert_eq!(source(&error), Some(StreamError::Cancelled));

    let (stream, endpoint, _peer, _peer_endpoint) = pair();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut blocking = BlockingStream::new(stream);
        let mut output = [0; 1];
        sender.send(blocking.read(&mut output)).unwrap();
    });
    wait_for_waiter(&endpoint);
    endpoint.disconnect();
    let error = receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(source(&error), Some(StreamError::CarrierLost));
}

#[test]
fn eof_and_network_timeout_keep_distinct_io_errors() {
    let (stream, endpoint, _peer, _peer_endpoint) = pair();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut blocking = BlockingStream::new(stream);
        let mut output = [0; 1];
        sender.send(blocking.read(&mut output)).unwrap();
    });
    wait_for_waiter(&endpoint);
    endpoint
        .deliver(Envelope {
            version: 1,
            session_id: "blocking-consumer".into(),
            stream_id: 1,
            kind: MessageKind::EndWrite,
            end_write: Some(EndWrite { final_offset: 0 }),
            ..Envelope::default()
        })
        .unwrap();
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap(),
        0
    );

    let (stream, endpoint) = Stream::new(config(Side::Endpoint)).unwrap();
    endpoint.set_io_state(IoState::Network).unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut blocking = BlockingStream::new(stream);
        let mut output = [0; 1];
        sender.send(blocking.read(&mut output)).unwrap();
    });
    wait_for_waiter(&endpoint);
    endpoint.advance(3);
    let error = receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(source(&error), Some(StreamError::Timeout));
}
