//! Integration proof: the generated host field carries typed values directly.
//! The bounded in-memory handoff also replays the same exchange through the
//! generated CBOR codec without adding physical framing.
use gwz_transport::{
    binding, codec,
    protocol::{Disposition, Effect, Envelope, ErrorCode, Facts, Failure, MessageKind},
    stream::{Config, Error, IoState, MessageEndpoint, Side, Stream},
};
use gwz_transport_consumer_proof::{cbor, generated::GwzTransportDelivery};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn handoff(
    from: &MessageEndpoint,
    to: &MessageEndpoint,
    cx: &mut Context<'_>,
    serialized: bool,
) -> bool {
    match pin!(from.next_message()).poll(cx) {
        Poll::Ready(Ok(Some(message))) => {
            to.deliver(generated_message(message, serialized)).unwrap();
            true
        }
        Poll::Pending | Poll::Ready(Ok(None)) => false,
        Poll::Ready(Err(error)) => {
            panic!("delivery failed: {error}");
        }
    }
}

fn generated_message(message: Envelope, serialized: bool) -> Envelope {
    let envelope = GwzTransportDelivery { message };
    if !serialized {
        return envelope.message;
    }
    // The wrapper is a trusted generated fixture for this test; the supplied
    // outer carrier owns pre-allocation bounds.
    let bytes = cbor::encode(&envelope.to_cbor());
    assert!(bytes.len() <= binding::default_limits().encoded_frame as usize);
    let decoded = cbor::try_decode(&bytes).expect("serialized message must decode");
    let envelope =
        GwzTransportDelivery::from_cbor(&decoded).expect("generated consumer codec must decode");
    let limits = binding::default_limits();
    let inner = codec::encode_limited(&envelope.message, &limits)
        .expect("owner codec must encode bounded message");
    GwzTransportDelivery {
        message: codec::decode_limited(&inner, &limits)
            .expect("owner codec must decode bounded message"),
    }
    .message
}

async fn send(stream: &Stream, input: &[u8]) -> Result<(), Error> {
    stream.write_all(input).await?;
    stream.end_write().await
}

fn exchange(serialized: bool) {
    let mut config = Config::new("consumer-memory", 1, Side::Initiator);
    config.send_buffer = 3;
    config.receive_window = 2;
    config.peer_receive_window = 2;
    config.max_payload = 2;
    let mut peer = config.clone();
    peer.side = Side::Endpoint;
    let (a, port_a) = Stream::new(config).unwrap();
    let (b, port_b) = Stream::new(peer).unwrap();
    let a_input = [0, 255, 13, 10, 1, 127, 128];
    let b_input = *b"reverse direction";
    let mut a_write = pin!(send(&a, &a_input));
    let mut b_write = pin!(send(&b, &b_input));
    let mut a_close = pin!(a.close());
    let mut writes_done = [false; 2];
    let mut eof = [false; 2];
    let mut cleanup_done = false;
    let mut close_done = false;
    let mut output: [Vec<u8>; 2] = Default::default();
    let mut cx = Context::from_waker(Waker::noop());
    for tick in 0..1000 {
        for (index, writer) in [a_write.as_mut(), b_write.as_mut()].into_iter().enumerate() {
            if !writes_done[index]
                && let Poll::Ready(result) = writer.poll(&mut cx)
            {
                result.unwrap();
                writes_done[index] = true;
            }
        }
        handoff(&port_a, &port_b, &mut cx, serialized);
        handoff(&port_b, &port_a, &mut cx, serialized);
        for (index, stream) in [&a, &b].into_iter().enumerate() {
            if eof[index] {
                continue;
            }
            let mut bytes = [0; 3];
            let result = { pin!(stream.read(&mut bytes)).poll(&mut cx) };
            if let Poll::Ready(result) = result {
                let count = result.unwrap();
                eof[index] |= count == 0;
                output[index].extend_from_slice(&bytes[..count]);
            }
        }
        if eof[1] && !cleanup_done {
            match port_b.complete_close(Disposition::Reusable, Facts::default()) {
                Ok(()) => cleanup_done = true,
                Err(Error::WouldBlock) => {}
                Err(error) => panic!("endpoint cleanup failed: {error:?}"),
            }
        }
        if writes_done == [true; 2] && eof == [true; 2] {
            if !close_done && let Poll::Ready(result) = a_close.as_mut().poll(&mut cx) {
                assert!(cleanup_done, "close completed before endpoint cleanup");
                let result = result.expect("graceful close must complete");
                assert_eq!(result.disposition, Disposition::Reusable);
                assert_eq!(result.facts, Facts::default());
                assert!(!result.unread_response_discarded);
                close_done = true;
            }
        }
        port_a.advance(tick);
        port_b.advance(tick);
        if writes_done == [true; 2] && eof == [true; 2] && cleanup_done && close_done {
            assert_eq!(output[0], b_input);
            assert_eq!(output[1], a_input);
            return;
        }
    }
    panic!("in-memory stream exchange did not finish");
}

