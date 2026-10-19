//! The stall wording of a blocking stream (T7); the adapter itself is `stream_io`.

use super::stream_io::*;
use gwz_transport::protocol::{Effect, ErrorCode};
use gwz_transport::stream::Error;
use std::io;

fn peer_timeout() -> Error {
    Error::PeerFailed {
        code: ErrorCode::Timeout,
        effect: Effect::Possible,
    }
}

#[test]
fn a_stalled_read_names_the_flag_and_its_value() {
    for error in [Error::Timeout, peer_timeout()] {
        let reported = io_error(error, 9_000);
        assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            reported.to_string(),
            "no progress for 9 seconds (--ssh-timeout 9)"
        );
        // The stream's own error stays reachable as the source.
        let stall = reported.get_ref().unwrap();
        assert!(
            stall
                .source()
                .is_some_and(|source| source.downcast_ref::<Error>() == Some(&error))
        );
    }
}

#[test]
fn a_stalled_read_names_a_value_other_than_the_default() {
    let reported = io_error(Error::Timeout, 45_000);
    assert_eq!(
        reported.to_string(),
        "no progress for 45 seconds (--ssh-timeout 45)"
    );
}

#[test]
fn no_stall_message_names_a_zero_bound() {
    // `--ssh-timeout 0` turns the stall clock off, so a timeout that reaches
    // a stream opened under it, or opened with no bound recorded, is not
    // worded as a stall.
    for error in [Error::Timeout, peer_timeout()] {
        let reported = io_error(error, 0);
        assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
        assert!(!reported.to_string().contains("--ssh-timeout"));
        assert!(!reported.to_string().contains("no progress"));
    }
}

#[test]
fn only_a_timeout_is_worded_as_a_stall() {
    for (error, kind) in [
        (Error::Cancelled, io::ErrorKind::ConnectionAborted),
        (Error::CarrierLost, io::ErrorKind::BrokenPipe),
        (Error::Protocol, io::ErrorKind::InvalidData),
    ] {
        let reported = io_error(error, 9_000);
        assert_eq!(reported.kind(), kind);
        assert!(!reported.to_string().contains("--ssh-timeout"));
    }
    let other = Error::PeerFailed {
        code: ErrorCode::Protocol,
        effect: Effect::None,
    };
    assert!(!io_error(other, 9_000).to_string().contains("--ssh-timeout"));
}

// The producer gate (GwzPushProducerBusyDesign §2): whether the thread that
// owns a stream is outside every call on it.
mod producer {
    use super::super::stream_io::{BlockingStream, ProducerGate};
    use gwz_transport::stream::{Config, MessageEndpoint, Side, Stream};
    use std::{
        io::{Read, Write},
        sync::mpsc,
        thread,
        time::Duration,
    };

    /// An initiator stream behind a gate. The endpoint half must stay alive: dropping it disconnects the stream.
    fn gated() -> (BlockingStream, ProducerGate, MessageEndpoint) {
        let (stream, peer) = Stream::new(Config::new("gate-session", 1, Side::Initiator)).unwrap();
        let gate = ProducerGate::new(600_000);
        let stream = BlockingStream::new(stream).with_producer(Some(gate.clone()));
        (stream, gate, peer)
    }

