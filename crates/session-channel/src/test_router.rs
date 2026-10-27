#![cfg(test)]
//! The router the tests give a byte-stream end: it queues the frames it is given and answers `recv`, as the
//! conformance suite needs (operator decision D-N), and it never blocks in `route`.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::{ByteStreamEnd, byte_stream};
use gwz_session_contract::{
    Closed, Frame, FrameKey, FrameRouter, FrameSink, FrameSource, Lane, SendError,
};

#[derive(Default)]
struct State {
    frames: VecDeque<Frame>,
    ended: Option<Closed>,
    ends_told: usize,
}

/// A router that queues what it is given.
#[derive(Default)]
pub(crate) struct QueueRouter {
    state: Mutex<State>,
    ready: Condvar,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl QueueRouter {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The next frame, waiting for it; or why the session has ended. An end this side closed receives nothing more;
    /// any other end delivers the frames that came before it first.
    pub(crate) fn recv(&self) -> Result<Frame, Closed> {
        let mut state = lock(&self.state);
        loop {
            if state.ended == Some(Closed::Local) {
                return Err(Closed::Local);
            }
            if let Some(frame) = state.frames.pop_front() {
                return Ok(frame);
            }
            if let Some(reason) = state.ended {
                return Err(reason);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// How many times the session end was reported.
    pub(crate) fn ends_told(&self) -> usize {
        lock(&self.state).ends_told
    }
}

impl FrameRouter for QueueRouter {
    fn route(&self, frame: Frame) -> Result<(), Closed> {
        let mut state = lock(&self.state);
        state.frames.push_back(frame);
        self.ready.notify_all();
        Ok(())
    }

    fn ended(&self, reason: Closed) {
        let mut state = lock(&self.state);
        state.ends_told += 1;
        state.ended.get_or_insert(reason);
        self.ready.notify_all();
    }
}

/// A byte-stream end and the router that answers its `recv`.
pub(crate) fn open<R, W>(read: R, write: W) -> (ByteStreamEnd<W>, Arc<QueueRouter>)
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let router = QueueRouter::new();
    let end = byte_stream(read, write, router.clone()).expect("a reader thread");
    (end, router)
}

/// A byte-stream end that is also a `FrameSource`, as the conformance suite takes it: the end is the sink, and the
/// router the source.
pub(crate) struct RoutedEnd<W> {
    end: ByteStreamEnd<W>,
    router: Arc<QueueRouter>,
}

impl<W: Write + Send + 'static> RoutedEnd<W> {
    pub(crate) fn new<R: Read + Send + 'static>(read: R, write: W) -> Self {
        let (end, router) = open(read, write);
        Self { end, router }
    }
}

impl<W: Write + Send + 'static> FrameSink for RoutedEnd<W> {
    fn send(&self, frame: Frame, lane: Lane, key: FrameKey) -> Result<(), SendError> {
        self.end.send(frame, lane, key)
    }

    fn close(&self) {
        self.end.close();
    }
}

impl<W: Write + Send + 'static> FrameSource for RoutedEnd<W> {
    fn recv(&self) -> Result<Frame, Closed> {
        self.router.recv()
    }
}

/// Every frame an end reads from `read`, and the closure that ends them.
pub(crate) fn read_all(read: impl Read + Send + 'static) -> (Vec<Frame>, Closed) {
    let (end, router) = open(read, std::io::sink());
    let mut frames = Vec::new();
    loop {
        match router.recv() {
            Ok(frame) => frames.push(frame),
            Err(closed) => {
                assert_eq!(router.recv(), Err(closed), "the closure is reported again");
                drop(end);
                return (frames, closed);
            }
        }
    }
}