fn small_pair() -> (Stream, MessageEndpoint, Stream, MessageEndpoint) {
    let mut config = Config::new("consumer-contract", 1, Side::Initiator);
    config.send_buffer = 4;
    config.receive_window = 4;
    config.peer_receive_window = 4;
    config.max_payload = 2;
    let mut peer = config.clone();
    peer.side = Side::Endpoint;
    let (a, port_a) = Stream::new(config).unwrap();
    let (b, port_b) = Stream::new(peer).unwrap();
    (a, port_a, b, port_b)
}

#[test]
fn flush_consumption_and_ordered_half_close_survive_both_handoffs() {
    for serialized in [false, true] {
        let (a, port_a, b, port_b) = small_pair();
        let mut cx = Context::from_waker(Waker::noop());
        let mut write = Box::pin(a.write_all(b"abc"));
        let mut write_done = false;
        let mut flush = None;
        let mut flush_done = false;
        let mut end_write = None;
        let mut eof = false;
        let mut received = Vec::new();
        for tick in 0..1000 {
            if !write_done {
                if let Poll::Ready(result) = write.as_mut().poll(&mut cx) {
                    result.unwrap();
                    write_done = true;
                }
            }
            if write_done && flush.is_none() {
                flush = Some(Box::pin(a.flush()));
            }
            handoff(&port_a, &port_b, &mut cx, serialized);
            handoff(&port_b, &port_a, &mut cx, serialized);
            let mut bytes = [0; 1];
            if !eof {
                let result = { pin!(b.read(&mut bytes)).poll(&mut cx) };
                match result {
                    Poll::Ready(Ok(0)) => {
                        eof = true;
                    }
                    Poll::Ready(Ok(count)) => {
                        received.extend_from_slice(&bytes[..count]);
                    }
                    Poll::Ready(Err(error)) => panic!("read failed: {error:?}"),
                    Poll::Pending => {}
                }
            }
            if let Some(flush) = flush.as_mut() {
                if !flush_done {
                    if let Poll::Ready(result) = flush.as_mut().poll(&mut cx) {
                        result.unwrap();
                        flush_done = true;
                    }
                }
            }
            if flush_done && end_write.is_none() {
                end_write = Some(Box::pin(a.end_write()));
            }
            if let Some(end_write_future) = end_write.as_mut() {
                if let Poll::Ready(result) = end_write_future.as_mut().poll(&mut cx) {
                    result.unwrap();
                    end_write = None;
                }
            }
            port_a.advance(tick);
            port_b.advance(tick);
            if flush_done && end_write.is_none() && eof {
                break;
            }
        }
        assert!(write_done && flush_done && eof);
        assert_eq!(received, b"abc");
    }
}

#[test]
fn close_discards_unread_reverse_data_after_request_cleanup_in_both_handoffs() {
    for serialized in [false, true] {
        let (a, port_a, b, port_b) = small_pair();
        let mut cx = Context::from_waker(Waker::noop());
        let mut request = Box::pin(a.write_all(b"req"));
        assert!(matches!(
            request.as_mut().poll(&mut cx),
            Poll::Ready(Ok(()))
        ));
        port_a.advance(100);
        assert!(handoff(&port_a, &port_b, &mut cx, serialized));
        let mut request_bytes = Vec::new();
        let mut request_chunk = [0; 3];
        let result = { pin!(b.read(&mut request_chunk)).poll(&mut cx) };
        if let Poll::Ready(Ok(count)) = result {
            request_bytes.extend_from_slice(&request_chunk[..count]);
        }
        while request_bytes.len() < b"req".len() {
            assert!(handoff(&port_a, &port_b, &mut cx, serialized));
            let result = { pin!(b.read(&mut request_chunk)).poll(&mut cx) };
            if let Poll::Ready(Ok(count)) = result {
                request_bytes.extend_from_slice(&request_chunk[..count]);
            }
        }
        assert_eq!(request_bytes, b"req");
        let mut close = Some(Box::pin(a.close()));
        let mut response = Some(Box::pin(b.write_all(b"response")));
        let mut end_write = None;
        let mut cleanup_done = false;
        let mut close_result = None;
        for tick in 0..1000 {
            if let Some(response_future) = response.as_mut() {
                if let Poll::Ready(result) = response_future.as_mut().poll(&mut cx) {
                    result.unwrap();
                    response = None;
                    if end_write.is_none() {
                        end_write = Some(Box::pin(b.end_write()));
                    }
                }
            }
            if let Some(end_write_future) = end_write.as_mut() {
                if let Poll::Ready(result) = end_write_future.as_mut().poll(&mut cx) {
                    result.unwrap();
                    end_write = None;
                }
            }
            handoff(&port_a, &port_b, &mut cx, serialized);
            handoff(&port_b, &port_a, &mut cx, serialized);
            if request_bytes != b"req" {
                let mut bytes = [0; 1];
                let result = { pin!(b.read(&mut bytes)).poll(&mut cx) };
                match result {
                    Poll::Ready(Ok(count)) => request_bytes.extend_from_slice(&bytes[..count]),
                    Poll::Ready(Err(error)) => panic!("request read failed: {error:?}"),
                    Poll::Pending => {}
                }
            }
            if request_bytes == b"req" && !cleanup_done {
                match port_b.complete_close(Disposition::Reusable, Facts::default()) {
                    Ok(()) => cleanup_done = true,
                    Err(Error::WouldBlock) => {}
                    Err(error) => panic!("endpoint cleanup failed: {error:?}"),
                }
            }
            if let Some(close_future) = close.as_mut() {
                if let Poll::Ready(result) = close_future.as_mut().poll(&mut cx) {
                    close_result = Some(result.unwrap());
                    close = None;
                }
            }
            port_a.advance(tick);
            port_b.advance(tick);
            if cleanup_done && close_result.is_some() {
                break;
            }
        }
        let result = close_result.expect("close must complete after cleanup");
        assert_eq!(request_bytes, b"req");
        assert!(result.unread_response_discarded);
        assert_eq!(result.disposition, Disposition::Reusable);
    }
}

