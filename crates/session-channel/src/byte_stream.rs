//! The byte-stream adapter (§3): frames over any reader and writer, such as
//! standard streams, a socket or SSH.
//!
//! On the stream, each frame is its size as a little-endian `u32`, then its
//! tag byte, then its body, exactly as taut-shape's interop tool frames it
//! (`taut-shape-tool`'s `framing.rs`). The size counts the tag byte and the
//! body, and is at most 64 MiB.
//!
//! **Reading (operator decision D-N).** The end reads in a thread of its own and
//! hands every frame it reads, as a tag and a body, to the [`FrameRouter`] that
//! core gave it. The reader checks the length prefix, the tag and the size, as
//! it always did, and never waits for a consumer: `route` never blocks, and core
//! owns every per-class and per-binding queue. The end is a [`FrameSink`] only.
//!
//! **Writing (D-N).** `send` takes a [`FrameKey`], and senders that wait for the
//! writer together take turns by the [`WriteQueue`] rule: the oldest control
//! frame goes ahead of the ordinary frames before it, except ordinary frames of
//! its own target and any `SessionCall`.
//!
//! A body can carry the environment snapshot (the server design's
//! `SessionOpen`), so the reader reads it into a guard, `Body`, that owns the
//! buffer until the frame is delivered. The guard's drop overwrites the whole
//! allocation with zeros, spare capacity included, without reallocating. So
//! every exit of a read but delivery wipes the body:
//! - a stream that ends inside the frame;
//! - one that fails;
//! - one that panics mid-read, whose unwinding drops the guard;
//! - a frame read whole while the session was ending here, which the reader
//!   discards.
//!
//! The wipe is safe code, since the crate forbids unsafe code. After the
//! writes, `std::hint::black_box` is std's best-effort barrier against the
//! compiler removing them, weaker than the volatile writes and fence with
//! which gwz-core wipes its snapshot. A delivered body is the router's: core's
//! decoder owns its wipe.

// Ordinary stream I/O in a channel adapter: this crate is outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml), whose disallowed writers exist
// to route merge artifact mutation through checked entries.
#![allow(clippy::disallowed_methods)]

use std::fmt;
use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread;

use crate::write_queue::WriteQueue;
use gwz_session_contract::{
    Carrier, Closed, Frame, FrameKey, FrameRouter, FrameSink, Lane, SendError,
};

/// One end of a byte-stream channel, writing frames to `write` and reading them from `read` in a thread of its own,
/// which hands each to `router`. The other end is whoever holds the stream's far side, usually another process.
///
/// # Errors
///
/// The reader thread could not be started.
pub fn byte_stream<R, W>(
    read: R,
    write: W,
    router: Arc<dyn FrameRouter>,
) -> io::Result<ByteStreamEnd<W>>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let shared = Arc::new(Shared {
        writer: Mutex::new(Some(write)),
        write_failed: Mutex::new(None),
        ended: Mutex::new(None),
        router,
        delivery: Mutex::new(()),
    });
    let reading = shared.clone();
    thread::Builder::new()
        .name("gwz-byte-stream-reader".into())
        .spawn(move || read_loop(read, &reading))?;
    Ok(ByteStreamEnd {
        shared,
        turns: Turns::default(),
    })
}

/// What the end and its reader thread share.
struct Shared<W> {
    writer: Mutex<Option<W>>,
    /// A failure of the stream's far side that the writer met (a broken pipe, a reset): later sends fail with it, but the
    /// session is not over, because the reader still has what the peer sent before it stopped listening to read.
    write_failed: Mutex<Option<ErrorKind>>,
    /// Why the session ended at this end; the first reason stands.
    ended: Mutex<Option<Closed>>,
    router: Arc<dyn FrameRouter>,
    /// Held while a frame is routed and while the end is reported, so that `ended` follows the last `route`.
    delivery: Mutex<()>,
}

