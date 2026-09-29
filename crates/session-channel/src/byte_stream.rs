//! The byte-stream adapter (§3): frames over any reader and writer, such as
//! standard streams, a socket or SSH.
//!
//! On the stream, each frame is its size as a little-endian `u32`, then its
//! tag byte, then its body, exactly as taut-shape's interop tool frames it
//! (`taut-shape-tool`'s `framing.rs`). The size counts the tag byte and the
//! body, and is at most 64 MiB.
//!
//! A body can carry the environment snapshot (the server design's
//! `SessionOpen`), so `recv` reads it into a guard, `Body`, that owns the
//! buffer until the frame is delivered. The guard's drop overwrites the whole
//! allocation with zeros, spare capacity included, without reallocating. So
//! every exit of a read but delivery wipes the body:
//! - a stream that ends inside the frame;
//! - one that fails;
//! - one that panics mid-read, whose unwinding drops the guard;
//! - a frame read whole while the session was ending here, which `recv`
//!   discards.
//!
//! The wipe is safe code, since the crate forbids unsafe code. After the
//! writes, `std::hint::black_box` is std's best-effort barrier against the
//! compiler removing them, weaker than the volatile writes and fence with
//! which gwz-core wipes its snapshot. A delivered body is the caller's: core's
//! decoder owns its wipe.

// Ordinary stream I/O in a channel adapter: this crate is outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml), whose disallowed writers exist
// to route merge artifact mutation through checked entries.
#![allow(clippy::disallowed_methods)]

use std::fmt;
use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use gwz_session_contract::{Carrier, Closed, Frame, FrameSink, FrameSource, Lane, SendError};

/// One end of a byte-stream channel, reading frames from `read` and writing
/// them to `write`. The other end is whoever holds the stream's far side,
/// usually another process.
pub fn byte_stream<R: Read, W: Write>(read: R, write: W) -> ByteStreamEnd<R, W> {
    ByteStreamEnd {
        reader: Mutex::new(Some(read)),
        writer: Mutex::new(Some(write)),
        ended: Mutex::new(None),
    }
}

/// One end of a byte-stream channel.
///
/// - The carrier is [`Carrier::ByteStream`]: tags 1 to 8.
/// - `recv` reads one frame. It checks the length prefix before it reads
///   on: a prefix of zero, or over 64 MiB, ends the session before anything
///   is allocated. It then checks the tag before it allocates or reads the
///   body. A tag outside the registry ends the session too. The body's buffer
///   is allocated once, at the size the prefix states, so no partial copy of
///   a body is left behind, and it is wiped before it is freed on every path
///   but delivery (the module's documentation).
/// - End of stream between frames is [`Closed::Peer`]. Inside a frame, and
///   on any failure of the stream other than an interruption, the session
///   ends with [`Closed::Stream`].
/// - `send` writes the whole frame and flushes it, holding the writer so
///   that frames from several threads never interleave. It waits while the
///   writer does. The lane is not used: a byte stream has no queue to bound,
///   and it writes every frame in call order. A frame under a tag outside
///   the registry, or over 64 MiB, ends the session unwritten.
/// - When the session ends at this end, by `close`, a protocol error or a
///   failed stream, the end drops its writer, so that the peer reads end of
///   stream after the frames sent before, and it drops its reader. Neither
///   is dropped while a call is using it: the call drops it on return.
///
/// A `send` or `recv` already waiting inside the stream stays there until the
/// stream returns, which usually happens once the peer closes in turn. Only
/// the stream's owner can force it, by shutting a socket down or killing a
/// child. Dropping a half must end the peer's stream: a pipe's half does,
/// but a cloned socket's half does not while its other clone lives, so its
/// owner passes halves that shut the socket down when dropped.
///
/// An end is `Send` and `Sync` when its halves are `Send`: one thread may wait
/// in `recv` while others send. Dropping the end drops both halves.
pub struct ByteStreamEnd<R, W> {
    reader: Mutex<Option<R>>,
    writer: Mutex<Option<W>>,
    /// Why the session ended at this end; the first reason stands.
    ended: Mutex<Option<Closed>>,
}

impl<R, W> ByteStreamEnd<R, W> {
    fn ended(&self) -> Option<Closed> {
        *lock(&self.ended)
    }

    /// Ends the session at this end if nothing has yet, and returns the
    /// reason that stands.
    fn end(&self, reason: Closed) -> Closed {
        *lock(&self.ended).get_or_insert(reason)
    }

    /// Locks a half of the stream. A call that panicked while holding it may
    /// have left a frame half read or half written, so the stream's framing
    /// is lost, and the session ends.
    fn lock_half<'a, T>(&self, half: &'a Mutex<Option<T>>) -> MutexGuard<'a, Option<T>> {
        match half.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.end(Closed::Stream(ErrorKind::Other));
                poisoned.into_inner()
            }
        }
    }

    /// Once the session has ended, drops each half no call is using. A call
    /// using one checks again once it has released it, so a half is always
    /// dropped by the last call to hold it, or here.
    fn release_if_ended(&self) {
        if self.ended().is_none() {
            return;
        }
        if let Some(mut writer) = try_lock(&self.writer) {
            writer.take();
        }
        if let Some(mut reader) = try_lock(&self.reader) {
            reader.take();
        }
    }
}

