//! A push whose producer is busy between its stream calls (GwzPushProducerBusyDesign
//! §3, §4, §7): libgit2 reads a whole object before it writes the object's first
//! byte, and that silence is the producer's, not the network's, for as long as
//! the ceiling allows and no longer.
use super::*;
use crate::git::endpoint::stream_io::ProducerGate;
use gwz_transport::protocol::EndWrite;

/// A ceiling that leaves room for the stall budget after it.
const CEILING_MS: u64 = 10_000;
/// The first ticks of a push, before the client writes.
const START: u64 = 2 * STALL_MS;

/// A producer that has made a call and is between calls: libgit2 with an object to read.
fn busy_gate(ceiling_ms: u64) -> ProducerGate {
    let gate = ProducerGate::new(ceiling_ms);
    drop(gate.enter());
    assert!(gate.busy());
    gate
}

/// The commands and the start of a pack: all the server has seen when the pause begins.
fn pack_head() -> Vec<u8> {
    [
        pkt(&format!(
            "{} {A} refs/heads/blob\0report-status\n",
            "0".repeat(40)
        )),
        b"0000".to_vec(),
        b"PACK\0\0\0\x02\0\0\0\x03".to_vec(),
    ]
    .concat()
}

/// A push at the client's turn, with the gate attached before the client writes. Nothing is sent yet.
fn push_at_the_clients_turn(gate: Option<&ProducerGate>) -> Exchange {
    let mut exchange = Exchange::of(GitService::ReceivePack);
    if let Some(gate) = gate {
        exchange.pump.set_producer(gate.clone());
    }
    exchange.server_writes(&advertisement());
    exchange.tick(1);
    assert_eq!(exchange.tick(2), IoState::Idle);
    exchange
}

/// The commands and the pack head go out at `at`: the last byte the pump moves toward the server.
fn send_head(exchange: &mut Exchange, at: u64) {
    exchange.pump.advance(at);
    exchange.client_sends(&pack_head());
    assert_eq!(
        exchange.tick(at),
        IoState::Network,
        "queued bytes are the server's to take"
    );
}

/// A push whose head has gone out at `START`, and whose producer is now busy.
fn pause_after_the_head(ceiling_ms: u64) -> (Exchange, ProducerGate) {
    let gate = busy_gate(ceiling_ms);
    let mut exchange = push_at_the_clients_turn(Some(&gate));
    send_head(&mut exchange, START);
    (exchange, gate)
}

#[test]
fn a_busy_producer_after_the_pack_began_does_not_run_the_stall_clock() {
    let (mut exchange, _gate) = pause_after_the_head(600_000);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    assert_eq!(exchange.tick(START + 10 * STALL_MS), IoState::Backpressure);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "the producer is reading an object, not the network: {:?}",
        exchange.failures
    );
    assert!(exchange.failures.is_empty());
}

