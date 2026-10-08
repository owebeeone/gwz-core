use super::{
    git_turns::GitTurns,
    ssh_channel::{GitService, SshChannel},
};
use gwz_transport::{
    protocol::{Disposition, Effect, Envelope, ErrorCode, Facts, Failure, MessageKind},
    stream::{Error as StreamError, IoState, MessageEndpoint, Stream},
};
use std::{
    collections::VecDeque,
    future::Future,
    io::{self, Read, Write},
    pin::pin,
    task::{Context, Poll},
};
pub(crate) trait ChannelIo {
    type Owner;
    fn poll_open(&mut self) -> io::Result<()>;
    fn write_backend(&mut self, input: &[u8]) -> io::Result<usize>;
    fn read_backend(&mut self, output: &mut [u8]) -> io::Result<usize>;
    fn read_stderr(&mut self, output: &mut [u8]) -> io::Result<usize>;
    fn send_eof(&mut self) -> io::Result<()>;
    fn finish(&mut self) -> io::Result<i32>;
    /// Close at once, the client being done, without the server's EOF: what the
    /// server sends meanwhile is read off and dropped, and only its CHANNEL_CLOSE
    /// is awaited. The status says nothing about the exchange.
    fn finish_early(&mut self) -> io::Result<i32>;
    fn abort(&mut self);
    fn poll_dispose(&mut self) -> io::Result<()>;
    fn force_dispose(&mut self) -> io::Result<()>;
    fn into_owner(self) -> Result<Self::Owner, Self>
    where
        Self: Sized;
}
impl ChannelIo for SshChannel {
    type Owner = super::ssh_connection::SshConnection;
    fn poll_open(&mut self) -> io::Result<()> {
        Self::poll_open(self)
    }
    fn write_backend(&mut self, input: &[u8]) -> io::Result<usize> {
        self.write(input)
    }
    fn read_backend(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.read(output)
    }
    fn read_stderr(&mut self, output: &mut [u8]) -> io::Result<usize> {
        Self::read_stderr(self, output)
    }
    fn send_eof(&mut self) -> io::Result<()> {
        Self::send_eof(self)
    }
    fn finish(&mut self) -> io::Result<i32> {
        Self::finish(self)
    }
    fn finish_early(&mut self) -> io::Result<i32> {
        Self::finish_early(self)
    }
    fn abort(&mut self) {
        Self::abort(self)
    }
    fn poll_dispose(&mut self) -> io::Result<()> {
        Self::poll_dispose(self)
    }
    fn force_dispose(&mut self) -> io::Result<()> {
        Self::force_dispose(self)
    }
    fn into_owner(self) -> Result<Self::Owner, Self> {
        SshChannel::into_session(self)
    }
}
/// Why the pump stopped: its stream, its channel, or a broken invariant. The
/// worker discards a failed exchange whatever the cause.
#[derive(Debug)]
pub(crate) enum PumpError {
    Stream,
    Io,
    Invariant,
}
pub(crate) struct SshPump<C: ChannelIo> {
    stream: Stream,
    endpoint: MessageEndpoint,
    channel: C,
    forward: VecDeque<u8>,
    reverse: VecDeque<u8>,
    stderr: Vec<u8>,
    stderr_truncated: bool,
    saw_stdout: bool,
    mirror_cap: usize,
    stderr_cap: usize,
    end_sent: bool,
    reverse_end_sent: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    finished: bool,
    /// The exchange only reads (upload-pack): the member's Close ends it.
    fetch: bool,
    close_received: bool,
    close_completed: bool,
    closed_drained: bool,
    opened: bool,
    invalidated: bool,
    facts: Facts,
    /// Whose turn the Git exchange is in, when the host asked to know.
    turns: GitTurns,
}
impl<C: ChannelIo> SshPump<C> {
    pub(crate) fn new(
        stream: Stream,
        endpoint: MessageEndpoint,
        channel: C,
        mirror_cap: usize,
        stderr_cap: usize,
    ) -> Self {
        assert!(mirror_cap > 0);
        assert!(stderr_cap > 0);
        Self {
            stream,
            endpoint,
            channel,
            forward: VecDeque::new(),
            reverse: VecDeque::new(),
            stderr: Vec::new(),
            stderr_truncated: false,
            saw_stdout: false,
            mirror_cap,
            stderr_cap,
            end_sent: false,
            reverse_end_sent: false,
            stdout_eof: false,
            stderr_eof: false,
            finished: false,
            fetch: false,
            close_received: false,
            close_completed: false,
            closed_drained: false,
            opened: false,
            invalidated: false,
            facts: Facts::default(),
            turns: GitTurns::untracked(),
        }
    }
    /// Say which Git service the exchange runs. The pump reads its pkt-line
    /// framing, so that the stall clock pauses while the server waits for the
    /// client (GwzRemoteTransportDesign §10.1); without it every live moment is
    /// `Network`. A fetch is also final at the member's Close, where a push is
    /// final at the server's EOF (GwzTransportSshBackgroundCloseDesign §3).
    pub(crate) fn set_service(&mut self, service: GitService) {
        self.turns = GitTurns::new(service);
        self.fetch = matches!(service, GitService::UploadPack);
    }
    /// The channel's close is complete: the connection can be reclaimed.
    pub(crate) fn finished(&self) -> bool {
        self.finished
    }
    /// The channel was abandoned: the connection cannot be reclaimed.
    pub(crate) fn retired(&self) -> bool {
        self.invalidated
    }
    pub(crate) fn set_facts(&mut self, facts: Facts) {
        self.facts = facts;
    }
    /// The channel is open and its command accepted. Until then no Git byte
    /// has been written to it.
    pub(crate) fn opened(&self) -> bool {
        self.opened
    }
    pub(crate) fn deliver(&mut self, message: Envelope) -> Result<(), PumpError> {
        let payload = if message.kind == MessageKind::Data {
            let data = message.data.as_ref().ok_or(PumpError::Invariant)?;
            if self.forward.len().saturating_add(data.payload.len()) > self.mirror_cap {
                return Err(PumpError::Invariant);
            }
            Some(data.payload.clone())
        } else {
            None
        };
        let kind = message.kind;
        if self.endpoint.deliver(message).is_err() {
            self.invalidate_after_error();
            return Err(PumpError::Stream);
        }
        if let Some(payload) = payload {
            self.turns.client_bytes(&payload);
            self.forward.extend(payload);
        }
        if kind == MessageKind::EndWrite {
            self.turns.client_end();
        }
        if kind == MessageKind::Close {
            self.close_received = true;
        }
        // An initiator's Cancel never arrives here: the placement endpoint
        // answers it and cancels the attachment, so the worker discards the
        // connection instead.
        if kind == MessageKind::Failed {
            self.invalidate_after_error();
        }
        Ok(())
    }
    /// Advance before admitting messages as well as before backend I/O: an
    /// incoming Close must not suppress a network deadline already reached.
    pub(crate) fn advance(&self, now: u64) {
        self.endpoint.advance(now);
    }
    pub(crate) fn tick(&mut self, cx: &mut Context<'_>, now: u64) -> Result<(), PumpError> {
        self.advance(now);
        let result = self.tick_inner(cx);
        if result.is_err() {
            self.invalidate_after_error();
        }
        result
    }
    pub(crate) fn poll_next_message(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Envelope>, StreamError>> {
        let future = self.endpoint.next_message();
        let result = pin!(future).poll(cx);
        if let Poll::Ready(Ok(Some(message))) = &result {
            if message.kind == MessageKind::Closed {
                self.closed_drained = true;
            }
        }
        result
    }
    pub(crate) fn cancel(&mut self) {
        self.endpoint.disconnect();
        self.forward.clear();
        self.reverse.clear();
        self.invalidated = true;
        self.channel.abort();
        let _ = self.channel.force_dispose();
    }
    pub(crate) fn poll_dispose(&mut self) -> io::Result<()> {
        self.forward.clear();
        self.reverse.clear();
        self.invalidated = true;
        // The channel aborts itself, after it has read which close is under way.
        self.channel.poll_dispose()
    }
    pub(crate) fn force_dispose(&mut self) -> io::Result<()> {
        self.forward.clear();
        self.reverse.clear();
        self.invalidated = true;
        self.channel.abort();
        self.channel.force_dispose()
    }
    pub(crate) fn into_owner(self) -> Result<C::Owner, Self> {
        if self.finished && self.close_completed && self.closed_drained && !self.invalidated {
            self.channel
                .into_owner()
                .map_err(|channel| Self { channel, ..self })
        } else {
            Err(self)
        }
    }
    fn write_forward(&mut self, cx: &mut Context<'_>) -> Result<(), PumpError> {
        if self.forward.is_empty() {
            if self.endpoint.stats().end_received && !self.end_sent {
                match self.channel.send_eof() {
                    Ok(()) => self.end_sent = true,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(PumpError::Io),
                }
            }
            return Ok(());
        }
        let (first, _) = self.forward.as_slices();
        let count = match self.channel.write_backend(first) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(_) => return Err(PumpError::Io),
        };
        if count == 0 {
            return Err(PumpError::Invariant);
        }
        self.record_progress(count)?;
        let mut consumed = vec![0; count];
        match poll_stream(&self.stream, cx, &mut consumed) {
            Poll::Ready(Ok(read)) if read == count => {
                for _ in 0..count {
                    self.forward.pop_front();
                }
                Ok(())
            }
            Poll::Ready(Ok(_)) => Err(PumpError::Invariant),
            Poll::Ready(Err(_)) => Err(PumpError::Stream),
            Poll::Pending => Err(PumpError::Invariant),
        }
    }
    /// A fetch's member has closed and the client's EOF has gone to the server:
    /// the result is final, and the server's reply is no longer wanted.
    fn closing_early(&self) -> bool {
        self.fetch && self.close_received && self.end_sent
    }
    fn read_reverse(&mut self) -> Result<(), PumpError> {
        if self.reverse.len() >= self.mirror_cap || self.stdout_eof || self.closing_early() {
            return Ok(());
        }
        let mut bytes = vec![0; self.mirror_cap - self.reverse.len()];
        match self.channel.read_backend(&mut bytes) {
            Ok(0) => {
                self.stdout_eof = true;
            }
            Ok(count) => {
                self.report_network()?;
                self.record_progress(count)?;
                self.saw_stdout = true;
                self.turns.server_bytes(&bytes[..count]);
                self.reverse.extend(&bytes[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(PumpError::Io),
        }
        Ok(())
    }
    fn write_reverse(&mut self, cx: &mut Context<'_>) -> Result<(), PumpError> {
        let early = self.closing_early();
        if early {
            // What the server has not yet sent, and what the member has not
            // read, is dropped: the stream ends without it.
            self.reverse.clear();
        }
        if self.reverse.is_empty() {
            if (early || self.stdout_eof && self.stderr_eof) && !self.reverse_end_sent {
                if !early && !self.saw_stdout && !self.stderr_truncated {
                    let message = String::from_utf8_lossy(&self.stderr)
                        .trim()
                        .to_ascii_lowercase();
                    let refused = message == "error: repository not found."
                        || message == "repository not found."
                        || (message.starts_with("error: permission to ")
                            && message.contains(" denied to ")
                            && !message.chars().any(char::is_control));
                    // Local stream-scoped classification only; no server
                    // text crosses this seam, only the typed refusal, which
                    // the worker's version 2 streams carry.
                    if refused {
                        self.endpoint
                            .fail_terminal(Failure {
                                detail: None,
                                setup_cause: None,
                                code: ErrorCode::RepositoryRefused,
                                effect: Effect::None,
                                facts: Some(self.facts.clone()),
                            })
                            .map_err(|_| PumpError::Stream)?;
                        // The terminal now owns the outcome. Do not attempt
                        // EndWrite or a successful service close after it.
                        return Ok(());
                    }
                }
                match poll_end_write(&self.stream, cx) {
                    Poll::Ready(Ok(())) => self.reverse_end_sent = true,
                    Poll::Ready(Err(_)) => return Err(PumpError::Stream),
                    Poll::Pending => {}
                }
            }
            return Ok(());
        }
        let bytes: Vec<u8> = self.reverse.iter().copied().collect();
        match poll_write(&self.stream, cx, &bytes) {
            Poll::Ready(Ok(count)) => {
                if count == 0 {
                    return Err(PumpError::Invariant);
                }
                self.reverse.drain(..count);
            }
            Poll::Ready(Err(_)) => return Err(PumpError::Stream),
            Poll::Pending => {}
        }
        Ok(())
    }
    fn drain_stderr(&mut self) -> Result<(), PumpError> {
        if self.stderr_eof || self.closing_early() {
            return Ok(());
        }
        let mut bytes = [0; 256];
        // Bound work as well as retained bytes: stderr must not starve timers.
        for _ in 0..8 {
            match self.channel.read_stderr(&mut bytes) {
                Ok(0) => {
                    self.stderr_eof = true;
                    return Ok(());
                }
                Ok(count) => {
                    self.report_network()?;
                    self.record_progress(count)?;
                    self.turns.server_stderr();
                    let remaining = self.stderr_cap.saturating_sub(self.stderr.len());
                    self.stderr_truncated |= count > remaining;
                    self.stderr
                        .extend_from_slice(&bytes[..count.min(remaining)]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err(PumpError::Io),
            }
        }
        Ok(())
    }
    /// Closes the channel once the exchange is over on the server's side too:
    /// at its EOF, or, for a fetch whose member has closed, at once. The close
    /// goes on after the member has its Closed, off the member's path.
    fn finish_request(&mut self) -> Result<(), PumpError> {
        if self.finished {
            return Ok(());
        }
        if self.closing_early() {
            // After an early close the server may have been stopped by the
            // client's close, so its status says nothing about the connection.
            return match self.channel.finish_early() {
                Ok(_) => {
                    self.finished = true;
                    Ok(())
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
                Err(_) => Err(PumpError::Io),
            };
        }
        if !(self.end_sent && self.stdout_eof && self.stderr_eof && self.reverse_end_sent) {
            return Ok(());
        }
        match self.channel.finish() {
            Ok(0) => self.finished = true,
            Ok(_) => return Err(PumpError::Invariant),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(_) => return Err(PumpError::Io),
        }
        Ok(())
    }
    /// Completes the member's close. The member's result is final here, and
    /// does not wait for the channel's close: that finishes after it, and the
    /// connection is reusable only if it finishes cleanly.
    fn finish_channel(&mut self) -> Result<(), PumpError> {
        // The client's EOF has gone to the server first: the channel's close
        // needs it, and it is sent only while the member's stream is live.
        if self.close_received
            && !self.close_completed
            && self.end_sent
            && self.endpoint.stats().end_sent
        {
            match self
                .endpoint
                .complete_close(Disposition::Reusable, self.facts.clone())
            {
                Ok(()) => self.close_completed = true,
                Err(StreamError::WouldBlock) => {}
                Err(_) => return Err(PumpError::Stream),
            }
        }
        Ok(())
    }
    fn record_progress(&self, count: usize) -> Result<(), PumpError> {
        if !self.close_received {
            self.endpoint
                .record_io_progress(count)
                .map_err(|_| PumpError::Stream)?;
        }
        Ok(())
    }
    fn set_io_state(&self) -> Result<(), PumpError> {
        // Close owns a distinct cleanup deadline and forbids active-I/O reports.
        if self.close_received {
            return Ok(());
        }
        let state = if self.forward.is_empty()
            && self.reverse.len() >= self.mirror_cap
            && self.stderr_eof
        {
            IoState::Backpressure
        } else if self.forward.is_empty() && self.reverse.is_empty() && self.turns.clients_turn() {
            // The server has finished its turn and the client has it: think
            // time does not start the stall clock.
            IoState::Idle
        } else {
            IoState::Network
        };
        self.endpoint
            .set_io_state(state)
            .map_err(|_| PumpError::Stream)?;
        Ok(())
    }
    /// Backend output is the server's turn: it may end an `Idle` the tick
    /// began with, and its progress is only accepted in `Network`.
    fn report_network(&self) -> Result<(), PumpError> {
        if self.close_received {
            return Ok(());
        }
        self.endpoint
            .set_io_state(IoState::Network)
            .map_err(|_| PumpError::Stream)
    }
    /// Abandon the physical channel. A stream that ended on its own keeps its
    /// terminal message (the I/O clock's Timeout, say) for the peer: a bridged
    /// exchange completes only once that message is delivered, and the peer's
    /// reads have no deadline of their own.
    fn retire_channel(&mut self) {
        self.forward.clear();
        self.reverse.clear();
        self.invalidated = true;
        self.channel.abort();
        let _ = self.channel.force_dispose();
    }
    fn invalidate_after_error(&mut self) {
        self.retire_channel();
        self.endpoint.disconnect();
    }
    fn tick_inner(&mut self, cx: &mut Context<'_>) -> Result<(), PumpError> {
        let snapshot = self.endpoint.stats();
        if snapshot.terminal {
            if !self.close_completed {
                self.retire_channel();
                return Ok(());
            }
            // The member has its Closed. The channel's close goes on.
            return self.finish_request();
        }
        self.set_io_state()?;
        if !self.opened {
            match self.channel.poll_open() {
                Ok(()) => self.opened = true,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err(PumpError::Io),
            }
        }
        self.write_forward(cx)?;
        self.read_reverse()?;
        self.write_reverse(cx)?;
        self.drain_stderr()?;
        self.finish_request()?;
        self.finish_channel()?;
        Ok(())
    }
}
fn poll_stream(
    stream: &Stream,
    cx: &mut Context<'_>,
    output: &mut [u8],
) -> Poll<Result<usize, StreamError>> {
    let future = stream.read(output);
    pin!(future).poll(cx)
}
fn poll_write(
    stream: &Stream,
    cx: &mut Context<'_>,
    input: &[u8],
) -> Poll<Result<usize, StreamError>> {
    let future = stream.write(input);
    pin!(future).poll(cx)
}
fn poll_end_write(stream: &Stream, cx: &mut Context<'_>) -> Poll<Result<(), StreamError>> {
    let future = stream.end_write();
    pin!(future).poll(cx)
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// What the pump's tests observe.
        impl<C: ChannelIo> SshPump<C> {
            pub(crate) fn stream_stats(&self) -> gwz_transport::stream::Snapshot {
                self.endpoint.stats()
            }
            pub(crate) fn io_status(&self) -> gwz_transport::stream::IoStatus {
                self.endpoint.io_status()
            }
            pub(crate) fn forward_len(&self) -> usize {
                self.forward.len()
            }
            pub(crate) fn stderr_len(&self) -> usize {
                self.stderr.len()
            }
            pub(crate) fn channel(&self) -> &C {
                &self.channel
            }
            pub(crate) fn channel_mut(&mut self) -> &mut C {
                &mut self.channel
            }
        }
    }
}
