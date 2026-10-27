//! What goes on the wire: a transport message as a frame, the close announcement, and the shape of the one registry
//! frame the client placement lets through, `SessionOpen`.
//!
//! **Tags.** The carrier uses the transport lane's two reserved tags, split by class as operator decision D-N states:
//! [`DATA_TAG`] (16) carries a transport message of the data lane (`Data`, `Open`, `CheckIdentity`) and
//! [`CONTROL_TAG`] (17) one of the control lane (everything else), so that the byte-stream writer can put a control
//! frame ahead of queued data. The close announcement is a control frame whose request-name length is zero, which no
//! message has (tags 18 to 31 stay reserved). The registry's own tags (1 to 8) pass through unchanged for the
//! operation service (plan 2.2b); only tag 4 is looked into.
//!
//! **A transport message** is a one-byte request-name length, the name, and the transport's own CBOR encoding of the
//! envelope, which the transport's codec admits against its limits before it allocates a decoded tree.
//!
//! **The `SessionOpen` of the client placement** carries no environment, in the one shape `GwzSplitCommandChannel.md`
//! §3.1 freezes. The parked schema is `{protocol (1), client (2), environment (3, optional), limits (4, optional),
//! attributes (5), host_environment (6, optional)}`, and its `ProcessAttributes` is `{umask (1), transport_off (2),
//! openssl (3)}`. The placement accepts exactly: keys 1 and 2; key 4 if present; key 5 present, a map with at most
//! `transport_off` (2) set to false, never `umask` or `openssl` (facts about the client's process the core must not
//! adopt); key 6 present and `true` (the core's own environment is used). Key 3, the environment snapshot, and any
//! other key are refused, so a field added to the schema later does not start to cross by accident. A missing 5 or 6,
//! and any other value of either, is refused. Both ends check it: the sender before a byte of the frame is written,
//! and the receiver before the frame is delivered; a receiver answers a refusal with one `SessionError`
//! ([`session_error_frame`]) and closes.
use super::{Attachment, CloseCode, Lane, ProtocolFault};
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use gwz_transport::{codec, protocol::Envelope};
use std::hash::{DefaultHasher, Hash, Hasher};

pub(super) const DATA_TAG: u8 = Tag::TransportData.byte();
pub(super) const CONTROL_TAG: u8 = Tag::TransportControl.byte();
/// The largest frame under [`DATA_TAG`] or [`CONTROL_TAG`]: the transport's own frame, its request name and a byte of
/// length. The router refuses a larger one.
pub(super) const MAX_TRANSPORT_FRAME: usize = codec::MAX_FRAME + 1 + 128 + 1;
const MAX_REQUEST_NAME: usize = 128;

/// Why a `SessionOpen` is not of the client placement's shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionOpenRefusal {
    /// The body is not a CBOR map.
    NotAMap,
    /// The body is not well-formed deterministic CBOR of the shape, or has bytes after it.
    Malformed,
    /// A key outside the shape: the environment (3) or any other.
    ForbiddenField(u64),
    /// A key the shape allows, with a value it does not: `attributes` (5) carrying `umask`, `openssl`, a set
    /// `transport_off` or anything but a map, or `host_environment` (6) other than `true`.
    ForbiddenValue(u64),
    /// A key the shape requires is absent.
    MissingField(u64),
    /// A key appears twice.
    DuplicateField(u64),
}

/// The request name rule of the mux: non-empty, at most 128 bytes, no control characters.
fn request_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_REQUEST_NAME && !name.chars().any(char::is_control)
}

pub(super) fn encode_attachment(message: &Attachment) -> Result<Frame, ProtocolFault> {
    if !request_name_ok(&message.0) {
        return Err(ProtocolFault::Message);
    }
    let encoded = codec::encode(&message.1).map_err(|_| ProtocolFault::Message)?;
    let mut body = Vec::with_capacity(1 + message.0.len() + encoded.len());
    body.push(u8::try_from(message.0.len()).map_err(|_| ProtocolFault::Message)?);
    body.extend_from_slice(message.0.as_bytes());
    body.extend_from_slice(&encoded);
    Ok(Frame {
        tag: tag_of(Lane::of(message.1.kind)),
        body,
    })
}