#[test]
fn a_producer_waiting_in_a_call_for_the_server_meets_the_stall_clock() {
    let gate = busy_gate(600_000);
    let mut exchange = push_at_the_clients_turn(Some(&gate));
    send_head(&mut exchange, START);
    // libgit2 has written its pack and waits for the server's report in `read`.
    let _in_read = gate.enter();
    assert_eq!(exchange.tick(START + 1), IoState::Network);
    exchange.tick(START + STALL_MS - 1);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(START + STALL_MS + 1);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn the_ceiling_ends_the_silence_and_the_kept_budget_then_runs() {
    let (mut exchange, _gate) = pause_after_the_head(CEILING_MS);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    assert_eq!(exchange.tick(START + CEILING_MS - 1), IoState::Backpressure);
    assert_eq!(
        exchange.tick(START + CEILING_MS),
        IoState::Network,
        "no byte has moved for the ceiling"
    );
    let left = exchange.pump.io_status().remaining_network_ms;
    assert!(
        0 < left && left <= STALL_MS,
        "the budget the pause kept: {left}"
    );
    exchange.tick(START + CEILING_MS + left - 1);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    exchange.tick(START + CEILING_MS + left + 1);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_byte_restarts_the_episode_so_each_large_object_gets_its_own_ceiling() {
    let (mut exchange, _gate) = pause_after_the_head(CEILING_MS);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    // The first object is written one tick before the ceiling.
    let second = START + CEILING_MS - 1;
    assert_eq!(exchange.tick(second - 1), IoState::Backpressure);
    exchange.pump.advance(second);
    exchange.client_sends(&[7; 4_096]);
    assert_eq!(exchange.tick(second), IoState::Network);
    assert_eq!(exchange.tick(second + 1), IoState::Backpressure);
    assert_eq!(
        exchange.tick(second + CEILING_MS - 1),
        IoState::Backpressure,
        "the second object's pause has a full ceiling"
    );
    assert_eq!(exchange.tick(second + CEILING_MS), IoState::Network);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
}

#[test]
fn the_pack_having_begun_does_not_depend_on_the_turn_tracker() {
    // A stderr line at the client's turn stops the tracker (`git_turns.rs`), as would a line out of place.
    let gate = busy_gate(600_000);
    let mut exchange = push_at_the_clients_turn(Some(&gate));
    exchange.server_writes_stderr(b"remote: hello\n");
    exchange.tick(START - 1);
    // The tracker has stopped: with the client's turn no longer known, the wait is the server's, not `Idle`.
    assert_eq!(exchange.tick(START), IoState::Network);
    send_head(&mut exchange, START);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    exchange.tick(START + 10 * STALL_MS);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "the fix must not vanish with the tracker: {:?}",
        exchange.failures
    );
}

#[test]
fn a_stopped_tracker_does_not_start_the_pack_before_the_clients_first_byte() {
    // The same stderr line stops the tracker, and libgit2 is busy with its delta search: the gate reads busy, but
    // no client byte has gone, so this is the server's wait and the stall clock runs.
    let gate = busy_gate(600_000);
    let mut exchange = push_at_the_clients_turn(Some(&gate));
    exchange.server_writes_stderr(b"remote: hello\n");
    exchange.tick(START - 1);
    assert_eq!(exchange.tick(START), IoState::Network);
    exchange.tick(START - 1 + STALL_MS - 1);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(START - 1 + STALL_MS + 1);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

// The negative controls: each is a state the new arm must leave to `Network`.

#[test]
fn bytes_queued_toward_a_server_that_takes_none_meet_the_stall_clock() {
    let (mut exchange, _gate) = pause_after_the_head(600_000);
    exchange.server.blocked.set(true);
    exchange.pump.advance(START + 1);
    exchange.client_sends(&[7; 4_096]);
    assert_eq!(exchange.tick(START + 1), IoState::Network);
    assert_eq!(
        exchange.tick(START + 2),
        IoState::Network,
        "the bytes are still queued"
    );
    exchange.tick(START + STALL_MS - 1);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(START + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn server_bytes_the_consumer_has_not_all_taken_meet_the_stall_clock() {
    let (mut exchange, _gate) = pause_after_the_head(600_000);
    // More than the stream's window and send buffer take (64 KiB each), and less than they and the pump's
    // buffer hold: some stay with the pump, below its cap, and the consumer grants no window.
    exchange.server_writes(&vec![7; 150 * 1024]);
    for at in 1..=3 {
        exchange.tick(START + at);
    }
    assert_eq!(exchange.tick(START + 4), IoState::Network);
    // The last server byte was read by START + 3.
    exchange.tick(START + STALL_MS);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(START + 4 + STALL_MS);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_fetch_with_a_busy_gate_meets_the_stall_clock() {
    let gate = busy_gate(600_000);
    let mut exchange = Exchange::new();
    exchange.pump.set_producer(gate);
    exchange.server_writes(&advertisement());
    exchange.tick(1);
    assert_eq!(exchange.tick(2), IoState::Idle);
    let request = [
        pkt(&format!("want {A} multi_ack_detailed side-band-64k\n")),
        b"0000".to_vec(),
        pkt("done\n"),
    ]
    .concat();
    exchange.client_sends(&request);
    assert_eq!(exchange.tick(START), IoState::Network);
    assert_eq!(exchange.tick(START + 1), IoState::Network);
    exchange.tick(START + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_finished_pack_meets_the_stall_clock_whatever_the_gate_says() {
    let (mut exchange, _gate) = pause_after_the_head(600_000);
    exchange.pump.advance(START + 1);
    exchange
        .pump
        .deliver(Envelope {
            version: 1,
            session_id: "clock-session".into(),
            stream_id: 1,
            kind: MessageKind::EndWrite,
            end_write: Some(EndWrite {
                final_offset: exchange.sent,
            }),
            ..Default::default()
        })
        .unwrap();
    exchange.tick(START + 1);
    assert_eq!(exchange.tick(START + 2), IoState::Network);
    exchange.tick(START + 1 + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_server_stall_mid_advertisement_meets_the_stall_clock_with_the_gate_busy() {
    let gate = busy_gate(600_000);
    let mut exchange = Exchange::of(GitService::ReceivePack);
    exchange.pump.set_producer(gate);
    let advertisement = advertisement();
    exchange.server_writes(&advertisement[..advertisement.len() / 2]);
    assert_eq!(exchange.tick(1), IoState::Network);
    assert_eq!(
        exchange.tick(2),
        IoState::Network,
        "the server's turn: no client byte has gone"
    );
    exchange.tick(STALL_MS);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(STALL_MS + 1);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_gate_no_call_has_returned_from_leaves_the_pause_to_the_stall_clock() {
    let gate = ProducerGate::new(600_000);
    let mut exchange = push_at_the_clients_turn(Some(&gate));
    send_head(&mut exchange, START);
    assert_eq!(exchange.tick(START + 1), IoState::Network);
    exchange.tick(START + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_push_with_no_gate_is_as_it_was() {
    let mut exchange = push_at_the_clients_turn(None);
    send_head(&mut exchange, START);
    assert_eq!(exchange.tick(START + 1), IoState::Network);
    exchange.tick(START + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn calls_that_move_no_byte_do_not_extend_the_ceiling() {
    let (mut exchange, gate) = pause_after_the_head(CEILING_MS);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    // Every second the producer is caught inside a call, and leaves it again, with no byte written.
    for second in 1..CEILING_MS / 1_000 {
        let at = START + second * 1_000;
        let call = gate.enter();
        exchange.tick(at);
        drop(call);
        assert_eq!(
            exchange.tick(at + 1),
            IoState::Backpressure,
            "second {second}"
        );
    }
    assert_eq!(
        exchange.tick(START + CEILING_MS),
        IoState::Network,
        "only a byte restarts the episode"
    );
}

#[test]
fn server_bytes_from_a_pause_are_progress_and_never_a_wrong_state() {
    let (mut exchange, _gate) = pause_after_the_head(CEILING_MS);
    assert_eq!(exchange.tick(START + 1), IoState::Backpressure);
    assert_eq!(exchange.tick(START + 5_000), IoState::Backpressure);
    // The tick begins in `Backpressure`, which refuses progress; the pump switches to `Network` first.
    exchange.server_writes_stderr(b"remote: still here\n");
    exchange.server_writes(&pkt("ok\n"));
    exchange.tick(START + 5_001);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    assert_eq!(exchange.tick(START + 5_002), IoState::Backpressure);
    // The server's bytes restarted the episode: the old ceiling passes without effect.
    assert_eq!(exchange.tick(START + CEILING_MS + 1), IoState::Backpressure);
    assert_eq!(exchange.tick(START + 5_001 + CEILING_MS), IoState::Network);
}
