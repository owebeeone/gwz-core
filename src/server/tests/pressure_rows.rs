//! A served session's replies against the host writer's byte bound (remediation of step 2.3g1, finding F1/P2-1;
//! `GwzSplitCommandChannel.md` §4.3 "Stuck peers"; `GwzCoreSessionDesign.md` §3 "Volume is bounded"). The writer's
//! queue holds at most `QUEUE_BYTES`; a client within its own call bound may have more than that in replies due. Byte
//! pressure is backpressure: a client that reads gets every reply, and one that has stopped reading ends at the
//! policy's stall bound as `Stalled`, never as an orderly close.
//!
//! Round 2 (finding F9/P2-N1; `GwzCoreSessionDesign.md` §5.1 "The reading thread handles `operation.cancel`,
//! `session.close` and log reads without blocking", §3 D-N): byte pressure never holds the session's reading thread,
//! so a cancel, a close and a new call behind replies past the byte bound are routed while the client reads nothing.
use std::thread;
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, Tag};

use super::session_rows::{
    Host, RECORD, STALL, WAIT, call_frame, call_id_of, cancel_call, fetch_call, hello,
    small_limits, spool_read,
};
use crate::server::session::SessionPolicy;
use crate::session_host::served::stream::StreamSession;
use crate::transport_host::carrier::{CloseCode, CloseReason, Lane, QUEUE_BYTES};

/// More `diff.output` reads than the writer's queue holds in replies: each reply is one [`RECORD`].
const READS: usize = 14;

// The rows' reply volume is past the writer's byte bound, with room to spare for what the stream itself holds.
const _: () = assert!(READS * RECORD > QUEUE_BYTES + 2 * RECORD);

/// F1 (a): the production policy's limits, a client that pipelines [`READS`] reads, within its outstanding-call bound,
/// and starts reading a little late. Every reply arrives and the session stays open until the client closes it.
#[test]
fn a_reading_client_within_its_bounds_gets_every_reply_past_the_byte_bound() {
    let host = Host::new(SessionPolicy::new(hello()));
    assert!(READS < host.policy.limits.outstanding_calls);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    let reads: Vec<Frame> = (0..READS)
        .map(|reader| spool_read(reader as i64 + 1, &log_id, reader))
        .collect();
    raw.write(&reads);
    // The client is slow to start reading: the host's queue meets its byte bound meanwhile.
    thread::sleep(Duration::from_millis(100));
    let mut answered: Vec<i64> = (0..READS).map(|_| call_id_of(&raw.read())).collect();
    answered.sort_unstable();
    assert_eq!(answered, (1..=READS as i64).collect::<Vec<_>>());
    assert!(
        session.carrier().wait_closed(Duration::ZERO).is_none(),
        "the session ended under byte pressure"
    );
    assert_eq!(session.service().dropped_replies(), 0);
    let close = crate::SessionCloseRequest {};
    raw.write(&[call_frame(
        READS as i64 + 1,
        "session.close",
        crate::encode(&close.to_cbor()),
    )]);
    let report = raw.read();
    assert_eq!(report.tag, Tag::SessionReply.byte());
    assert_eq!(call_id_of(&report), READS as i64 + 1);
    let closed = session.carrier().wait_closed(WAIT).expect("the close");
    assert_eq!(closed.reason, CloseReason::Local(CloseCode::Normal));
}

/// F1 (b): a client that has stopped reading, with more replies due than the writer's queue holds, ends at the policy's
/// stall bound as carrier loss (`Stalled`), as row 5's does below the byte bound; not as an orderly close.
#[test]
fn a_stopped_client_past_the_byte_bound_ends_as_stalled() {
    let mut policy = SessionPolicy::new(hello());
    policy.limits = small_limits();
    policy.limits.outstanding_calls = 16;
    policy.stall = STALL;
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    let started = Instant::now();
    let reads: Vec<Frame> = (0..READS)
        .map(|reader| spool_read(reader as i64 + 1, &log_id, reader))
        .collect();
    raw.write(&reads);
    let closed = session
        .carrier()
        .wait_closed(WAIT)
        .expect("the session ends");
    let elapsed = started.elapsed();
    assert_eq!(closed.reason, CloseReason::Stalled);
    assert!(elapsed >= STALL && elapsed < WAIT, "{elapsed:?}");
    assert!(session.service().wait_ended(WAIT));
    drop(raw);
}