impl<R: Read, W: Write> FrameSink for ByteStreamEnd<R, W> {
    fn send(&self, frame: Frame, lane: Lane) -> Result<(), SendError> {
        // A byte stream writes every lane's frames in call order.
        let _ = lane;
        let mut writer = self.lock_half(&self.writer);
        let sent = match (self.ended(), writer.as_mut()) {
            (Some(closed), _) => Err(closed),
            // The writer goes only once the session has ended.
            (None, None) => Err(self.end(Closed::Local)),
            (None, Some(stream)) => match frame.check(Carrier::ByteStream) {
                Err(error) => Err(self.end(Closed::Protocol(error))),
                Ok(_) => write_frame(stream, &frame)
                    .map_err(|error| self.end(Closed::Stream(error.kind()))),
            },
        };
        if self.ended().is_some() {
            writer.take();
        }
        drop(writer);
        self.release_if_ended();
        sent.map_err(|closed| SendError::Closed(frame, closed))
    }

    fn close(&self) {
        self.end(Closed::Local);
        self.release_if_ended();
    }
}

impl<R: Read, W: Write> FrameSource for ByteStreamEnd<R, W> {
    fn recv(&self) -> Result<Frame, Closed> {
        let mut reader = self.lock_half(&self.reader);
        // The body's only owner until the frame is delivered. On every other
        // exit, unwinding included, it drops here and wipes the body.
        let mut body = Body::default();
        let received = match (self.ended(), reader.as_mut()) {
            (Some(closed), _) => Err(closed),
            // The reader goes only once the session has ended.
            (None, None) => Err(self.end(Closed::Local)),
            (None, Some(stream)) => {
                read_frame(stream, &mut body).map_err(|reason| self.end(reason))
            }
        };
        // A frame read while the session was ending here is not delivered: its
        // body stays in the guard, which wipes it.
        let received = match (received, self.ended()) {
            (Ok(tag), None) => Ok(Frame {
                tag,
                body: body.into_inner(),
            }),
            (Ok(_), Some(closed)) | (Err(closed), _) => Err(closed),
        };
        if self.ended().is_some() {
            reader.take();
        }
        drop(reader);
        self.release_if_ended();
        received
    }
}

impl<R, W> fmt::Debug for ByteStreamEnd<R, W> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ByteStreamEnd")
            .field("ended", &self.ended())
            .finish_non_exhaustive()
    }
}

/// A frame's body while it is read: the buffer's only owner until the frame
/// is delivered. Dropping it wipes the whole allocation.
#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    /// What the drop does before the free: overwrites the whole allocation,
    /// spare capacity included, with zeros, without reallocating.
    fn wipe(&mut self) {
        let capacity = self.0.capacity();
        self.0.clear();
        self.0.resize(capacity, 0);
        std::hint::black_box(&self.0);
    }

    /// The body, for delivery. The guard is left empty, so its drop wipes
    /// nothing of it.
    fn into_inner(mut self) -> Vec<u8> {
        mem::take(&mut self.0)
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        self.wipe();
    }
}

/// Reads one frame: the length prefix, checked; the tag byte, checked; then
/// the body, into a buffer of the size the prefix states, which `body` owns
/// from its allocation on. Returns the tag; the body stays in `body`, so every
/// exit but the caller's delivery wipes it.
fn read_frame<R: Read>(stream: &mut R, body: &mut Body) -> Result<u8, Closed> {
    let mut prefix = [0; 4];
    match fill(stream, &mut prefix)? {
        0 => return Err(Closed::Peer),
        4 => {}
        _ => return Err(truncated()),
    }
    let size = u32::from_le_bytes(prefix) as usize;
    Frame::check_size(size).map_err(Closed::Protocol)?;
    let mut tag = [0; 1];
    if fill(stream, &mut tag)? != 1 {
        return Err(truncated());
    }
    Carrier::ByteStream.tag(tag[0]).map_err(Closed::Protocol)?;
    // Replacing the guard drops, and so wipes, whatever it held before.
    *body = Body(vec![0; size - 1]);
    if fill(stream, &mut body.0)? != body.0.len() {
        return Err(truncated());
    }
    Ok(tag[0])
}

/// Reads until `buffer` is full or the stream ends, retrying interruptions.
/// Returns how much it read.
fn fill<R: Read>(stream: &mut R, buffer: &mut [u8]) -> Result<usize, Closed> {
    let mut filled = 0;
    while filled < buffer.len() {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(Closed::Stream(error.kind())),
        }
    }
    Ok(filled)
}

fn truncated() -> Closed {
    Closed::Stream(ErrorKind::UnexpectedEof)
}

/// Writes a checked frame: its size, its tag byte and its body, then flushes.
fn write_frame<W: Write>(stream: &mut W, frame: &Frame) -> io::Result<()> {
    // A checked frame's size is at most 64 MiB, so it fits a `u32`.
    let size = u32::try_from(frame.size()).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    let [a, b, c, d] = size.to_le_bytes();
    stream.write_all(&[a, b, c, d, frame.tag])?;
    stream.write_all(&frame.body)?;
    stream.flush()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Only the ending's reason uses this lock, and nothing that holds it can
    // panic, so it is never poisoned in practice.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Locks a half without waiting, for dropping it once the session has ended,
/// poisoned or not.
fn try_lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

mod tests;
mod wipe_tests;