    /// A thread blocked in `call` for as long as the stream lets it, and what it returned.
    fn blocked_in<T: Send + 'static>(
        stream: &BlockingStream,
        call: impl FnOnce(&mut BlockingStream) -> T + Send + 'static,
    ) -> (thread::JoinHandle<()>, mpsc::Receiver<T>) {
        let mut stream = stream.clone();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = sender.send(call(&mut stream));
        });
        // Long enough for the thread to be inside its call: the call does not return by itself.
        thread::sleep(Duration::from_millis(300));
        (handle, receiver)
    }

    #[test]
    fn a_gate_no_call_has_returned_from_is_never_busy() {
        let gate = ProducerGate::new(1);
        assert!(!gate.busy(), "nothing has been called yet");
        let guard = gate.enter();
        assert!(!gate.busy(), "inside a call");
        drop(guard);
        assert!(gate.busy(), "a call has returned and none is in flight");
        assert_eq!(gate.ceiling_ms(), 1);
    }

    #[test]
    fn a_stream_with_no_gate_has_no_calls_to_count() {
        let (stream, peer) = Stream::new(Config::new("gate-session", 1, Side::Initiator)).unwrap();
        let mut stream = BlockingStream::new(stream);
        assert_eq!(stream.write(b"x").unwrap(), 1);
        drop(peer);
    }

    #[test]
    fn the_owner_is_busy_between_calls_and_not_in_a_blocked_read() {
        let (mut stream, gate, _peer) = gated();
        assert!(!gate.busy());
        assert_eq!(stream.write(b"x").unwrap(), 1);
        assert!(gate.busy(), "a write has returned");
        let (handle, result) = blocked_in(&stream, |stream| stream.read(&mut [0; 8]));
        assert!(!gate.busy(), "the read is waiting for the server");
        stream.cancel();
        assert!(
            result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
        handle.join().unwrap();
        assert!(gate.busy(), "the call returned an error and ended with it");
    }

    #[test]
    fn the_owner_is_not_busy_in_a_write_the_stream_cannot_take() {
        let (stream, gate, _peer) = gated();
        let (handle, result) = blocked_in(&stream, |stream| stream.write_all(&vec![0; 4 << 20]));
        assert!(!gate.busy(), "the send buffer is full and the write waits");
        stream.cancel();
        assert!(
            result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
        handle.join().unwrap();
        assert!(gate.busy());
    }

    #[test]
    fn the_owner_is_not_busy_in_a_close_waiting_for_the_endpoint() {
        let (mut stream, gate, _peer) = gated();
        assert_eq!(stream.write(b"x").unwrap(), 1);
        assert!(gate.busy());
        let (handle, result) = blocked_in(&stream, |stream| stream.close().map(|_| ()));
        assert!(!gate.busy(), "the close waits for the endpoint's cleanup");
        stream.cancel();
        assert!(
            result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
        handle.join().unwrap();
        assert!(gate.busy());
    }

    #[test]
    fn the_owner_is_not_busy_in_an_end_write_the_endpoint_has_not_taken() {
        let (mut stream, gate, _peer) = gated();
        assert_eq!(stream.write(b"x").unwrap(), 1);
        assert!(gate.busy());
        let (handle, result) = blocked_in(&stream, |stream| stream.end_write());
        assert!(!gate.busy(), "the half-close waits to be sent");
        stream.cancel();
        assert!(
            result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
        handle.join().unwrap();
        assert!(gate.busy());
    }

    #[test]
    fn a_call_that_unwinds_ends_with_it() {
        let gate = ProducerGate::new(1);
        let owner = gate.clone();
        let outcome = std::panic::catch_unwind(move || {
            let _call = owner.enter();
            panic!("the producer's callback failed");
        });
        assert!(outcome.is_err());
        assert!(gate.busy(), "the unwound call is not still in flight");
    }

    #[test]
    fn clones_of_a_stream_share_one_gate() {
        let (mut stream, gate, _peer) = gated();
        let mut clone = stream.clone();
        assert_eq!(clone.write(b"x").unwrap(), 1);
        assert!(gate.busy(), "a call on the clone counted");
        let (handle, result) = blocked_in(&stream, |stream| stream.read(&mut [0; 8]));
        assert!(!gate.busy(), "a call on one clone is a call on the stream");
        clone.cancel();
        let _ = result.recv_timeout(Duration::from_secs(10)).unwrap();
        handle.join().unwrap();
        assert_eq!(
            stream.write(b"y").is_err(),
            true,
            "the stream was cancelled"
        );
    }
}
