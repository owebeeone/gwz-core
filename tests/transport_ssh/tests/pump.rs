#![allow(dead_code)]

#[path = "../../../src/git/endpoint/ssh_channel.rs"]
mod ssh_channel;
#[path = "../../../src/git/endpoint/ssh_connection.rs"]
mod ssh_connection;
#[path = "../../../src/git/endpoint/ssh_pump.rs"]
mod ssh_pump;

use gwz_transport::{
    protocol::{Data, EndWrite, Envelope, MessageKind},
    stream::{Config, MessageEndpoint, Side, Stream},
};
use ssh_pump::{ChannelIo, PumpError, SshPump};
use std::{
    collections::VecDeque,
    io,
    task::{Context, Waker},
};

#[derive(Default)]
struct FakeChannel {
    writes: Vec<u8>,
    output: VecDeque<u8>,
    stderr: VecDeque<u8>,
    write_limit: usize,
    write_would_block: bool,
    read_limit: usize,
    open: bool,
    sent_eof: bool,
    output_eof: bool,
    stderr_eof: bool,
    finished: bool,
    disposed: bool,
    aborts: usize,
    open_calls: usize,
    finish_status: i32,
    eof_would_block: bool,
}

impl FakeChannel {
    fn active() -> Self {
        Self {
            open: true,
            write_limit: 2,
            read_limit: 2,
            ..Self::default()
        }
    }
}

impl ChannelIo for FakeChannel {
    type Owner = usize;

    fn poll_open(&mut self) -> io::Result<()> {
        self.open_calls += 1;
        self.open = true;
        Ok(())
    }

