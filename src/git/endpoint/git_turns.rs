//! Whose turn a Git exchange is in, read from the pkt-line framing that the
//! SSH pump carries.
//!
//! GwzRemoteTransportDesign §10.1: `Network` means an outstanding backend
//! operation can make peer progress, and application think time must not start
//! the endpoint's stall clock. The pump moves bytes, not requests, so it learns
//! whose turn it is from Git's own framing. It is the server's turn while the
//! server owes a response; it is the client's turn only once the server has
//! positively finished one and the client has sent nothing since.
//!
//! Only protocol v0/v1 upload-pack and receive-pack are read. That is all
//! libgit2 speaks over SSH: it never asks for protocol v2. Anything else ends
//! the tracking, and the exchange stays the server's turn to the end, as it
//! was before this tracker existed: protocol v2, a line out of place, a
//! malformed or oversized length, stderr while the client should be thinking,
//! the client's half-close, and on a push everything from the client's first
//! byte (its commands are followed by a raw pack). Parsing keeps at most
//! `PREFIX` bytes of any packet and never panics.
use super::ssh_channel::GitService;

/// Git's largest packet, header included (`LARGE_PACKET_MAX`, pkt-line.h).
const LARGE_PACKET_MAX: usize = 65520;
/// Payload bytes kept to classify a packet. The longest line read whole is a
/// want line with its capabilities; a longer one ends the tracking.
const PREFIX: usize = 1024;

pub(crate) struct GitTurns {
    fetch: bool,
    client: Framing,
    server: Framing,
    stopped: bool,
    server_started: bool,
    /// The advertisement's closing flush has arrived.
    advertised: bool,
    client_started: bool,
    /// The client's last packet was a flush and nothing has followed it.
    client_rested: bool,
    wants_done: bool,
    wants: u32,
    deepen: bool,
    /// The shallow list a deepen request is owed has ended with its flush.
    shallow_listed: bool,
    /// Have rounds the client has flushed, and those the server has answered.
    rounds: u32,
    naks: u32,
}

impl GitTurns {
    pub(crate) fn new(service: GitService) -> Self {
        Self {
            fetch: matches!(service, GitService::UploadPack),
            client: Framing::default(),
            server: Framing::default(),
            stopped: false,
            server_started: false,
            advertised: false,
            client_started: false,
            client_rested: false,
            wants_done: false,
            wants: 0,
            deepen: false,
            shallow_listed: false,
            rounds: 0,
            naks: 0,
        }
    }

    /// A tracker that has stopped before it began: every moment is the server's.
    pub(crate) fn untracked() -> Self {
        let mut turns = Self::new(GitService::UploadPack);
        turns.stop();
        turns
    }

    /// True only while the server is known to be waiting for the client.
    pub(crate) fn clients_turn(&self) -> bool {
        self.waiting_for_client() && self.server.at_boundary()
    }

    /// Bytes the client sent toward the server, in order.
    pub(crate) fn client_bytes(&mut self, bytes: &[u8]) {
        if self.stopped || bytes.is_empty() {
            return;
        }
        self.client_started = true;
        self.client_rested = false;
        if !self.fetch {
            // receive-pack: commands, then a raw pack, then the server's report.
            self.stop();
            return;
        }
        let mut input = bytes;
        while !self.stopped {
            match self.client.next(&mut input) {
                Err(Malformed) => self.stop(),
                Ok(None) => break,
                Ok(Some(Frame::Flush)) => {
                    self.client_flush();
                    self.client_rested = true;
                }
                Ok(Some(Frame::Line { length })) => {
                    self.client_rested = false;
                    let negotiating = self.wants_done;
                    let line = whole_line(&self.client.prefix, length)
                        .map(|line| classify_client(line, negotiating));
                    match line {
                        Some(line) => self.client_line(line),
                        None => self.stop(),
                    }
                }
            }
        }
        self.client_rested &= self.client.at_boundary();
    }

    /// Bytes the server wrote to its standard output, in order.
    pub(crate) fn server_bytes(&mut self, bytes: &[u8]) {
        let mut input = bytes;
        while !self.stopped {
            match self.server.next(&mut input) {
                Err(Malformed) => self.stop(),
                Ok(None) => break,
                Ok(Some(frame)) => self.server_frame(frame),
            }
        }
    }

