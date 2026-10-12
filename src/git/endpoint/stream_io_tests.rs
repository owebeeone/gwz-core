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
