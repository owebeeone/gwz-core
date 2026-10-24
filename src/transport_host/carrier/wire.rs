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
//! **The `SessionOpen` of the client placement** carries no environment. The parked schema is
//! `{protocol (1), client (2), environment (3, optional), limits (4, optional), attributes (5), host_environment (6,
//! optional)}`; the client placement's shape keeps keys 1, 2 and 4 and nothing else. Key 3 is the environment
//! snapshot, 6 its marker, and 5 the client's process attributes (umask, OpenSSL identity, the off switch), which are
//! facts about the client's process that the core must not adopt. Any other key is refused too, so a field added to
//! the schema later does not start to cross by accident. Both ends check it: the sender before a byte is written, and
//! the receiver before the frame is delivered.
use super::{Attachment, CloseCode, Lane, ProtocolFault};
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use gwz_transport::codec;
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
    /// A key outside the shape: the environment (3), the client's process attributes (5), the marker (6) or any other.
    ForbiddenField(u64),
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
    let message = decode_attachment_any_class(frame)?;
    // The class is the tag's: a message under the other class's tag would be counted against, and ordered with, the
    // wrong lane.
    if tag_of(Lane::of(message.1.kind)) != frame.tag {
        return Err(ProtocolFault::Message);
    }
    Ok(message)
}

/// [`decode_attachment`] without the check that the kind is under its own class's tag, for a router that judges it
/// against the message's binding (a wrong class faults that binding, not the session).
pub(super) fn decode_attachment_any_class(frame: &Frame) -> Result<Attachment, ProtocolFault> {
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
    let envelope = codec::decode(encoded).map_err(|_| ProtocolFault::Message)?;
    Ok((name.to_owned(), envelope))
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

/// Checks that `body`, a `SessionOpen`, is of the client placement's shape. It reads the CBOR structure and nothing
/// else: no value is decoded, and nothing is allocated for it.
pub(crate) fn validate_session_open(body: &[u8]) -> Result<(), SessionOpenRefusal> {
    const ALLOWED: [u64; 3] = [1, 2, 4];
    const REQUIRED: [u64; 2] = [1, 2];
    let mut reader = Reader { bytes: body, at: 0 };
    let (major, entries) = reader.head()?;
    if major != 5 {
        return Err(SessionOpenRefusal::NotAMap);
    }
    if entries > body.len() as u64 {
        return Err(SessionOpenRefusal::Malformed);
    }
    let mut seen = Vec::new();
    for _ in 0..entries {
        let (key_major, key) = reader.head()?;
        if key_major != 0 {
            return Err(SessionOpenRefusal::Malformed);
        }
        if seen.contains(&key) {
            return Err(SessionOpenRefusal::DuplicateField(key));
        }
        if !ALLOWED.contains(&key) {
            return Err(SessionOpenRefusal::ForbiddenField(key));
        }
        seen.push(key);
        reader.skip(0)?;
    }
    if reader.at != body.len() {
        return Err(SessionOpenRefusal::Malformed);
    }
    match REQUIRED.iter().find(|key| !seen.contains(key)) {
        Some(&key) => Err(SessionOpenRefusal::MissingField(key)),
        None => Ok(()),
    }
}

/// A walk over definite-length CBOR.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    const MAX_DEPTH: u32 = 16;

    fn take(&mut self, count: u64) -> Result<&[u8], SessionOpenRefusal> {
        let count = usize::try_from(count).map_err(|_| SessionOpenRefusal::Malformed)?;
        let end = self
            .at
            .checked_add(count)
            .ok_or(SessionOpenRefusal::Malformed)?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(SessionOpenRefusal::Malformed)?;
        self.at = end;
        Ok(slice)
    }

    /// The major type and argument of the next item's head. Indefinite lengths and reserved forms are malformed:
    /// the encoding is deterministic.
    fn head(&mut self) -> Result<(u8, u64), SessionOpenRefusal> {
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
            _ => return Err(SessionOpenRefusal::Malformed),
        };
        Ok((major, argument))
    }

    /// Steps over one item and everything inside it.
    fn skip(&mut self, depth: u32) -> Result<(), SessionOpenRefusal> {
        if depth > Self::MAX_DEPTH {
            return Err(SessionOpenRefusal::Malformed);
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
                for _ in 0..items.ok_or(SessionOpenRefusal::Malformed)? {
                    self.skip(depth + 1)?;
                }
                Ok(())
            }
            _ => self.skip(depth + 1),
        }
    }
}