/// The reads a row pipelines behind a running fetch (call 1): calls 2 to `READS + 1`.
fn reads_behind_a_fetch(log_id: &str) -> Vec<Frame> {
    (0..READS)
        .map(|reader| spool_read(reader as i64 + 2, log_id, reader))
        .collect()
}

/// F9 (a): the production policy, a running fetch, [`READS`] reads past the byte bound, a new fetch and a cancel of the
/// first. While the client reads nothing, the cancel reaches the handler and the new fetch is admitted and run: the
/// reading thread is not held by byte pressure. Then the client reads every reply, the cancel's ahead of spool replies
/// queued before it, and keeps its session.
#[test]
fn a_cancel_and_a_new_call_progress_while_replies_past_the_byte_bound_are_unread() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    raw.write(&[fetch_call(&host.root, 1, "net-1")]);
    host.script.wait("the fetch", |seen| seen.started == 1);
    let fetch = READS as i64 + 2;
    let cancel = READS as i64 + 3;
    let mut frames = reads_behind_a_fetch(&log_id);
    frames.push(fetch_call(&host.root, fetch, "plain-2"));
    frames.push(cancel_call(cancel, 1));
    raw.write(&frames);
    host.script
        .wait("the cancel at the handler and the new fetch", |seen| {
            seen.cancel_seen && seen.started == 2
        });
    // The cancel's reply is with the writer, on the control lane, while spool replies before it are still held.
    let deadline = Instant::now() + WAIT;
    while session.carrier().free_frames(Lane::Control) == host.policy.limits.control_reserve {
        assert!(
            Instant::now() < deadline,
            "the cancel's reply is held behind spool replies"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let mut order: Vec<i64> = (1..=cancel).map(|_| call_id_of(&raw.read())).collect();
    let at = |id: i64| order.iter().position(|&read| read == id).unwrap();
    assert!(
        at(cancel) < at(READS as i64 + 1),
        "the cancel's reply waited behind every spool reply: {order:?}"
    );
    order.sort_unstable();
    assert_eq!(order, (1..=cancel).collect::<Vec<_>>());
    assert!(
        session.carrier().wait_closed(Duration::ZERO).is_none(),
        "the session ended under byte pressure"
    );
    assert_eq!(session.service().dropped_replies(), 0);
}

/// F9 (b): `session.close` behind [`READS`] reads past the byte bound starts at once: it cancels the running fetch while
/// the client reads nothing. The client then reads every reply, the close report last, and the carrier closes in
/// order (`Local(Normal)`, not `Stalled`).
#[test]
fn session_close_behind_replies_past_the_byte_bound_cancels_at_once_and_reports_last() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    raw.write(&[fetch_call(&host.root, 1, "net-1")]);
    host.script.wait("the fetch", |seen| seen.started == 1);
    let close = READS as i64 + 2;
    let mut frames = reads_behind_a_fetch(&log_id);
    frames.push(call_frame(
        close,
        "session.close",
        crate::encode(&crate::SessionCloseRequest {}.to_cbor()),
    ));
    raw.write(&frames);
    host.script
        .wait("the close's cancel at the handler", |seen| seen.cancel_seen);
    let order: Vec<i64> = (1..=close).map(|_| call_id_of(&raw.read())).collect();
    assert_eq!(order.last(), Some(&close), "{order:?}");
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=close).collect::<Vec<_>>());
    let closed = session.carrier().wait_closed(WAIT).expect("the close");
    assert_eq!(closed.reason, CloseReason::Local(CloseCode::Normal));
    assert!(session.service().wait_ended(WAIT));
}

