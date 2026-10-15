//! The SSH pump's stall clock against Git's turns (GwzRemoteTransportDesign
//! §10.1): the client's think time never runs it, and a server that stalls in
//! its own turn always meets it, with the timeout reaching the client.
use super::{
    ssh_channel::GitService,
    ssh_pump::{ChannelIo, SshPump},
};
use gwz_transport::{
    protocol::{Data, Envelope, ErrorCode, MessageKind, Window},
    stream::{Config, IoState, Side, Stream},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io,
    rc::Rc,
    task::{Context, Poll, Waker},
};

/// The fixture endpoint's stall (`SshEndpointConfig::fixture`).
const STALL_MS: u64 = 3_000;
const A: &str = "1111111111111111111111111111111111111111";

fn pkt(line: &str) -> Vec<u8> {
    let mut bytes = format!("{:04x}", line.len() + 4).into_bytes();
    bytes.extend_from_slice(line.as_bytes());
    bytes
}

fn advertisement() -> Vec<u8> {
    [
        pkt(&format!(
            "{A} HEAD\0multi_ack_detailed side-band-64k ofs-delta\n"
        )),
        pkt(&format!("{A} refs/heads/main\n")),
        b"0000".to_vec(),
    ]
    .concat()
}

/// A server whose output the test releases, and that keeps what it was sent.
#[derive(Clone, Default)]
struct Server {
    stdout: Rc<RefCell<VecDeque<u8>>>,
    stdin: Rc<RefCell<Vec<u8>>>,
    /// The server has exited: both its outputs end once drained (r19stall).
    exited: Rc<std::cell::Cell<bool>>,
    /// Only its standard output has ended; the standard error is open (r19stall P3-3).
    stdout_ended: Rc<std::cell::Cell<bool>>,
    /// The server takes no more of what the client writes (r19stall P3-3).
    blocked: Rc<std::cell::Cell<bool>>,
}