/// How the byte-stream writer orders a transport frame: control by its tag, and the target the stream it belongs to, as
/// the hash of `(request_id, session_id, stream_id)`. Equal triples hash alike within a process; a collision only keeps
/// more order than needed. (Core derives the key because the channel decodes no body, operator decision D-N.)
pub(super) fn key_of(message: &Attachment) -> FrameKey {
    let mut hasher = DefaultHasher::new();
    message.0.hash(&mut hasher);
    message.1.session_id.hash(&mut hasher);
    message.1.stream_id.hash(&mut hasher);
    let target = Target::new(hasher.finish());
    match Lane::of(message.1.kind) {
        Lane::Data => FrameKey::ordinary(target),
        Lane::Control => FrameKey::control(target),
    }
}

/// The tag a message of `lane` travels under.
pub(super) fn tag_of(lane: Lane) -> u8 {
    match lane {
        Lane::Data => DATA_TAG,
        Lane::Control => CONTROL_TAG,
    }
}

pub(super) fn decode_attachment(frame: &Frame) -> Result<Attachment, ProtocolFault> {
    let parts = split_lane(frame)?;
    let envelope = decode_envelope(parts.encoded)?;
    // The class is the tag's: a message under the other class's tag would be counted against, and ordered with, the
    // wrong lane.
    if tag_of(Lane::of(envelope.kind)) != frame.tag {
        return Err(ProtocolFault::Message);
    }
    Ok((parts.name.to_owned(), envelope))
}

/// A transport frame's body, split at the request name: the framing, before any CBOR is read.
pub(super) struct LaneParts<'a> {
    pub(super) name: &'a str,
    pub(super) encoded: &'a [u8],
}

/// The request-name framing of a transport frame. A fault here means the frame has no recognisable shape, which ends
/// the session: nothing in it can be attributed to a binding.
pub(super) fn split_lane(frame: &Frame) -> Result<LaneParts<'_>, ProtocolFault> {
    if frame.tag != DATA_TAG && frame.tag != CONTROL_TAG {
        return Err(ProtocolFault::Unexpected);
    }
    let (&length, rest) = frame.body.split_first().ok_or(ProtocolFault::Message)?;
    let length = usize::from(length);
    if length == 0 || length > MAX_REQUEST_NAME || rest.len() < length {
        return Err(ProtocolFault::Message);
    }
    let (name, encoded) = rest.split_at(length);
    let name = std::str::from_utf8(name).map_err(|_| ProtocolFault::Message)?;
    if !request_name_ok(name) {
        return Err(ProtocolFault::Message);
    }
    Ok(LaneParts { name, encoded })
}

/// The transport's own decode of an envelope, with its validation.
pub(super) fn decode_envelope(encoded: &[u8]) -> Result<Envelope, ProtocolFault> {
    codec::decode(encoded).map_err(|_| ProtocolFault::Message)
}

/// The envelope's `session_id` (key 2) by a shallow walk of the CBOR map: no tree is built and nothing is validated
/// beyond the structure up to that key, so a frame that fails the transport's own decode can still be attributed to
/// the binding it names. `Ok(None)` when the map has no text `session_id` before its end. A body that is not a map, or
/// whose structure breaks before the key is found, has no attribution: a session fault.
pub(super) fn envelope_session_id(encoded: &[u8]) -> Result<Option<&str>, ProtocolFault> {
    const SESSION_ID_KEY: u64 = 2;
    let fault = |_: CborFault| ProtocolFault::Message;
    let mut reader = Reader {
        bytes: encoded,
        at: 0,
    };
    let (major, entries) = reader.head().map_err(fault)?;
    if major != 5 || entries > encoded.len() as u64 {
        return Err(ProtocolFault::Message);
    }
    for _ in 0..entries {
        let (key_major, key) = reader.head().map_err(fault)?;
        if key_major != 0 {
            return Err(ProtocolFault::Message);
        }
        if key == SESSION_ID_KEY {
            let (value_major, length) = reader.head().map_err(fault)?;
            if value_major != 3 {
                return Ok(None);
            }
            let text = reader.take(length).map_err(fault)?;
            return Ok(std::str::from_utf8(text).ok());
        }
        reader.skip(0).map_err(fault)?;
    }
    Ok(None)
}