    fn write_backend(&mut self, input: &[u8]) -> io::Result<usize> {
        if !self.open || self.disposed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if self.write_would_block {
            self.write_would_block = false;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = input.len().min(self.write_limit.max(1));
        self.writes.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn read_backend(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if let Some(byte) = self.output.pop_front() {
            output[0] = byte;
            let mut count = 1;
            while count < output.len() && count < self.read_limit {
                if let Some(byte) = self.output.pop_front() {
                    output[count] = byte;
                    count += 1;
                } else {
                    break;
                }
            }
            return Ok(count);
        }
        if self.output_eof {
            return Ok(0);
        }
        Err(io::ErrorKind::WouldBlock.into())
    }

    fn read_stderr(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.stderr.is_empty() {
            return if self.stderr_eof {
                Ok(0)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            };
        }
        let count = output.len().min(self.stderr.len());
        for slot in &mut output[..count] {
            *slot = self.stderr.pop_front().expect("bounded stderr");
        }
        Ok(count)
    }

    fn send_eof(&mut self) -> io::Result<()> {
        if self.eof_would_block {
            self.eof_would_block = false;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.sent_eof = true;
        Ok(())
    }

    fn finish(&mut self) -> io::Result<i32> {
        if self.sent_eof && self.output_eof && self.stderr_eof {
            self.finished = true;
            Ok(self.finish_status)
        } else {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    fn abort(&mut self) {
        self.aborts += 1;
        self.open = false;
    }

    fn poll_dispose(&mut self) -> io::Result<()> {
        self.disposed = true;
        Ok(())
    }

    fn force_dispose(&mut self) -> io::Result<()> {
        self.disposed = true;
        Ok(())
    }

    fn is_disposed(&self) -> bool {
        self.disposed
    }

    fn into_owner(self) -> Result<Self::Owner, Self> {
        if self.finished {
            Ok(self.writes.len())
        } else {
            Err(self)
        }
    }
}

fn pair(window: usize) -> (Stream, MessageEndpoint) {
    let mut config = Config::new("pump-session", 1, Side::Endpoint);
    config.receive_window = window;
    config.send_buffer = window;
    config.peer_receive_window = window;
    config.max_payload = window.min(4);
    Stream::new(config).expect("valid pump config")
}

fn data(payload: &[u8], offset: i64) -> Envelope {
    Envelope {
        version: 1,
        session_id: "pump-session".into(),
        stream_id: 1,
        kind: MessageKind::Data,
        data: Some(Data {
            offset,
            payload: payload.to_vec(),
        }),
        ..Default::default()
    }
}

fn end(final_offset: i64) -> Envelope {
    Envelope {
        version: 1,
        session_id: "pump-session".into(),
        stream_id: 1,
        kind: MessageKind::EndWrite,
        end_write: Some(EndWrite { final_offset }),
        ..Default::default()
    }
}

fn cx() -> Context<'static> {
    Context::from_waker(Waker::noop())
}

fn drain_messages<C: ChannelIo>(pump: &mut SshPump<C>, context: &mut Context<'_>) {
    loop {
        match pump.poll_next_message(context) {
            std::task::Poll::Ready(Ok(Some(_))) => {}
            std::task::Poll::Pending | std::task::Poll::Ready(Ok(None)) => return,
            std::task::Poll::Ready(Err(error)) => panic!("message poll failed: {error:?}"),
        }
    }
}

#[test]
fn data_is_validated_before_mirror_and_consumed_after_partial_backend_writes() {
    let (stream, endpoint) = pair(4);
    let mut pump = SshPump::new(stream, endpoint, FakeChannel::active(), 4, 8);
    let mut bad = data(b"bad", 0);
    bad.session_id = "wrong".into();
    assert!(matches!(pump.deliver(bad), Err(PumpError::Stream(_))));
    assert_eq!(pump.forward_len(), 0);
    assert_eq!(pump.channel().writes, []);

    let (stream, endpoint) = pair(4);
    let mut pump = SshPump::new(stream, endpoint, FakeChannel::active(), 4, 8);
    pump.deliver(data(b"abcd", 0)).expect("valid data");
    let mut context = cx();
    for now in 0..8 {
        pump.tick(&mut context, now).expect("partial write drains");
    }
    assert_eq!(pump.channel().writes, b"abcd");
    assert_eq!(pump.forward_len(), 0);
    assert_eq!(pump.stream_stats().consumed, 4);
}

#[test]
fn reverse_credit_does_not_stop_stderr_drain() {
    let (stream, endpoint) = pair(2);
    let mut channel = FakeChannel::active();
    channel.output.extend(b"response".iter().copied());
    channel.stderr.extend(b"diagnostic".iter().copied());
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 2, 8);
    let mut context = cx();
    for now in 0..16 {
        pump.tick(&mut context, now).expect("reverse pump tick");
    }
    assert!(pump.stderr_len() <= 8);
    assert!(pump.stderr_len() > 0);
}

#[test]
fn end_write_waits_for_forward_acceptance_and_terminal_error_discards_mirrors() {
    let (stream, endpoint) = pair(4);
    let mut channel = FakeChannel::active();
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.deliver(data(b"abcd", 0)).unwrap();
    pump.deliver(end(4)).unwrap();
    let mut context = cx();
    for now in 0..16 {
        pump.tick(&mut context, now).expect("half close drains");
        drain_messages(&mut pump, &mut context);
    }
    assert!(pump.channel().sent_eof);
    assert!(pump.channel().finished);

    let (stream, endpoint) = pair(4);
    let mut channel = FakeChannel::active();
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.deliver(data(b"drop", 0)).unwrap();
    pump.cancel();
    assert_eq!(pump.forward_len(), 0);
    assert!(pump.channel().aborts > 0);
    assert!(pump.channel().disposed);
}

#[test]
fn backend_would_block_keeps_reverse_and_stderr_progress_independent() {
    let (stream, endpoint) = pair(4);
    let mut channel = FakeChannel::active();
    channel.write_would_block = true;
    channel.output.extend(b"reply".iter().copied());
    channel.stderr.extend(b"warning".iter().copied());
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.deliver(data(b"req", 0)).unwrap();
    let mut context = cx();
    pump.tick(&mut context, 0)
        .expect("would block is retryable");
    assert_eq!(pump.channel().writes, []);
    assert!(pump.stream_stats().send_buffer > 0);
    assert!(pump.stderr_len() > 0);
    pump.tick(&mut context, 1).expect("retry writes");
    assert!(!pump.channel().writes.is_empty());
}

#[test]
fn network_clock_classification_keeps_pending_stderr_active() {
    let (stream, endpoint) = pair(2);
    let mut channel = FakeChannel::active();
    channel.output.extend(b"xy".iter().copied());
    let mut pump = SshPump::new(stream, endpoint, channel, 2, 8);
    let mut context = cx();
    pump.tick(&mut context, 0).unwrap();
    assert_eq!(
        pump.io_status().state,
        gwz_transport::stream::IoState::Network
    );
}

#[test]
fn nonzero_service_status_poisoned_channel_cannot_be_reused() {
    let (stream, endpoint) = pair(2);
    let mut channel = FakeChannel::active();
    channel.finish_status = 7;
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 2, 8);
    pump.deliver(end(0)).unwrap();
    let mut context = cx();
    for now in 0..8 {
        drain_messages(&mut pump, &mut context);
        if pump.tick(&mut context, now).is_err() {
            assert!(pump.channel().disposed);
            return;
        }
    }
    panic!("nonzero backend status was accepted");
}

#[test]
fn close_uses_cleanup_clock_and_eof_would_block_is_retryable() {
    let (stream, endpoint) = pair(4);
    let mut channel = FakeChannel::active();
    channel.eof_would_block = true;
    channel.output.extend(b"tail".iter().copied());
    channel.output_eof = true;
    channel.stderr_eof = true;
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.deliver(end(0)).unwrap();
    pump.deliver(Envelope {
        version: 1,
        session_id: "pump-session".into(),
        stream_id: 1,
        kind: MessageKind::Close,
        close: Some(gwz_transport::protocol::Close { final_offset: 0 }),
        ..Default::default()
    })
    .unwrap();
    let mut context = cx();
    for now in 0..16 {
        pump.tick(&mut context, now)
            .expect("cleanup remains retryable");
        drain_messages(&mut pump, &mut context);
    }
    assert!(pump.stream_stats().terminal);
    assert!(pump.into_owner().is_ok());
}

#[test]
fn stderr_work_is_bounded_per_turn_and_exact_timeout_prevents_backend_writes() {
    let (stream, endpoint) = pair(4);
    let mut channel = FakeChannel::active();
    channel.stderr.extend(std::iter::repeat_n(b'x', 4096));
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.tick(&mut cx(), 0).unwrap();
    assert_eq!(pump.channel().stderr.len(), 2048);
    assert_eq!(pump.stderr_len(), 8);

    let mut config = Config::new("pump-session", 1, Side::Endpoint);
    config.io_timeout_ms = 5;
    let (stream, endpoint) = Stream::new(config).unwrap();
    let mut pump = SshPump::new(stream, endpoint, FakeChannel::active(), 4, 8);
    pump.tick(&mut cx(), 100).unwrap();
    pump.deliver(data(b"req", 0)).unwrap();
    pump.tick(&mut cx(), 105).unwrap();
    assert!(pump.stream_stats().terminal);
    assert!(pump.channel().writes.is_empty());
    assert!(pump.channel().disposed);
}
