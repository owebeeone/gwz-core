//! The SSH pump's stall clock against Git's turns (GwzRemoteTransportDesign
//! §10.1): the client's think time never runs it, and a server that stalls in
//! its own turn always meets it, with the timeout reaching the client.
use super::{
    ssh_channel::GitService,
    ssh_pump::{ChannelIo, SshPump},
};
use gwz_transport::{
    protocol::{Data, Envelope, ErrorCode, MessageKind},
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
}

impl ChannelIo for Server {
    type Owner = ();
    fn poll_open(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn write_backend(&mut self, input: &[u8]) -> io::Result<usize> {
        self.stdin.borrow_mut().extend_from_slice(input);
        Ok(input.len())
    }
    fn read_backend(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut stdout = self.stdout.borrow_mut();
        if stdout.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = output.len().min(stdout.len());
        for (slot, byte) in output.iter_mut().zip(stdout.drain(..count)) {
            *slot = byte;
        }
        Ok(count)
    }
    fn read_stderr(&mut self, _: &mut [u8]) -> io::Result<usize> {
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
        }
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
            if let Some(failure) = message.failed {
                self.failures.push(failure.code);
            }
        }
        self.pump.io_status().state
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
