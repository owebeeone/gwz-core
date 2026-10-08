//! The pump's close: a member's result is final before the server's exit
//! sequence, which continues after the member's Closed
//! (`dev-docs/GwzTransportSshBackgroundCloseDesign.md` §3, §4, §10).
use super::pump::{FakeChannel, close, cx, end, pair};
use crate::git::endpoint::{
    ssh_channel::GitService,
    ssh_pump::{PumpError, SshPump},
};
use gwz_transport::{
    protocol::{ErrorCode, MessageKind},
    stream::{Config, Side, Stream},
};
use std::{io, task::Poll};

/// A server that is still running: no EOF on either output, and its CHANNEL_CLOSE
/// not arrived, so the close cannot finish.
fn server_still_running() -> FakeChannel {
    let mut channel = FakeChannel::active();
    channel.output_eof = false;
    channel.stderr_eof = false;
    channel.finish_blocked = true;
    channel
}

/// A server whose outputs have ended and whose CHANNEL_CLOSE has not arrived.
fn server_closing() -> FakeChannel {
    let mut channel = FakeChannel::active();
    channel.output_eof = true;
    channel.stderr_eof = true;
    channel.finish_blocked = true;
    channel
}

/// What the member has been sent so far: the kinds the pump queued.
#[derive(Default)]
struct Sent {
    kinds: Vec<MessageKind>,
}

impl Sent {
    fn closed(&self) -> bool {
        self.kinds.contains(&MessageKind::Closed)
    }
}

/// Runs the pump `ticks` times, as the worker does, handing the member what
/// it queues. A tick that fails is returned with what was sent before it.
fn run(
    pump: &mut SshPump<FakeChannel>,
    sent: &mut Sent,
    first: u64,
    ticks: u64,
) -> Result<(), PumpError> {
    let mut context = cx();
    for now in first..first + ticks {
        pump.tick(&mut context, now)?;
        while let Poll::Ready(Ok(Some(message))) = pump.poll_next_message(&mut context) {
            sent.kinds.push(message.kind);
        }
    }
    Ok(())
}

fn upload_pack(channel: FakeChannel) -> SshPump<FakeChannel> {
    let (stream, endpoint) = pair(4);
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.set_service(GitService::UploadPack);
    pump
}

fn receive_pack(channel: FakeChannel) -> SshPump<FakeChannel> {
    let (stream, endpoint) = pair(4);
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 8);
    pump.set_service(GitService::ReceivePack);
    pump
}

#[test]
fn upload_pack_member_completes_before_server_close() {
    let mut pump = upload_pack(server_still_running());
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).expect("the early close is not an error");
    assert!(pump.channel().sent_eof);
    assert!(
        sent.closed(),
        "the member waited for the server: {:?}",
        sent.kinds
    );
    assert!(pump.stream_stats().terminal);
    assert!(!pump.finished(), "the server's close has not arrived");
    assert!(pump.into_owner().is_err());
}

#[test]
fn upload_pack_close_leaves_the_channel_unread_while_the_server_acknowledges() {
    let mut pump = upload_pack(server_still_running());
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 8).expect("a closing channel is not read or written");
    pump.channel_mut().finish_blocked = false;
    run(&mut pump, &mut sent, 8, 8).expect("the close finishes");
    assert!(pump.channel().early_finished);
    assert!(pump.finished());
    assert!(pump.into_owner().is_ok());
}

#[test]
fn receive_pack_close_waits_for_eof_not_for_close() {
    // Not before both outputs have ended: the push's result is the server's.
    let mut channel = server_still_running();
    channel.output_eof = true;
    let mut pump = receive_pack(channel);
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).unwrap();
    assert!(!sent.closed(), "closed before the server's stderr ended");
    assert!(
        !pump.channel().closing,
        "the channel closed before the server's EOF"
    );
    // Then at once, while the server's CHANNEL_CLOSE is still to come.
    pump.channel_mut().stderr_eof = true;
    run(&mut pump, &mut sent, 16, 16).unwrap();
    assert!(
        sent.closed(),
        "the member waited for the close: {:?}",
        sent.kinds
    );
    assert!(pump.stream_stats().terminal);
    assert!(!pump.finished());
    assert!(!pump.channel().early_finished, "a push never closes early");
    // The close finishes off the member's path.
    pump.channel_mut().finish_blocked = false;
    run(&mut pump, &mut sent, 32, 8).unwrap();
    assert!(pump.finished());
    assert!(pump.into_owner().is_ok());
}

