//! The writer's byte bound under pressure (remediation of step 2.3g1, finding F1/P2-1). `send_frame` and `try_send`
//! answer `Full` when the queue holds `QUEUE_BYTES`, as before; the control lane has a byte reserve that data never
//! uses, as it has a frame reserve; and the paced send a served session replies through waits for the writer instead,
//! so byte pressure is backpressure, while a lane at its frame count is still `Full` at once.
use super::super::{FrameRefused, Lane, QUEUE_BYTES};
use super::support::*;
use gwz_session_contract::{Frame, FrameKey, Tag, Target};

const MIB: usize = 1024 * 1024;

fn reply(call_id: u64, bytes: usize) -> (Frame, FrameKey) {
    (
        Frame::new(Tag::SessionReply, vec![7; bytes]),
        FrameKey::ordinary(Target::new(call_id)),
    )
}

/// Queues data replies of 1 MiB, then of 4 KiB, until the queue's bytes refuse each size; the far end reads nothing.
/// Returns how many queued. The fill is repeated once the writer is held inside a write, since it may take a frame it
/// had not yet taken when the queue first refused.
fn fill_bytes(carrier: &super::super::StreamCarrier) -> usize {
    let mut queued = 0;
    for (round, size) in [MIB, 4096, MIB, 4096].into_iter().enumerate() {
        if round == 2 {
            eventually("the writer held", || {
                carrier
                    .stalled_for()
                    .filter(|held| *held >= std::time::Duration::from_millis(50))
            });
        }
        loop {
            let (frame, key) = reply(queued as u64 + 1, size);
            match carrier.send_frame(frame, Lane::Data, key) {
                Ok(()) => queued += 1,
                Err(FrameRefused::Full(_)) => break,
                Err(other) => panic!("{other:?}"),
            }
            assert!(
                queued <= 2 * QUEUE_BYTES / 4096,
                "the byte bound never refused"
            );
        }
    }
    queued
}

/// Whether `thread` returns within the rows' bound.
fn finishes<T>(thread: &std::thread::ScopedJoinHandle<'_, T>) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !thread.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    thread.is_finished()
}

/// The control lane's byte reserve: with the queue's bytes full of data, a control reply is still accepted, and a
/// data reply is not.
#[test]
fn a_control_frame_is_accepted_while_data_fills_the_queues_bytes() {
    let (carrier, _from_carrier, _to_carrier) = against_raw(limits(1000, 4));
    let queued = fill_bytes(&carrier);
    assert!(queued < 1000, "the frame count, not the bytes, refused");
    let (frame, key) = reply(5000, 4096);
    carrier
        .send_frame(frame, Lane::Control, key)
        .expect("the control reserve takes a control reply");
    let (frame, key) = reply(5001, 4096);
    assert!(matches!(
        carrier.send_frame(frame, Lane::Data, key),
        Err(FrameRefused::Full(_))
    ));
}

/// A paced send of a frame larger than the queue, behind queued frames, waits while the far end reads nothing, and is
/// written once the far end reads: every frame arrives, in order, the large one last.
#[test]
fn a_paced_send_waits_for_the_writer_and_is_delivered_once_the_peer_reads() {
    let (carrier, mut from_carrier, _to_carrier) = against_raw(limits(1000, 4));
    let mut queued = 0usize;
    loop {
        let (frame, key) = reply(queued as u64 + 1, MIB);
        match carrier.send_frame(frame, Lane::Data, key) {
            Ok(()) => queued += 1,
            Err(FrameRefused::Full(_)) => break,
            Err(other) => panic!("{other:?}"),
        }
    }
    let large = QUEUE_BYTES + MIB;
    std::thread::scope(|scope| {
        let paced = scope.spawn(|| {
            let (frame, key) = reply(9000, large);
            carrier.send_frame_paced(frame, Lane::Data, key)
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!paced.is_finished(), "the paced send did not wait");
        let mut sizes = Vec::new();
        for _ in 0..queued {
            sizes.push(read_wire_frame(&mut from_carrier).1.len());
        }
        // The queue has drained: the waiting sender must have been woken.
        let woken = finishes(&paced);
        if !woken {
            carrier.abort();
        }
        let sent = paced.join().unwrap();
        assert!(
            woken,
            "the paced sender was not woken as the writer drained: {sent:?}"
        );
        assert!(sent.is_ok(), "{sent:?}");
        sizes.push(read_wire_frame(&mut from_carrier).1.len());
        assert_eq!(sizes.last(), Some(&large));
        assert!(sizes[..queued].iter().all(|size| *size == MIB));
    });
}

/// A paced send to a lane at its frame count is `Full` at once: that is the peer's overrun, not byte pressure.
#[test]
fn a_paced_send_at_the_lanes_frame_count_is_full_at_once() {
    let (carrier, _from_carrier, _to_carrier) = against_raw(limits(4, 4));
    let mut queued = 0usize;
    loop {
        let (frame, key) = reply(queued as u64 + 1, 60_000);
        match carrier.send_frame(frame, Lane::Data, key) {
            Ok(()) => queued += 1,
            Err(FrameRefused::Full(_)) => break,
            Err(other) => panic!("{other:?}"),
        }
        assert!(queued < 1000, "an unread flood must hit a bound");
    }
    // The writer may still take a frame it had not yet taken when the lane first answered `Full`: wait until it is
    // held inside a write, and fill the lane again.
    eventually("the writer held", || {
        carrier
            .stalled_for()
            .filter(|held| *held >= std::time::Duration::from_millis(50))
    });
    loop {
        let (frame, key) = reply(queued as u64 + 1, 60_000);
        match carrier.send_frame(frame, Lane::Data, key) {
            Ok(()) => queued += 1,
            Err(FrameRefused::Full(_)) => break,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert!(queued * 60_000 < QUEUE_BYTES);
    std::thread::scope(|scope| {
        let paced = scope.spawn(|| {
            let (frame, key) = reply(9000, 60_000);
            carrier.send_frame_paced(frame, Lane::Data, key)
        });
        let at_once = finishes(&paced);
        // A sender that waited is released, so a failing row does not hang.
        carrier.abort();
        let sent = paced.join().unwrap();
        assert!(at_once, "the paced send waited at a full lane: {sent:?}");
        assert!(matches!(sent, Err(FrameRefused::Full(_))), "{sent:?}");
    });
}

/// A paced sender waiting for room learns the carrier's end: its frame comes back `Closed`.
#[test]
fn a_paced_sender_waiting_for_room_ends_closed_when_the_carrier_ends() {
    let (carrier, _from_carrier, _to_carrier) = against_raw(limits(1000, 4));
    fill_bytes(&carrier);
    std::thread::scope(|scope| {
        let paced = scope.spawn(|| {
            let (frame, key) = reply(9000, MIB);
            carrier.send_frame_paced(frame, Lane::Data, key)
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!paced.is_finished(), "the paced send did not wait");
        carrier.abort();
        assert!(matches!(
            paced.join().unwrap(),
            Err(FrameRefused::Closed(..))
        ));
    });
}
