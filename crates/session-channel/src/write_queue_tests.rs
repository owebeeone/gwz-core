#![cfg(test)]
//! The byte-stream writer's order (operator decision D-N): a control frame goes ahead of ordinary frames at a frame
//! boundary, except ordinary frames of its own target and any `SessionCall`, so per-target order and the
//! increasing-call-ID rule survive. The queue sees a key and a tag byte, never a body.

use crate::WriteQueue;
use gwz_session_contract::{FrameKey, Target};

const DATA: u8 = 16;
const CONTROL: u8 = 17;
const CALL: u8 = 1;
const REPLY: u8 = 2;

fn ordinary(target: u64) -> FrameKey {
    FrameKey::ordinary(Target::new(target))
}

fn control(target: u64) -> FrameKey {
    FrameKey::control(Target::new(target))
}

fn drain(mut queue: WriteQueue<&'static str>) -> Vec<&'static str> {
    std::iter::from_fn(|| queue.pop()).collect()
}

#[test]
fn a_control_frame_overtakes_a_queue_of_large_data_frames() {
    let mut queue = WriteQueue::new();
    for name in ["d1", "d2", "d3", "d4"] {
        queue.push(name, DATA, ordinary(2));
    }
    queue.push("cancel", CONTROL, control(1));
    assert_eq!(drain(queue), ["cancel", "d1", "d2", "d3", "d4"]);
}

#[test]
fn frames_of_one_target_keep_their_order_and_the_control_frame_waits_for_its_own() {
    let mut queue = WriteQueue::new();
    queue.push("a1", DATA, ordinary(1));
    queue.push("b1", DATA, ordinary(2));
    queue.push("b2", DATA, ordinary(2));
    queue.push("close-a", CONTROL, control(1));
    // The close of stream 1 is behind a1, the ordinary frame of its own target; it overtakes b1 and b2.
    assert_eq!(drain(queue), ["a1", "close-a", "b1", "b2"]);
}

#[test]
fn a_session_call_is_never_overtaken() {
    let mut queue = WriteQueue::new();
    queue.push("call-4", CALL, ordinary(4));
    queue.push("reply-1", REPLY, ordinary(1));
    queue.push("window", CONTROL, control(9));
    // The window waits for the call ahead of it, whatever its target.
    assert_eq!(drain(queue), ["call-4", "window", "reply-1"]);

    // A control call (a cancel) goes ahead of replies and data, but not ahead of an earlier call.
    let mut queue = WriteQueue::new();
    queue.push("reply-1", REPLY, ordinary(1));
    queue.push("call-5", CALL, ordinary(5));
    queue.push("reply-2", REPLY, ordinary(2));
    queue.push("cancel-6", CALL, control(6));
    assert_eq!(drain(queue), ["reply-1", "call-5", "cancel-6", "reply-2"]);

    let mut queue = WriteQueue::new();
    queue.push("reply-1", REPLY, ordinary(1));
    queue.push("reply-2", REPLY, ordinary(2));
    queue.push("cancel-6", CALL, control(6));
    assert_eq!(drain(queue), ["cancel-6", "reply-1", "reply-2"]);
}

#[test]
fn a_control_reply_waits_behind_ordinary_frames_of_its_own_call() {
    let mut queue = WriteQueue::new();
    queue.push("spool-7", REPLY, ordinary(7));
    queue.push("spool-8", REPLY, ordinary(8));
    queue.push("done-7", REPLY, control(7));
    assert_eq!(drain(queue), ["spool-7", "done-7", "spool-8"]);
}

#[test]
fn the_close_report_keyed_ordinary_is_written_after_every_frame_queued_before_it() {
    // The close report travels on the control lane but core keys it ordinary, so it is not control here.
    let mut queue = WriteQueue::new();
    queue.push("reply-1", REPLY, ordinary(1));
    queue.push("reply-2", REPLY, ordinary(2));
    queue.push("close-report", REPLY, ordinary(3));
    assert_eq!(drain(queue), ["reply-1", "reply-2", "close-report"]);
}

#[test]
fn control_frames_keep_their_order_among_themselves() {
    let mut queue = WriteQueue::new();
    queue.push("d", DATA, ordinary(3));
    queue.push("w1", CONTROL, control(4));
    queue.push("w2", CONTROL, control(5));
    queue.push("w3", CONTROL, control(4));
    assert_eq!(drain(queue), ["w1", "w2", "w3", "d"]);
}

#[test]
fn a_target_that_collides_only_keeps_more_order() {
    // Two streams that hash alike are one target to the queue: the close of one waits for the data of the other.
    let mut queue = WriteQueue::new();
    queue.push("a1", DATA, ordinary(1));
    queue.push("close", CONTROL, control(1));
    queue.push("b1", DATA, ordinary(1));
    assert_eq!(drain(queue), ["a1", "close", "b1"]);
}

#[test]
fn the_queue_counts_its_frames() {
    let mut queue = WriteQueue::new();
    queue.push((), DATA, ordinary(1));
    queue.push((), CONTROL, control(1));
    assert_eq!(queue.len(), 2);
    queue.pop();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.peek(), Some(&()));
    queue.clear();
    assert!(queue.is_empty());
}
