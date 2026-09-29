#![cfg(test)]
//! The byte-stream adapter's body guard, beyond the shared suite: a body is
//! wiped on every path but delivery (the module's documentation; the crate
//! map steps' Safety review, P3-1).

use std::io::{self, ErrorKind, Read};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Weak};

use super::{Body, ByteStreamEnd, read_frame};
use crate::byte_stream;
use gwz_session_contract::{Closed, FrameSink, FrameSource};

#[test]
fn a_bodys_wipe_overwrites_the_allocation_that_held_it() {
    // A buffer that held a secret, which now also sits in its spare capacity,
    // as it does in one a stream filled and the guard then shortened.
    let mut buffer = Vec::with_capacity(64);
    buffer.resize(buffer.capacity(), b'#');
    buffer[..12].copy_from_slice(b"s3cr3t-token");
    buffer.truncate(12);
    let (held, capacity) = (buffer.as_ptr(), buffer.capacity());
    let mut body = Body(buffer);
    // What its drop does, stopped before the free.
    body.wipe();
    assert_eq!(body.0.as_ptr(), held, "the allocation that held the body");
    assert_eq!(body.0.capacity(), capacity, "no reallocation");
    assert_eq!(body.0.len(), capacity, "the spare capacity included");
    assert!(body.0.iter().all(|&byte| byte == 0));
    // Delivery takes the body out, and leaves nothing to wipe.
    let mut delivered = Body(b"delivered".to_vec());
    delivered.0.shrink_to_fit();
    assert_eq!(delivered.into_inner(), b"delivered");
}

/// Serves a frame's head, a length prefix of 9 and tag 4 (`SessionOpen`,
/// whose body can carry the snapshot), then four bytes of its eight-byte body,
/// and then ends, fails or panics, as `after` says.
struct PartBody {
    bytes: &'static [u8],
    after: Exit,
}

#[derive(Clone, Copy, Debug)]
enum Exit {
    Ends,
    Fails,
    Panics,
}

impl Read for PartBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.bytes.is_empty() {
            return match self.after {
                Exit::Ends => Ok(0),
                Exit::Fails => Err(ErrorKind::ConnectionReset.into()),
                Exit::Panics => panic!("the stream fails in the middle of a body"),
            };
        }
        let length = buffer.len().min(self.bytes.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

#[test]
fn a_body_read_in_part_is_owned_by_its_guard_on_every_exit() {
    // Structural: the caller's guard is the buffer's only owner. However the
    // read leaves `read_frame`, by returning or by unwinding, the buffer is
    // still in the guard, at the size the prefix stated and holding what was
    // read, and nothing else holds it. `recv` owns the guard, so its drop
    // wipes the body on each of these exits.
    for after in [Exit::Ends, Exit::Fails, Exit::Panics] {
        let mut stream = PartBody {
            bytes: &[9, 0, 0, 0, 4, b's', b'3', b'c', b'r'],
            after,
        };
        let mut body = Body::default();
        let exit = panic::catch_unwind(AssertUnwindSafe(|| read_frame(&mut stream, &mut body)));
        match (after, exit) {
            (Exit::Ends, Ok(result)) => {
                assert_eq!(result, Err(Closed::Stream(ErrorKind::UnexpectedEof)));
            }
            (Exit::Fails, Ok(result)) => {
                assert_eq!(result, Err(Closed::Stream(ErrorKind::ConnectionReset)));
            }
            (Exit::Panics, Err(_)) => {}
            (after, exit) => panic!("{after:?} left the read as {exit:?}"),
        }
        assert_eq!(body.0.len(), 8, "{after:?}: the buffer the prefix sized");
        assert_eq!(
            &body.0[..4],
            b"s3cr",
            "{after:?}: what was read, in the guard"
        );
    }
}

/// Serves `bytes`, and closes its own end just before it serves the last of
/// them: a close that lands while `recv` is inside a frame.
struct ClosesMidFrame {
    end: Weak<ByteStreamEnd<ClosesMidFrame, io::Sink>>,
    bytes: &'static [u8],
}

impl Read for ClosesMidFrame {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(self.bytes.len());
        if length == self.bytes.len()
            && let Some(end) = self.end.upgrade()
        {
            end.close();
        }
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

#[test]
fn a_frame_read_while_the_session_ends_here_is_discarded_whole() {
    // The whole frame arrives, but the session ended here while it was read,
    // so `recv` discards it. Its body never left the guard, which wipes it.
    let end = Arc::new_cyclic(|end| {
        byte_stream(
            ClosesMidFrame {
                end: end.clone(),
                bytes: &[5, 0, 0, 0, 4, b's', b'3', b'c', b'r'],
            },
            io::sink(),
        )
    });
    assert_eq!(end.recv(), Err(Closed::Local));
    assert_eq!(end.recv(), Err(Closed::Local), "the end stays closed");
}