#[test]
fn a_failed_background_close_is_never_reused() {
    let mut pump = receive_pack(server_closing());
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).unwrap();
    assert!(
        sent.closed(),
        "the member waited for the close: {:?}",
        sent.kinds
    );
    {
        let channel = pump.channel_mut();
        channel.finish_blocked = false;
        channel.finish_error = Some(io::ErrorKind::BrokenPipe);
    }
    let result = run(&mut pump, &mut sent, 16, 4);
    assert!(
        matches!(result, Err(PumpError::Io)),
        "a close that fails after the result is an error: {result:?}"
    );
    assert!(pump.channel().disposed);
    assert!(pump.retired());
    assert!(pump.into_owner().is_err());
}

#[test]
fn nonzero_status_after_completion_keeps_the_result_and_discards() {
    let mut channel = server_closing();
    channel.finish_status = 7;
    let mut pump = receive_pack(channel);
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).unwrap();
    assert!(
        sent.closed(),
        "the member waited for the status: {:?}",
        sent.kinds
    );
    pump.channel_mut().finish_blocked = false;
    let result = run(&mut pump, &mut sent, 16, 4);
    assert!(
        matches!(result, Err(PumpError::Invariant)),
        "a nonzero status after the server's EOF is not a clean close: {result:?}"
    );
    assert!(pump.channel().disposed);
    assert!(pump.into_owner().is_err());
}

#[test]
fn early_close_status_is_not_a_reuse_gate() {
    let mut channel = server_still_running();
    channel.finish_blocked = false;
    // What a server that a client's close stopped with SIGPIPE reports.
    channel.finish_status = 141;
    let mut pump = upload_pack(channel);
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).expect("the status of an early close is ignored");
    assert!(sent.closed());
    assert!(pump.channel().early_finished);
    assert!(pump.into_owner().is_ok());
}

/// The version 2 profile, which carries a refusal as a typed terminal.
fn refusal_pump(channel: FakeChannel) -> SshPump<FakeChannel> {
    let mut config = Config::new("pump-session", 1, Side::Endpoint);
    config.profile_version = 2;
    config.receive_window = 4;
    config.send_buffer = 4;
    config.peer_receive_window = 4;
    config.max_payload = 4;
    let (stream, endpoint) = Stream::new(config).expect("valid pump config");
    let mut pump = SshPump::new(stream, endpoint, channel, 4, 64);
    pump.set_service(GitService::UploadPack);
    pump
}

#[test]
fn a_failed_terminal_ends_the_exchange_at_once() {
    let mut channel = FakeChannel::active();
    channel.stderr.extend(b"repository not found.");
    channel.output_eof = true;
    channel.stderr_eof = true;
    channel.finish_status = 1;
    let mut pump = refusal_pump(channel);
    let mut context = cx();
    let mut refused = false;
    for now in 0..16 {
        pump.tick(&mut context, now).unwrap();
        while let Poll::Ready(Ok(Some(message))) = pump.poll_next_message(&mut context) {
            refused |= message.kind == MessageKind::Failed
                && message
                    .failed
                    .as_ref()
                    .is_some_and(|failure| failure.code == ErrorCode::RepositoryRefused);
        }
        if refused {
            break;
        }
    }
    assert!(refused, "the refusal never reached the member");
    // The worker has handed the terminal over. One tick later nothing of the
    // exchange remains to wait for: the channel is abandoned and not reusable.
    pump.tick(&mut context, 16).unwrap();
    assert!(pump.retired());
    assert!(pump.channel().disposed);
    assert!(pump.into_owner().is_err());
}

/// A push's server can end its output before the client's EOF has gone to it.
/// The member's Closed waits for that EOF, which only a live stream sends, so
/// the channel's close, which needs it, can follow.
#[test]
fn closed_waits_for_the_clients_eof_to_reach_the_server() {
    let mut channel = server_closing();
    channel.eof_withheld = true;
    let mut pump = receive_pack(channel);
    let mut sent = Sent::default();
    pump.deliver(end(0)).unwrap();
    pump.deliver(close(0)).unwrap();
    run(&mut pump, &mut sent, 0, 16).unwrap();
    assert!(
        !sent.closed(),
        "Closed reached the member before the client's EOF reached the server"
    );
    pump.channel_mut().eof_withheld = false;
    pump.channel_mut().finish_blocked = false;
    run(&mut pump, &mut sent, 16, 8).unwrap();
    assert!(sent.closed());
    assert!(pump.finished(), "the close never finished");
    assert!(pump.into_owner().is_ok());
}