impl ChannelIo for Server {
    type Owner = ();
    fn poll_open(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn write_backend(&mut self, input: &[u8]) -> io::Result<usize> {
        if self.blocked.get() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.stdin.borrow_mut().extend_from_slice(input);
        Ok(input.len())
    }
    fn read_backend(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut stdout = self.stdout.borrow_mut();
        if stdout.is_empty() {
            if self.exited.get() || self.stdout_ended.get() {
                return Ok(0);
            }
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = output.len().min(stdout.len());
        for (slot, byte) in output.iter_mut().zip(stdout.drain(..count)) {
            *slot = byte;
        }
        Ok(count)
    }
    fn read_stderr(&mut self, _: &mut [u8]) -> io::Result<usize> {
        if self.exited.get() {
            return Ok(0);
        }
        Err(io::ErrorKind::WouldBlock.into())
    }
    fn send_eof(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn finish(&mut self) -> io::Result<i32> {
        Err(io::ErrorKind::WouldBlock.into())
    }
    fn finish_early(&mut self) -> io::Result<i32> {
        Err(io::ErrorKind::WouldBlock.into())
    }
    fn abort(&mut self) {}
    fn poll_dispose(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn force_dispose(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn into_owner(self) -> Result<(), Self> {
        Err(self)
    }
}

struct Exchange {
    pump: SshPump<Server>,
    server: Server,
    sent: i64,
    failures: Vec<ErrorCode>,
    /// Data bytes the pump has sent toward the client (r19stall).
    delivered: i64,
}

impl Exchange {
    fn new() -> Self {
        let mut config = Config::new("clock-session", 1, Side::Endpoint);
        config.io_timeout_ms = STALL_MS;
        config.receive_window = 65_536;
        config.send_buffer = 65_536;
        config.peer_receive_window = 65_536;
        let (stream, endpoint) = Stream::new(config).unwrap();
        let server = Server::default();
        let mut pump = SshPump::new(stream, endpoint, server.clone(), 65_536, 65_536);
        pump.set_service(GitService::UploadPack);
        Self {
            pump,
            server,
            sent: 0,
            failures: Vec::new(),
            delivered: 0,
        }
    }
    /// The server has sent its last byte and exited (r19stall).
    fn server_exits(&self) {
        self.server.exited.set(true);
    }
    fn server_writes(&self, bytes: &[u8]) {
        self.server.stdout.borrow_mut().extend(bytes);
    }
    fn client_sends(&mut self, bytes: &[u8]) {
        self.pump
            .deliver(Envelope {
                version: 1,
                session_id: "clock-session".into(),
                stream_id: 1,
                kind: MessageKind::Data,
                data: Some(Data {
                    offset: self.sent,
                    payload: bytes.to_vec(),
                }),
                ..Default::default()
            })
            .unwrap();
        self.sent += i64::try_from(bytes.len()).unwrap();
    }
    /// One pump turn at `now`, then whatever the pump sends toward the client.
    fn tick(&mut self, now: u64) -> IoState {
        let mut context = Context::from_waker(Waker::noop());
        self.pump.tick(&mut context, now).unwrap();
        while let Poll::Ready(Ok(Some(message))) = self.pump.poll_next_message(&mut context) {
            if let Some(data) = &message.data {
                self.delivered += i64::try_from(data.payload.len()).unwrap();
            }
            if let Some(failure) = message.failed {
                self.failures.push(failure.code);
            }
        }
        self.pump.io_status().state
    }
    /// The client's consumer reads everything delivered so far: it grants a fresh receive window (r19stall).
    fn consumer_reads(&mut self) {
        self.pump
            .deliver(Envelope {
                version: 1,
                session_id: "clock-session".into(),
                stream_id: 1,
                kind: MessageKind::Window,
                window: Some(Window {
                    max_offset: self.delivered + 65_536,
                }),
                ..Default::default()
            })
            .unwrap();
    }
    fn timed_out(&self) -> bool {
        self.pump.stream_stats().terminal && self.failures == [ErrorCode::Timeout]
    }
}

#[test]
fn client_think_time_never_runs_the_stall_clock() {
    let mut exchange = Exchange::new();
    exchange.server_writes(&advertisement());
    assert_eq!(exchange.tick(1), IoState::Network);
    assert_eq!(
        exchange.tick(2),
        IoState::Idle,
        "the advertisement is delivered"
    );
    assert_eq!(exchange.tick(10 * STALL_MS), IoState::Idle);

    let round = [
        pkt(&format!("want {A} multi_ack_detailed\n")),
        b"0000".to_vec(),
    ]
    .concat();
    let haves = [pkt(&format!("have {A}\n")), b"0000".to_vec()].concat();
    exchange.client_sends(&[round, haves].concat());
    assert_eq!(exchange.tick(10 * STALL_MS + 1), IoState::Network);
    exchange.server_writes(&pkt("NAK\n"));
    exchange.tick(10 * STALL_MS + 2);
    assert_eq!(
        exchange.tick(10 * STALL_MS + 3),
        IoState::Idle,
        "the next round is the client's"
    );
    assert_eq!(exchange.tick(30 * STALL_MS), IoState::Idle);

    exchange.client_sends(&pkt("done\n"));
    assert_eq!(exchange.tick(30 * STALL_MS + 1), IoState::Network);
    assert!(!exchange.pump.stream_stats().terminal);
    assert!(exchange.failures.is_empty());
    assert!(exchange.server.stdin.borrow().ends_with(b"0009done\n"));
}

#[test]
fn a_server_stall_mid_advertisement_times_out_and_reaches_the_client() {
    let mut exchange = Exchange::new();
    let advertisement = advertisement();
    exchange.server_writes(&advertisement[..advertisement.len() / 2]);
    assert_eq!(exchange.tick(1), IoState::Network);
    exchange.tick(STALL_MS);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(STALL_MS + 1);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

#[test]
fn a_server_stall_after_done_times_out_and_reaches_the_client() {
    let mut exchange = Exchange::new();
    exchange.server_writes(&advertisement());
    exchange.tick(1);
    assert_eq!(exchange.tick(2), IoState::Idle);
    let start = 2 * STALL_MS;
    let request = [
        pkt(&format!("want {A} multi_ack_detailed side-band-64k\n")),
        b"0000".to_vec(),
        pkt("done\n"),
    ]
    .concat();
    exchange.client_sends(&request);
    assert_eq!(exchange.tick(start), IoState::Network);
    // The server answers, then stalls before the pack.
    exchange.server_writes(&pkt("NAK\n"));
    assert_eq!(exchange.tick(start + 1), IoState::Network);
    exchange.tick(start + STALL_MS);
    assert!(!exchange.pump.stream_stats().terminal);
    exchange.tick(start + 1 + STALL_MS);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

/// r19stall. A fetch past `done`: the server is sending the pack and the client's consumer has stopped reading.
fn exchange_in_the_pack() -> (Exchange, u64) {
    exchange_in_the_pack_of(1 << 20)
}

/// The same, with `len` bytes of pack in the server's hands.
fn exchange_in_the_pack_of(len: usize) -> (Exchange, u64) {
    let mut exchange = Exchange::new();
    exchange.server_writes(&advertisement());
    exchange.tick(1);
    assert_eq!(exchange.tick(2), IoState::Idle);
    let start = 2 * STALL_MS;
    let request = [
        pkt(&format!("want {A} multi_ack_detailed side-band-64k\n")),
        b"0000".to_vec(),
        pkt("done\n"),
    ]
    .concat();
    exchange.client_sends(&request);
    assert_eq!(exchange.tick(start), IoState::Network);
    exchange.server_writes(&pkt("NAK\n"));
    exchange.tick(start + 1);
    // The server has much more pack than the buffers hold (with the default `len`).
    exchange.server_writes(&vec![7_u8; len]);
    (exchange, start)
}

#[test]
fn a_consumer_that_stops_reading_does_not_run_the_stall_clock() {
    let (mut exchange, start) = exchange_in_the_pack();
    // The pump fills the client's window and its own buffer, then cannot move another byte.
    let mut now = start + 2;
    for _ in 0..64 {
        exchange.tick(now);
        now += 1;
    }
    assert_eq!(
        exchange.pump.io_status().state,
        IoState::Backpressure,
        "the buffers are full and the consumer is not reading: the server is held by us"
    );
    // The consumer is away for ten stall limits. The server cannot send; that is not the network's fault.
    exchange.tick(now + 10 * STALL_MS);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "stream died: {:?}",
        exchange.failures
    );
    assert!(exchange.failures.is_empty(), "{:?}", exchange.failures);
}

#[test]
fn after_a_long_consumer_pause_a_silent_server_still_times_out() {
    let (mut exchange, start) = exchange_in_the_pack();
    let mut now = start + 2;
    for _ in 0..64 {
        exchange.tick(now);
        now += 1;
    }
    now += 10 * STALL_MS;
    exchange.tick(now);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    // The consumer reads again until the server has nothing left, then the server goes silent.
    for _ in 0..400 {
        exchange.consumer_reads();
        now += 1;
        exchange.tick(now);
    }
    assert!(
        exchange.delivered >= 1 << 20,
        "the consumer drained the pack"
    );
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    // The server sends nothing more: the stall limit, from its last byte, expires.
    exchange.tick(now + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

/// r19stall. The pack has been received and libgit2 resolves deltas (minutes on a large repository) before it closes the
/// stream: the server has exited, the consumer has taken every byte, and nothing is pending on the network.
#[test]
fn local_work_after_the_servers_last_byte_does_not_run_the_stall_clock() {
    let (mut exchange, start) = exchange_in_the_pack();
    exchange.server_exits();
    let mut now = start + 2;
    for _ in 0..400 {
        exchange.consumer_reads();
        now += 1;
        exchange.tick(now);
    }
    assert!(
        exchange.delivered >= 1 << 20,
        "the consumer took the whole pack"
    );
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    assert_eq!(
        exchange.pump.io_status().state,
        IoState::Idle,
        "the server is done and the consumer has everything: only the consumer is working"
    );
    exchange.tick(now + 10 * STALL_MS);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "stream died: {:?}",
        exchange.failures
    );
    assert!(exchange.failures.is_empty(), "{:?}", exchange.failures);
}

/// r19stall. The server has exited and the consumer has not yet taken the last bytes (fewer than the pump's buffer holds).
#[test]
fn a_consumer_that_has_not_taken_the_last_bytes_does_not_run_the_stall_clock() {
    let mut exchange = Exchange::new();
    exchange.server_writes(&advertisement());
    exchange.tick(1);
    assert_eq!(exchange.tick(2), IoState::Idle);
    let start = 2 * STALL_MS;
    let request = [
        pkt(&format!("want {A} multi_ack_detailed side-band-64k\n")),
        b"0000".to_vec(),
        pkt("done\n"),
    ]
    .concat();
    exchange.client_sends(&request);
    exchange.tick(start);
    // 160 KiB: the stream takes 128 KiB (its send buffer and the client's window), the pump holds the other 32 KiB, below its 64 KiB buffer.
    exchange.server_writes(&vec![7_u8; 160 * 1024]);
    exchange.server_exits();
    let mut now = start + 1;
    for _ in 0..64 {
        exchange.tick(now);
        now += 1;
    }
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    exchange.tick(now + 10 * STALL_MS);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "stream died: {:?}",
        exchange.failures
    );
    assert!(exchange.failures.is_empty(), "{:?}", exchange.failures);
}

/// r19stall P3-1. The server has finished its turn (the advertisement's flush is read), the client has it, and the consumer has not taken the
/// last of it: about 31 KiB wait in the pump (the stream takes 128 KiB, its send buffer and the client's window), below its 64 KiB cap. Nothing is pending on the network, so the clock must not run.
#[test]
fn untaken_advertisement_bytes_at_the_clients_turn_do_not_run_the_stall_clock() {
    let mut exchange = Exchange::new();
    let mut advertisement = pkt(&format!(
        "{A} HEAD\0multi_ack_detailed side-band-64k ofs-delta\n"
    ));
    for _ in 0..170 {
        advertisement.extend(pkt(&format!("{A} refs/heads/{}\n", "x".repeat(900))));
    }
    advertisement.extend(b"0000");
    exchange.server_writes(&advertisement);
    let mut now = 1;
    for _ in 0..64 {
        exchange.tick(now);
        now += 1;
    }
    assert_eq!(
        exchange.pump.io_status().state,
        IoState::Backpressure,
        "the server's turn is over and the consumer holds what is left"
    );
    now += 10 * STALL_MS;
    exchange.tick(now);
    assert!(
        !exchange.pump.stream_stats().terminal,
        "stream died: {:?}",
        exchange.failures
    );
    for _ in 0..400 {
        exchange.consumer_reads();
        now += 1;
        exchange.tick(now);
    }
    assert!(
        exchange.delivered >= i64::try_from(advertisement.len()).unwrap(),
        "the consumer took the whole advertisement"
    );
    assert_eq!(
        exchange.tick(now + STALL_MS + 1),
        IoState::Idle,
        "it is still the client's turn: think time"
    );
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
}

/// Fail-closed rows (r19stall P3-3): the cases that must stay `Network`, so that a dead peer is never hidden. Each fills the pump's buffers, lets
/// the consumer take nothing, and expects the stream to time out one stall limit after the last byte the peer moved.
fn assert_network_until_timeout(exchange: &mut Exchange, from: u64) {
    let mut now = from;
    for _ in 0..64 {
        exchange.tick(now);
        now += 1;
    }
    assert_eq!(
        exchange.pump.io_status().state,
        IoState::Network,
        "the peer is the one that is not moving"
    );
    assert!(
        !exchange.pump.stream_stats().terminal,
        "{:?}",
        exchange.failures
    );
    exchange.tick(now + STALL_MS + 2);
    assert!(exchange.timed_out(), "{:?}", exchange.failures);
}

/// (i) Bytes queued toward the server that it does not take (a push, or a negotiation, where it stopped reading), with the reverse buffer full:
/// the server is not blocked by us, we are blocked by it.
#[test]
fn queued_client_bytes_behind_a_full_buffer_still_run_the_stall_clock() {
    let (mut exchange, start) = exchange_in_the_pack();
    exchange.server.blocked.set(true);
    exchange.client_sends(&[1_u8; 512]);
    assert_network_until_timeout(&mut exchange, start + 2);
}

/// (ii) A buffer that is partly full at the server's turn: the server owes the rest of the pack and is silent.
#[test]
fn a_partly_full_buffer_at_the_servers_turn_still_runs_the_stall_clock() {
    let (mut exchange, start) = exchange_in_the_pack_of(160 * 1024);
    assert_network_until_timeout(&mut exchange, start + 2);
}

/// (iii) One output ended and the other open (over libssh2 EOF is per channel, but `server_done` must need both): the server is not done.
#[test]
fn one_output_ended_while_the_other_is_open_still_runs_the_stall_clock() {
    let (mut exchange, start) = exchange_in_the_pack_of(160 * 1024);
    exchange.server.stdout_ended.set(true);
    assert_network_until_timeout(&mut exchange, start + 2);
}