/// The close announcement: a control frame whose request-name length is zero, then the code.
pub(super) fn close_frame(code: CloseCode) -> Frame {
    Frame {
        tag: CONTROL_TAG,
        body: vec![0, code.wire()],
    }
}

/// Whether `frame` is a close announcement (and so not a message).
pub(super) fn is_close(frame: &Frame) -> bool {
    frame.tag == CONTROL_TAG && frame.body.first() == Some(&0)
}

pub(super) fn parse_close(frame: &Frame) -> Result<CloseCode, ProtocolFault> {
    match (frame.tag, frame.body.as_slice()) {
        (CONTROL_TAG, [0, code]) => CloseCode::from_wire(*code).ok_or(ProtocolFault::Close),
        _ => Err(ProtocolFault::Close),
    }
}

/// Checks that `body`, a `SessionOpen`, is of the client placement's frozen shape (the module documentation). It reads
/// the CBOR structure and the few values the shape fixes, and nothing else: nothing is decoded or allocated for it.
pub(crate) fn validate_session_open(body: &[u8]) -> Result<(), SessionOpenRefusal> {
    const REQUIRED: [u64; 4] = [1, 2, 5, 6];
    let malformed = |_: CborFault| SessionOpenRefusal::Malformed;
    let mut reader = Reader { bytes: body, at: 0 };
    let (major, entries) = reader.head().map_err(malformed)?;
    if major != 5 {
        return Err(SessionOpenRefusal::NotAMap);
    }
    if entries > body.len() as u64 {
        return Err(SessionOpenRefusal::Malformed);
    }
    let mut seen = Vec::new();
    for _ in 0..entries {
        let (key_major, key) = reader.head().map_err(malformed)?;
        if key_major != 0 {
            return Err(SessionOpenRefusal::Malformed);
        }
        if seen.contains(&key) {
            return Err(SessionOpenRefusal::DuplicateField(key));
        }
        seen.push(key);
        match key {
            1 | 2 | 4 => reader.skip(0).map_err(malformed)?,
            5 => check_attributes(&mut reader)?,
            6 => {
                // `true` is the single byte 0xf5.
                if reader.take(1).map_err(malformed)? != [0xf5] {
                    return Err(SessionOpenRefusal::ForbiddenValue(6));
                }
            }
            _ => return Err(SessionOpenRefusal::ForbiddenField(key)),
        }
    }
    if reader.at != body.len() {
        return Err(SessionOpenRefusal::Malformed);
    }
    match REQUIRED.iter().find(|key| !seen.contains(key)) {
        Some(&key) => Err(SessionOpenRefusal::MissingField(key)),
        None => Ok(()),
    }
}

/// `ProcessAttributes` as the shape fixes it: a map with at most `transport_off` (2), and that false.
fn check_attributes(reader: &mut Reader<'_>) -> Result<(), SessionOpenRefusal> {
    const TRANSPORT_OFF: u64 = 2;
    let malformed = |_: CborFault| SessionOpenRefusal::Malformed;
    let (major, entries) = reader.head().map_err(malformed)?;
    if major != 5 || entries > 1 {
        return Err(SessionOpenRefusal::ForbiddenValue(5));
    }
    if entries == 1 {
        let (key_major, key) = reader.head().map_err(malformed)?;
        // `false` is the single byte 0xf4.
        if key_major != 0 || key != TRANSPORT_OFF || reader.take(1).map_err(malformed)? != [0xf4] {
            return Err(SessionOpenRefusal::ForbiddenValue(5));
        }
    }
    Ok(())
}

