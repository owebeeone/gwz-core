//! Integration proof: the generated host field carries typed values directly.
//! The bounded in-memory handoff also replays the same exchange through the
//! generated CBOR codec without adding physical framing.
use gwz_transport::{
    binding, codec,
    protocol::{Disposition, Facts},
    stream::{Config, Error, MessageEndpoint, Side, Stream},
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
            let delivery = if serialized {
                let envelope = GwzTransportDelivery { message };
                // The wrapper is a trusted generated fixture for this test;
                // the supplied outer carrier owns pre-allocation bounds.
                let bytes = cbor::encode(&envelope.to_cbor());
                assert!(bytes.len() <= binding::default_limits().encoded_frame as usize);
                let decoded = cbor::try_decode(&bytes).expect("serialized message must decode");
                let envelope = GwzTransportDelivery::from_cbor(&decoded)
                    .expect("generated consumer codec must decode");
                let limits = binding::default_limits();
                let inner = codec::encode_limited(&envelope.message, &limits)
                    .expect("owner codec must encode bounded message");
                let message = codec::decode_limited(&inner, &limits)
                    .expect("owner codec must decode bounded message");
                GwzTransportDelivery { message }
            } else {
                GwzTransportDelivery { message }
            };
            to.deliver(delivery.message).unwrap();
            true
        }
        Poll::Pending | Poll::Ready(Ok(None)) => false,
        Poll::Ready(Err(error)) => {
            panic!("delivery failed: {error}");
        }
    }
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

#[test]
fn async_file_streams_exchange_shared_typed_messages_in_both_directions() {
    exchange(false);
}

#[test]
fn async_file_streams_exchange_shared_serialized_messages_in_both_directions() {
    exchange(true);
}