/// The replies a session holds for its writer are bounded by the session's call bound: a client that keeps sending
/// reads past its outstanding-call bound and reads none meets a full lane, and its session ends at once (session design
/// §3: "only a client that exceeds its own bounds meets a full call lane"; remediation round 3, N1): the carrier closes
/// `Normal` and not as `Stalled`, the session ends, and a call sent after the overrun is never run. Past the byte bound,
/// so the held replies are what is counted; in batches that the host routes before the next, so the carrier's inbound
/// bound is not what ends it.
#[test]
fn a_client_past_its_call_bound_behind_the_byte_bound_loses_its_session() {
    const BATCH: usize = 8;
    const SOON: Duration = Duration::from_secs(2);
    let mut policy = SessionPolicy::new(hello());
    policy.limits = small_limits();
    policy.limits.outstanding_calls = 16;
    // Well past `SOON`: a session left open by the overrun ends as `Stalled` here, far later than the row's look.
    policy.stall = Duration::from_secs(5);
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    let most = 4 * host.policy.limits.outstanding_calls;
    let mut sent = 0;
    // The overrun is the session host's `Full`: the channel closes, and a reply behind it is counted as dropped.
    while session.service().dropped_replies() == 0
        && session.carrier().wait_closed(Duration::ZERO).is_none()
    {
        assert!(
            sent < most,
            "{sent} replies were held: nothing bounds a client past its call bound"
        );
        let frames: Vec<Frame> = (sent..sent + BATCH)
            .map(|reader| spool_read(reader as i64 + 1, &log_id, reader))
            .collect();
        if !raw.try_write(&frames) {
            break;
        }
        sent += BATCH;
        let deadline = Instant::now() + WAIT;
        while session.carrier().queued_in() > 0 {
            assert!(Instant::now() < deadline, "the host routed nothing");
            thread::sleep(Duration::from_millis(2));
        }
        // The last frame taken is routed, its reply offered, before the next look.
        thread::sleep(Duration::from_millis(20));
    }
    // A call after the overrun: a session left open runs it.
    let started = host.script.seen.lock().unwrap().started;
    let _ = raw.try_write(&[fetch_call(&host.root, sent as i64 + 1, "net-after-overrun")]);
    let closed = session
        .carrier()
        .wait_closed(SOON)
        .expect("the overrun closes the carrier at once");
    assert_eq!(closed.reason, CloseReason::Local(CloseCode::Normal));
    assert!(session.service().wait_ended(SOON));
    assert_eq!(
        host.script.seen.lock().unwrap().started,
        started,
        "a call after the overrun was run"
    );
    let mut replies = 0;
    while let Some(frame) = raw.try_read() {
        if frame.tag == Tag::SessionReply.byte() || frame.tag == Tag::SessionError.byte() {
            replies += 1;
        }
    }
    assert!(replies < sent, "{replies} replies of {sent}");
}