/// Error code `invalid_request` of `GwzErrorCode`.
pub(crate) const INVALID_REQUEST: u64 = 1;
/// Error code `server_peer_refused` of `GwzErrorCode` (provisional: the schema step 2.2b appends the names).
pub(crate) const SERVER_PEER_REFUSED: u64 = 82;

/// A `SessionError` frame (tag 3) for `call_id` 0 carrying `code` and a fixed short `message` and nothing else: the one
/// frame a host writes to a peer it refuses. The message never repeats anything the peer sent.
pub(crate) fn session_error_frame(code: u64, message: &str) -> Frame {
    let mut body = Vec::new();
    // SessionError { call_id (1): 0, error (2): GwzError { code (1), message (2) } }
    body.extend(put_head(5, 2));
    body.extend(put_head(0, 1));
    body.extend(put_head(0, 0));
    body.extend(put_head(0, 2));
    body.extend(put_head(5, 2));
    body.extend(put_head(0, 1));
    body.extend(put_head(0, code));
    body.extend(put_head(0, 2));
    body.extend(put_head(3, message.len() as u64));
    body.extend_from_slice(message.as_bytes());
    Frame::new(Tag::SessionError, body)
}

/// The head of a CBOR item of `major` type with `argument`, in its shortest form.
fn put_head(major: u8, argument: u64) -> Vec<u8> {
    let major = major << 5;
    match argument {
        0..=23 => vec![major | argument as u8],
        24..=0xff => vec![major | 24, argument as u8],
        0x100..=0xffff => [vec![major | 25], (argument as u16).to_be_bytes().to_vec()].concat(),
        0x1_0000..=0xffff_ffff => {
            [vec![major | 26], (argument as u32).to_be_bytes().to_vec()].concat()
        }
        _ => [vec![major | 27], argument.to_be_bytes().to_vec()].concat(),
    }
}

/// The CBOR structure was broken: a truncated item, a reserved or indefinite form, or too deep.
#[derive(Clone, Copy, Debug)]
struct CborFault;

/// A walk over definite-length CBOR.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const MAX_DEPTH: u32 = 16;

    fn take(&mut self, count: u64) -> Result<&'a [u8], CborFault> {
        let count = usize::try_from(count).map_err(|_| CborFault)?;
        let end = self.at.checked_add(count).ok_or(CborFault)?;
        let slice = self.bytes.get(self.at..end).ok_or(CborFault)?;
        self.at = end;
        Ok(slice)
    }

    /// The major type and argument of the next item's head. Indefinite lengths and reserved forms are malformed:
    /// the encoding is deterministic.
    fn head(&mut self) -> Result<(u8, u64), CborFault> {
        let first = self.take(1)?[0];
        let major = first >> 5;
        let argument = match first & 0x1f {
            small @ 0..=23 => u64::from(small),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?.try_into().expect("two bytes"),
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?.try_into().expect("four bytes"),
            )),
            27 => u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes")),
            _ => return Err(CborFault),
        };
        Ok((major, argument))
    }

    /// Steps over one item and everything inside it.
    fn skip(&mut self, depth: u32) -> Result<(), CborFault> {
        if depth > Self::MAX_DEPTH {
            return Err(CborFault);
        }
        let (major, argument) = self.head()?;
        let remaining = (self.bytes.len() - self.at) as u64;
        match major {
            0 | 1 | 7 => Ok(()),
            2 | 3 => self.take(argument).map(|_| ()),
            4 | 5 => {
                let items = if major == 5 {
                    argument.checked_mul(2)
                } else {
                    Some(argument)
                };
                let items = items.filter(|items| *items <= remaining);
                for _ in 0..items.ok_or(CborFault)? {
                    self.skip(depth + 1)?;
                }
                Ok(())
            }
            _ => self.skip(depth + 1),
        }
    }
}