    /// The server wrote to standard error.
    pub(crate) fn server_stderr(&mut self) {
        if self.clients_turn() {
            self.stop();
        }
    }

    /// The client half-closed: whatever follows is the server's.
    pub(crate) fn client_end(&mut self) {
        self.stop();
    }

    fn waiting_for_client(&self) -> bool {
        if self.stopped || !self.advertised {
            return false;
        }
        if !self.client_started {
            // Choosing wants, or building the pack to push.
            return true;
        }
        self.fetch
            && self.client_rested
            && self.wants_done
            && (!self.deepen || self.shallow_listed)
            && self.naks == self.rounds
    }

    fn stop(&mut self) {
        self.stopped = true;
    }

    fn client_flush(&mut self) {
        if !self.wants_done {
            self.wants_done = true;
            if self.wants == 0 {
                // No wants: upload-pack returns without negotiating
                // (upload-pack.c, `upload_pack`).
                self.stop();
            }
            return;
        }
        match self.rounds.checked_add(1) {
            Some(rounds) => self.rounds = rounds,
            None => self.stop(),
        }
    }

    fn client_line(&mut self, line: ClientLine) {
        match line {
            ClientLine::Want => match self.wants.checked_add(1) {
                Some(wants) => self.wants = wants,
                None => self.stop(),
            },
            ClientLine::Deepen => self.deepen = true,
            ClientLine::Quiet => {}
            // After `done` the server answers and sends the pack without
            // reading again (`get_common_commits`, then `create_pack_file`).
            ClientLine::Done | ClientLine::Unknown => self.stop(),
        }
    }

    fn server_frame(&mut self, frame: Frame) {
        if !self.advertised {
            match frame {
                Frame::Flush => self.advertised = true,
                Frame::Line { .. } => {
                    let first = !self.server_started;
                    self.server_started = true;
                    if !advertisement_line(&self.server.prefix, first) {
                        self.stop();
                    }
                }
            }
            return;
        }
        if !self.fetch || !self.wants_done || self.waiting_for_client() {
            // The server spoke out of turn.
            self.stop();
            return;
        }
        if self.deepen && !self.shallow_listed {
            // `receive_needs` answers a deepen request with shallow and
            // unshallow lines, then a flush, before negotiation begins.
            match frame {
                Frame::Flush => self.shallow_listed = true,
                Frame::Line { length } => {
                    let listed = whole_line(&self.server.prefix, length).is_some_and(|line| {
                        line.starts_with(b"shallow ") || line.starts_with(b"unshallow ")
                    });
                    if !listed {
                        self.stop();
                    }
                }
            }
            return;
        }
        let Frame::Line { length } = frame else {
            self.stop();
            return;
        };
        match whole_line(&self.server.prefix, length).map(classify_reply) {
            // `get_common_commits` ends its reply to each flushed round with
            // NAK when multi_ack is on, and without it until the first common
            // commit: always one NAK per round, and never one we did not ask for.
            Some(Reply::Nak) if self.naks < self.rounds => self.naks += 1,
            // ACK common/continue/ready answer haves within a round.
            Some(Reply::AckStatus) => {}
            // A bare ACK before `done` means no NAK ends the later rounds
            // (no multi_ack, or no-done after ready): stop counting.
            _ => self.stop(),
        }
    }
}

#[derive(Debug)]
struct Malformed;

enum Frame {
    Flush,
    Line { length: usize },
}

/// One direction's pkt-line framing, read across arbitrary chunk boundaries.
#[derive(Default)]
struct Framing {
    header: [u8; 4],
    header_len: usize,
    /// Payload bytes of the current packet still to arrive.
    remaining: usize,
    length: usize,
    /// Up to `PREFIX` bytes of the current packet's payload.
    prefix: Vec<u8>,
}

impl Framing {
    fn at_boundary(&self) -> bool {
        self.header_len == 0 && self.remaining == 0
    }