/// The control lane's twin of the row above (remediation round 4, R4-N1; the reviewer's probe P-I): data replies are held
/// behind the byte bound, and the client then sends more control calls than the session's control reserve and reads
/// none. The control replies go ahead to the carrier, whose control lane is full on the one past the reserve: that is
/// the session host's `Full` as well, and the session ends at once, though data replies are held. The carrier closes
/// `Normal` and not as `Stalled`, the session ends, and a call sent after the overrun is never run.
#[test]
fn a_client_past_its_control_reserve_behind_held_replies_loses_its_session() {
    const SOON: Duration = Duration::from_secs(2);
    let mut policy = SessionPolicy::new(hello());
    policy.limits = small_limits();
    policy.limits.outstanding_calls = 16;
    // Well past `SOON`: a session left open by the overrun ends as `Stalled` here, far later than the row's look.
    policy.stall = Duration::from_secs(5);
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 2);
    let reads: Vec<Frame> = (0..READS)
        .map(|reader| spool_read(reader as i64 + 1, &log_id, reader))
        .collect();
    raw.write(&reads);
    let routed = |session: &StreamSession| {
        let deadline = Instant::now() + WAIT;
        while session.carrier().queued_in() > 0 {
            assert!(Instant::now() < deadline, "the host routed nothing");
            thread::sleep(Duration::from_millis(2));
        }
        // The last frame taken is routed, its reply offered, before the next look.
        thread::sleep(Duration::from_millis(20));
    };
    routed(&session);
    // The data replies are past the byte bound and held by the outbox, not yet read by the client.
    thread::sleep(Duration::from_millis(300));
    assert!(session.carrier().wait_closed(Duration::ZERO).is_none());
    // Control calls past the reserve (answered at once, on the control lane), none of whose replies is read. The
    // target is unknown, so no operation is involved.
    let control_calls = host.policy.limits.control_reserve + 2;
    let first = READS as i64 + 1;
    for at in 0..control_calls {
        if !raw.try_write(&[cancel_call(first + at as i64, 9999)]) {
            break;
        }
        routed(&session);
    }
    assert!(
        session.service().dropped_replies() > 0
            || session.carrier().wait_closed(Duration::ZERO).is_some(),
        "{control_calls} control replies were not past the control lane"
    );
    // A call after the overrun: a session left open runs it.
    let started = host.script.seen.lock().unwrap().started;
    let _ = raw.try_write(&[fetch_call(
        &host.root,
        first + control_calls as i64,
        "net-after-overrun",
    )]);
    let closed = session
        .carrier()
        .wait_closed(SOON)
        .expect("the overrun closes the carrier at once");
    assert_eq!(closed.reason, CloseReason::Local(CloseCode::Normal));
    assert!(session.service().wait_ended(SOON));
    assert_eq!(
        host.script.seen.lock().unwrap().started,
        started,
        "a call after the overrun was run"
    );
}

/// A reply larger than the control lane's byte room (R3-1): two such replies, then reads held behind them and a cancel
/// whose reply the writer's queue has no control room for. The cancel's reply is held by the outbox, and the pacer sends
/// it ahead of the held reads of other targets, as the writer's own rule does (`GwzSplitCommandChannel.md` §3.3 "the
/// oldest unblocked control frame"): it is read right after the large replies, not behind the reads.
#[test]
fn a_pressed_control_reply_goes_ahead_of_held_replies_of_other_targets() {
    const BIG: usize = 10 * 1024 * 1024;
    const SMALL_READS: usize = 30;
    let mut policy = SessionPolicy::new(hello());
    policy.limits.read_bytes = 16 * 1024 * 1024;
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let big = Host::diff_log_of(&session, 1, BIG);
    let small = Host::diff_log(&session, 2);
    raw.write(&[fetch_call(&host.root, 1, "net-1")]);
    host.script.wait("the fetch", |seen| seen.started == 1);
    let mut frames = vec![spool_read(2, &big, 0), spool_read(3, &big, 1)];
    for at in 0..SMALL_READS {
        frames.push(spool_read(at as i64 + 4, &small, at));
    }
    let cancel = SMALL_READS as i64 + 4;
    frames.push(cancel_call(cancel, 1));
    raw.write(&frames);
    host.script
        .wait("the cancel at the handler", |seen| seen.cancel_seen);
    let order: Vec<i64> = (1..=cancel).map(|_| call_id_of(&raw.read())).collect();
    let at = |id: i64| order.iter().position(|&read| read == id).unwrap();
    assert!(
        at(cancel) < 4,
        "the cancel's reply waited behind held spool replies: {order:?}"
    );
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=cancel).collect::<Vec<_>>());
    assert_eq!(session.service().dropped_replies(), 0);
}
