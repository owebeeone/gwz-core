#![cfg(test)]
//! The byte-stream writer takes turns by the D-N rule: a control frame goes ahead of ordinary frames that wait with
//! it, except those of its own target and any `SessionCall`; alone, a sender is written in call order. The end sees a
//! key and a tag byte, never a body.

use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::test_router::open;
use gwz_session_contract::{Frame, FrameKey, FrameSink, Lane, Target};

/// A writer whose first write waits for a release, so that the senders behind it queue up, and that records the body
/// of every frame it is given (each frame's body is its name).
#[derive(Clone)]
struct Gated {
    inner: Arc<(Mutex<GateState>, Condvar)>,
}

#[derive(Default)]
struct GateState {
    open: bool,
    bytes: Vec<u8>,
}

impl Gated {
    fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(GateState::default()), Condvar::new())),
        }
    }

    fn open(&self) {
        self.inner.0.lock().unwrap().open = true;
        self.inner.1.notify_all();
    }

    /// The names of the frames written, in order.
    fn names(&self) -> Vec<String> {
        let bytes = self.inner.0.lock().unwrap().bytes.clone();
        let mut names = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            names.push(String::from_utf8(bytes[at + 5..at + 4 + size].to_vec()).unwrap());
            at += 4 + size;
        }
        names
    }
}

impl Write for Gated {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (lock, changed) = &*self.inner;
        let mut state = lock.lock().unwrap();
        while !state.open {
            state = changed.wait(state).unwrap();
        }
        state.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame(tag: u8, name: &str) -> Frame {
    Frame {
        tag,
        body: name.as_bytes().to_vec(),
    }
}

fn ordinary(target: u64) -> FrameKey {
    FrameKey::ordinary(Target::new(target))
}

fn control(target: u64) -> FrameKey {
    FrameKey::control(Target::new(target))
}

/// What a row queues: a tag, a name and a key.
type Queued<'a> = (u8, &'a str, FrameKey);

/// Sends `first` (which holds the writer at the gate), then each of `waiting` in order from its own thread, once the
/// one before it is queued; opens the gate and returns the order the frames were written in.
fn written(first: Queued<'_>, waiting: &[Queued<'_>]) -> Vec<String> {
    let gate = Gated::new();
    let (reads, _peer) = io::pipe().expect("an OS pipe");
    let (end, _router) = open(reads, gate.clone());
    let end = Arc::new(end);
    let mut threads = Vec::new();
    let mut expect = 0;
    let mut start = |(tag, name, key): Queued<'_>| {
        let sender = end.clone();
        let frame = frame(tag, name);
        threads.push(thread::spawn(move || {
            sender.send(frame, Lane::Call, key).unwrap();
        }));
        expect += 1;
        let deadline = Instant::now() + Duration::from_secs(20);
        while end.waiting_senders() < expect {
            assert!(Instant::now() < deadline, "a sender never queued");
            thread::sleep(Duration::from_millis(1));
        }
    };
    start(first);
    for queued in waiting {
        start(*queued);
    }
    gate.open();
    for thread in threads {
        thread.join().unwrap();
    }
    gate.names()
}

#[test]
fn a_control_frame_overtakes_data_frames_waiting_for_the_writer() {
    let order = written(
        (16, "first", ordinary(99)),
        &[
            (16, "d1", ordinary(2)),
            (16, "d2", ordinary(2)),
            (16, "d3", ordinary(2)),
            (17, "cancel", control(1)),
        ],
    );
    // The frame being written is not interrupted; the cancel goes next.
    assert_eq!(order, ["first", "cancel", "d1", "d2", "d3"]);
}

#[test]
fn a_frame_of_the_same_target_is_never_overtaken() {
    let order = written(
        (16, "first", ordinary(99)),
        &[
            (16, "a1", ordinary(1)),
            (16, "b1", ordinary(2)),
            (16, "a2", ordinary(1)),
            (17, "close-a", control(1)),
        ],
    );
    // The close of stream 1 stays behind a1 and a2, the frames of its own target.
    assert_eq!(order, ["first", "a1", "b1", "a2", "close-a"]);
}

#[test]
fn a_cancel_queued_after_the_open_and_data_of_its_stream_arrives_after_them() {
    let order = written(
        (16, "first", ordinary(99)),
        &[
            (16, "open-1", ordinary(1)),
            (16, "data-1", ordinary(1)),
            (16, "other", ordinary(2)),
            (17, "cancel-1", control(1)),
        ],
    );
    assert_eq!(order, ["first", "open-1", "data-1", "cancel-1", "other"]);
}

#[test]
fn a_session_call_is_never_overtaken() {
    let order = written(
        (16, "first", ordinary(99)),
        &[
            (1, "call-4", ordinary(4)),
            (16, "d1", ordinary(2)),
            (17, "window", control(1)),
        ],
    );
    assert_eq!(order, ["first", "call-4", "window", "d1"]);
}

#[test]
fn a_control_call_goes_ahead_of_replies_but_not_of_an_earlier_call() {
    let order = written(
        (2, "reply-1", ordinary(1)),
        &[
            (2, "reply-2", ordinary(2)),
            (1, "call-5", ordinary(5)),
            (2, "reply-3", ordinary(3)),
            (1, "cancel-6", control(6)),
        ],
    );
    assert_eq!(
        order,
        ["reply-1", "reply-2", "call-5", "cancel-6", "reply-3"]
    );
}

#[test]
fn the_close_report_never_overtakes_the_replies_queued_before_it() {
    // The close report goes on the control lane but core keys it ordinary, so the writer writes it last.
    let order = written(
        (2, "reply-1", ordinary(1)),
        &[
            (2, "reply-2", ordinary(2)),
            (2, "reply-3", ordinary(3)),
            (2, "close-report", ordinary(4)),
        ],
    );
    assert_eq!(order, ["reply-1", "reply-2", "reply-3", "close-report"]);
}

#[test]
fn senders_that_never_wait_together_are_written_in_call_order() {
    let gate = Gated::new();
    gate.open();
    let (reads, _peer) = io::pipe().expect("an OS pipe");
    let (end, _router) = open(reads, gate.clone());
    for (index, (tag, key)) in [
        (16, ordinary(1)),
        (17, control(2)),
        (16, ordinary(3)),
        (17, control(4)),
        (1, ordinary(5)),
    ]
    .into_iter()
    .enumerate()
    {
        end.send(frame(tag, &format!("f{index}")), Lane::Control, key)
            .unwrap();
    }
    assert_eq!(gate.names(), ["f0", "f1", "f2", "f3", "f4"]);
}