    /// The next whole packet in `input`, consuming what it reads.
    fn next(&mut self, input: &mut &[u8]) -> Result<Option<Frame>, Malformed> {
        loop {
            if self.remaining == 0 {
                while self.header_len < self.header.len() {
                    let Some((&byte, rest)) = input.split_first() else {
                        return Ok(None);
                    };
                    *input = rest;
                    self.header[self.header_len] = byte;
                    self.header_len += 1;
                }
                self.header_len = 0;
                self.prefix.clear();
                match packet_length(&self.header).ok_or(Malformed)? {
                    0 => return Ok(Some(Frame::Flush)),
                    // Protocol v2's delim and response-end, or invalid.
                    1..=3 => return Err(Malformed),
                    4 => return Ok(Some(Frame::Line { length: 0 })),
                    length if length > LARGE_PACKET_MAX => return Err(Malformed),
                    length => {
                        self.remaining = length - 4;
                        self.length = length - 4;
                    }
                }
            }
            let take = self.remaining.min(input.len());
            if take == 0 {
                return Ok(None);
            }
            let (chunk, rest) = input.split_at(take);
            *input = rest;
            let room = PREFIX.saturating_sub(self.prefix.len());
            self.prefix.extend_from_slice(&chunk[..take.min(room)]);
            self.remaining -= take;
            if self.remaining == 0 {
                return Ok(Some(Frame::Line {
                    length: self.length,
                }));
            }
        }
    }
}

fn packet_length(header: &[u8; 4]) -> Option<usize> {
    header.iter().try_fold(0_usize, |length, &byte| {
        let digit = char::from(byte).to_digit(16)?;
        Some(length * 16 + digit as usize)
    })
}

/// A packet's payload without Git's optional trailing newline, or None when it
/// was longer than `PREFIX` and so was not kept whole.
fn whole_line(prefix: &[u8], length: usize) -> Option<&[u8]> {
    if length > prefix.len() {
        return None;
    }
    Some(prefix.strip_suffix(b"\n").unwrap_or(prefix))
}

/// Advertisement lines are refs, capabilities and shallow grafts; only a
/// protocol version other than 1, or an error, ends the tracking.
fn advertisement_line(line: &[u8], first: bool) -> bool {
    if line.starts_with(b"ERR ") {
        return false;
    }
    if first && line.starts_with(b"version ") {
        return matches!(line, b"version 1" | b"version 1\n");
    }
    true
}

enum ClientLine {
    Want,
    Deepen,
    /// Lines the server does not answer on their own.
    Quiet,
    Done,
    Unknown,
}

/// The lines `receive_needs` reads before the first flush, then the lines
/// `get_common_commits` reads after it.
fn classify_client(line: &[u8], negotiating: bool) -> ClientLine {
    if negotiating {
        return if line.starts_with(b"have ") {
            ClientLine::Quiet
        } else if line == b"done" {
            ClientLine::Done
        } else {
            ClientLine::Unknown
        };
    }
    if let Some(rest) = line.strip_prefix(b"want ") {
        // no-done lets the server skip the round-ending NAK (upload-pack.c).
        return if rest
            .split(|&byte| byte == b' ')
            .any(|token| token == b"no-done")
        {
            ClientLine::Unknown
        } else {
            ClientLine::Want
        };
    }
    if line.starts_with(b"deepen ")
        || line.starts_with(b"deepen-since ")
        || line.starts_with(b"deepen-not ")
    {
        ClientLine::Deepen
    } else if line.starts_with(b"shallow ") || line.starts_with(b"filter ") {
        ClientLine::Quiet
    } else {
        ClientLine::Unknown
    }
}

enum Reply {
    Nak,
    AckStatus,
    Other,
}

fn classify_reply(line: &[u8]) -> Reply {
    if line == b"NAK" {
        return Reply::Nak;
    }
    let Some(rest) = line.strip_prefix(b"ACK ") else {
        return Reply::Other;
    };
    let mut fields = rest.split(|&byte| byte == b' ');
    let _oid = fields.next();
    match (fields.next(), fields.next()) {
        (Some(b"common" | b"continue" | b"ready"), None) => Reply::AckStatus,
        _ => Reply::Other,
    }
}