/// One end of a byte-stream channel.
///
/// - The carrier is [`Carrier::ByteStream`]: tags 1 to 8, 16 and 17.
/// - The reader checks the length prefix before it reads on: a prefix of
///   zero, or over 64 MiB, ends the session before anything is allocated. It
///   then checks the tag before it allocates or reads the body. A tag outside
///   the registry ends the session too. The body's buffer is allocated once,
///   at the size the prefix states, so no partial copy of a body is left
///   behind, and it is wiped before it is freed on every path but delivery
///   (the module's documentation).
/// - End of stream between frames is [`Closed::Peer`]. Inside a frame, and
///   on any failure of the stream other than an interruption, the session
///   ends with [`Closed::Stream`]. An error from the router ends it with the
///   router's reason. The router is told once, with the reason that stands.
/// - `send` writes the whole frame and flushes it, holding the writer so
///   that frames from several threads never interleave. It waits while the
///   writer does. The lane bounds nothing: a byte stream has no queue of its
///   own to bound. Senders that wait together take turns by the writer's
///   rule (operator decision D-N, [`WriteQueue`]): the oldest control frame
///   goes ahead of the ordinary frames before it, except ordinary frames of
///   its own target and any `SessionCall`. A sender that never waits with
///   another is written in call order. A frame under a tag outside the
///   registry, or over 64 MiB, ends the session unwritten.
/// - When the session ends at this end, by `close`, a protocol error or a
///   failed stream, the end drops its writer, so that the peer reads end of
///   stream after the frames sent before. It is not dropped while a call is
///   using it: the call drops it on return.
///
/// A `send` already waiting inside the stream, and the reader thread, stay
/// there until the stream returns, which usually happens once the peer closes
/// in turn. Only the stream's owner can force it, by shutting a socket down or
/// killing a child. Dropping a half must end the peer's stream: a pipe's half
/// does, but a cloned socket's half does not while its other clone lives, so
/// its owner passes halves that shut the socket down when dropped.
///
/// An end is `Send` and `Sync` when its writer is `Send`. Dropping the end
/// closes it.
pub struct ByteStreamEnd<W> {
    shared: Arc<Shared<W>>,
    /// Senders waiting for the writer, in the order the writer takes them (operator decision D-N).
    turns: Turns,
}

/// The senders waiting for the writer. The writer writes one frame at a time; the next is the one the
/// [`WriteQueue`] rule picks among the frames waiting, so a control frame does not wait behind a queue of ordinary
/// ones of other targets. Alone, a sender never waits and the order is the order of the calls.
#[derive(Default)]
struct Turns {
    state: Mutex<TurnState>,
    changed: Condvar,
}

#[derive(Default)]
struct TurnState {
    queue: WriteQueue<u64>,
    next: u64,
    busy: bool,
}

/// The writer's turn, held while a frame is written and given back on drop.
struct Turn<'a>(&'a Turns);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .busy = false;
        self.0.changed.notify_all();
    }
}

impl Turns {
    fn wait(&self, tag: u8, key: FrameKey) -> Turn<'_> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = state.next;
        state.next += 1;
        state.queue.push(ticket, tag, key);
        loop {
            if !state.busy && state.queue.peek() == Some(&ticket) {
                state.queue.pop();
                state.busy = true;
                return Turn(self);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl<W> Shared<W> {
    fn ended(&self) -> Option<Closed> {
        *lock(&self.ended)
    }

    /// Ends the session at this end if nothing has yet, tells the router, and returns the reason that stands.
    fn end(&self, reason: Closed) -> Closed {
        let (stands, first) = {
            let mut ended = lock(&self.ended);
            match *ended {
                Some(stands) => (stands, false),
                None => {
                    *ended = Some(reason);
                    (reason, true)
                }
            }
        };
        if first {
            let _delivery = lock(&self.delivery);
            // A router that panics has nothing more to learn; the end goes on.
            let _ = panic::catch_unwind(AssertUnwindSafe(|| self.router.ended(stands)));
        }
        stands
    }

    /// Locks the writer. A call that panicked while holding it may have left a frame half written, so the stream's
    /// framing is lost, and the session ends.
    fn lock_writer(&self) -> MutexGuard<'_, Option<W>> {
        match self.writer.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.end(Closed::Stream(ErrorKind::Other));
                poisoned.into_inner()
            }
        }
    }

    /// Once the session has ended, drops the writer if no call is using it. A call using it checks again once it has
    /// released it, so it is always dropped by the last call to hold it, or here.
    fn release_if_ended(&self) {
        if self.ended().is_none() {
            return;
        }
        if let Some(mut writer) = try_lock(&self.writer) {
            writer.take();
        }
    }
}