#[test]
fn cancellation_preserves_received_prefix_and_first_terminal_in_both_handoffs() {
    for serialized in [false, true] {
        let (a, port_a, b, port_b) = small_pair();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            pin!(a.write(b"first")).poll(&mut cx),
            Poll::Ready(Ok(4))
        ));
        assert!(handoff(&port_a, &port_b, &mut cx, serialized));
        assert!(handoff(&port_a, &port_b, &mut cx, serialized));
        a.cancel();
        assert!(handoff(&port_a, &port_b, &mut cx, serialized));
        assert!(!handoff(&port_a, &port_b, &mut cx, serialized));
        let late_terminal = generated_message(
            Envelope {
                version: 1,
                session_id: "consumer-contract".into(),
                stream_id: 1,
                kind: MessageKind::Failed,
                failed: Some(Failure {
                    code: ErrorCode::Io,
                    effect: Effect::Possible,
                    facts: None,
                }),
                ..Default::default()
            },
            serialized,
        );
        port_b.deliver(late_terminal).unwrap();
        let mut prefix = Vec::new();
        let mut bytes = [0; 2];
        assert_eq!(pin!(b.read(&mut bytes)).poll(&mut cx), Poll::Ready(Ok(2)));
        prefix.extend_from_slice(&bytes);
        assert_eq!(pin!(b.read(&mut bytes)).poll(&mut cx), Poll::Ready(Ok(2)));
        prefix.extend_from_slice(&bytes);
        assert_eq!(
            pin!(b.read(&mut bytes)).poll(&mut cx),
            Poll::Ready(Err(Error::Cancelled))
        );
        assert_eq!(prefix, b"firs");
    }
}

#[test]
fn carrier_loss_wakes_a_pending_reader_after_generated_handoff_in_both_modes() {
    for serialized in [false, true] {
        let (a, port_a, b, port_b) = small_pair();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            pin!(a.write(b"x")).poll(&mut cx),
            Poll::Ready(Ok(1))
        ));
        port_a.advance(100);
        assert!(handoff(&port_a, &port_b, &mut cx, serialized));
        let mut bytes = [0; 1];
        assert_eq!(pin!(b.read(&mut bytes)).poll(&mut cx), Poll::Ready(Ok(1)));
        let mut pending = pin!(b.read(&mut bytes));
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        drop(port_b);
        assert_eq!(
            pending.as_mut().poll(&mut cx),
            Poll::Ready(Err(Error::CarrierLost))
        );
    }
}

#[test]
fn endpoint_io_deadline_is_delivered_as_typed_failure_in_both_modes() {
    for serialized in [false, true] {
        let (a, port_a, b, port_b) = small_pair();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            pin!(b.write(b"prefix")).poll(&mut cx),
            Poll::Ready(Ok(4))
        ));
        port_b.advance(100);
        assert!(handoff(&port_b, &port_a, &mut cx, serialized));
        let mut bytes = [0; 2];
        let result = { pin!(a.read(&mut bytes)).poll(&mut cx) };
        assert!(matches!(result, Poll::Ready(Ok(count)) if count > 0));

        port_b.advance(4_000_000);
        port_b.set_io_state(IoState::Network).unwrap();
        let deadline = port_b.next_deadline().expect("network deadline");
        port_b.advance(deadline - 1);
        assert!(!port_b.stats().terminal);
        port_b.advance(deadline);
        assert!(handoff(&port_b, &port_a, &mut cx, serialized));
        assert_eq!(
            { pin!(a.read(&mut bytes)).poll(&mut cx) },
            Poll::Ready(Err(Error::PeerFailed {
                code: ErrorCode::Timeout,
                effect: Effect::Possible,
            }))
        );
    }
}

#[test]
fn async_file_streams_exchange_shared_typed_messages_in_both_directions() {
    exchange(false);
}

#[test]
fn async_file_streams_exchange_shared_serialized_messages_in_both_directions() {
    exchange(true);
}