/// Reads frames until the stream or the session ends, handing each to the router.
fn read_loop<R: Read, W>(mut stream: R, shared: &Shared<W>) {
    loop {
        // The body's only owner until the frame is delivered. On every other exit, unwinding included, it drops and
        // wipes the body.
        let mut body = Body::default();
        let read = panic::catch_unwind(AssertUnwindSafe(|| read_frame(&mut stream, &mut body)));
        let tag = match read {
            Ok(Ok(tag)) => tag,
            Ok(Err(reason)) => {
                shared.end(reason);
                break;
            }
            // A stream that panicked in the middle of a frame has lost its framing.
            Err(_) => {
                shared.end(Closed::Stream(ErrorKind::Other));
                break;
            }
        };
        // A frame read while the session was ending here is not delivered: its body stays in the guard, which wipes it.
        let routed = {
            let _delivery = lock(&shared.delivery);
            if shared.ended().is_some() {
                break;
            }
            let frame = Frame {
                tag,
                body: body.into_inner(),
            };
            panic::catch_unwind(AssertUnwindSafe(|| shared.router.route(frame)))
        };
        match routed {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => {
                shared.end(reason);
                break;
            }
            Err(_) => {
                shared.end(Closed::Stream(ErrorKind::Other));
                break;
            }
        }
    }
    shared.release_if_ended();
}

impl<W: Write> FrameSink for ByteStreamEnd<W> {
    /// The lane bounds nothing on a byte stream; the key orders the write among the senders waiting with this one.
    fn send(&self, frame: Frame, lane: Lane, key: FrameKey) -> Result<(), SendError> {
        let _ = lane;
        let _turn = self.turns.wait(frame.tag, key);
        let mut writer = self.shared.lock_writer();
        let failed = *lock(&self.shared.write_failed);
        let sent = match (self.shared.ended(), writer.as_mut(), failed) {
            (Some(closed), _, _) => Err(closed),
            (None, _, Some(kind)) => Err(Closed::Stream(kind)),
            // The writer goes only once the session has ended.
            (None, None, None) => Err(self.shared.end(Closed::Local)),
            (None, Some(stream), None) => match frame.check(Carrier::ByteStream) {
                Err(error) => Err(self.shared.end(Closed::Protocol(error))),
                Ok(_) => match write_frame(stream, &frame) {
                    Ok(()) => Ok(()),
                    // The far side stopped listening. The session goes on reading until the stream ends.
                    Err(error) if peer_side_failure(error.kind()) => {
                        *lock(&self.shared.write_failed) = Some(error.kind());
                        Err(Closed::Stream(error.kind()))
                    }
                    Err(error) => Err(self.shared.end(Closed::Stream(error.kind()))),
                },
            },
        };
        if self.shared.ended().is_some() {
            writer.take();
        }
        drop(writer);
        self.shared.release_if_ended();
        sent.map_err(|closed| SendError::Closed(frame, closed))
    }

    fn close(&self) {
        self.shared.end(Closed::Local);
        self.shared.release_if_ended();
    }
}

impl<W> ByteStreamEnd<W> {
    /// Senders waiting for the writer, the one writing included.
    pub fn waiting_senders(&self) -> usize {
        let state = self
            .turns
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.queue.len() + usize::from(state.busy)
    }
}

impl<W> Drop for ByteStreamEnd<W> {
    fn drop(&mut self) {
        self.shared.end(Closed::Local);
        self.shared.release_if_ended();
    }
}

impl<W> fmt::Debug for ByteStreamEnd<W> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ByteStreamEnd")
            .field("ended", &self.shared.ended())
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

/// Whether an I/O failure is the far side of the stream going away (a broken pipe, a reset, a stream that ended inside
/// a frame), as opposed to this end's own trouble. A byte-stream end does not end its session on such a failure of its
/// writer, and its owner treats the same failure of the reader as the peer's departure.
pub fn peer_side_failure(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof
    )
}

fn truncated() -> Closed {
    Closed::Stream(ErrorKind::UnexpectedEof)
}

/// Writes a checked frame: its size, its tag byte and its body, then flushes. For a caller that owns a raw stream and has
/// one frame to put on it before any end exists (the refusal a listener owes a peer it turned away); a frame past the
/// 64 MiB limit is refused as invalid input.
pub fn write_frame<W: Write>(stream: &mut W, frame: &Frame) -> io::Result<()> {
    Frame::check_size(frame.size()).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    // A checked frame's size is at most 64 MiB, so it fits a `u32`.
    let size = u32::try_from(frame.size()).map_err(|_| io::Error::from(ErrorKind::InvalidInput))?;
    let [a, b, c, d] = size.to_le_bytes();
    stream.write_all(&[a, b, c, d, frame.tag])?;
    stream.write_all(&frame.body)?;
    stream.flush()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Only the ending's reason and the delivery fence use these locks, and nothing that holds them can panic past a
    // catch, so they are never poisoned in practice.
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

mod priority_tests;
mod tests;
mod wipe_tests;
