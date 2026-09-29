//! Frames, the tag registry and the frame checks (§1, §3; server design §3,
//! as its §8 amends the contract's §3).

use std::error::Error;
use std::fmt;
use std::ops::RangeInclusive;

/// The largest frame, fixed at 64 MiB (§1, §3). It counts a frame's tag byte
/// and its body, which are the bytes a byte stream's length prefix counts.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// The tags reserved for a transport lane (client placement). The contract
/// uses none of them, so a frame there is a protocol error (§3).
pub const RESERVED_TAGS: RangeInclusive<u8> = 16..=31;

/// One frame: a tag byte and a body (§3).
///
/// The body is bytes the session encodes, deterministic CBOR in GWZ's use,
/// and neither this crate nor a channel adapter decodes it. The fields are
/// plain bytes, so a frame can hold any tag and any size. A carrier checks
/// both when the frame crosses it ([`Frame::check`]), and a frame that fails
/// the check ends the session.
///
/// `Debug` shows the tag and the body's length, never the body. A body can
/// carry the environment snapshot, which is secret-bearing (the server
/// design's `SessionOpen`, its §3, and its §8 amendment of the contract's
/// §5.6).
#[derive(Clone, Eq, PartialEq)]
pub struct Frame {
    /// The tag byte. [`Tag`] names the registry's.
    pub tag: u8,
    /// The body.
    pub body: Vec<u8>,
}

impl Frame {
    /// A frame under a registry tag.
    pub fn new(tag: Tag, body: Vec<u8>) -> Self {
        Self {
            tag: tag.byte(),
            body,
        }
    }

    /// The frame's size: its tag byte and its body. A byte stream's length
    /// prefix states it, and [`MAX_FRAME_BYTES`] bounds it.
    pub fn size(&self) -> usize {
        self.body.len().saturating_add(1)
    }

    /// Whether `carrier` carries this frame: its size passes
    /// [`Frame::check_size`], and `carrier` carries its tag
    /// ([`Carrier::tag`]). The size is checked first, as a byte stream's
    /// reader meets the length prefix before the tag. Returns the tag.
    pub fn check(&self, carrier: Carrier) -> Result<Tag, FrameError> {
        Self::check_size(self.size())?;
        carrier.tag(self.tag)
    }

    /// Whether a frame of `size` bytes, its tag byte and its body, is
    /// allowed: at least the tag byte, and at most [`MAX_FRAME_BYTES`]. A
    /// byte stream checks its length prefix with this before it reads on.
    pub const fn check_size(size: usize) -> Result<(), FrameError> {
        if size == 0 {
            Err(FrameError::Empty)
        } else if size > MAX_FRAME_BYTES {
            Err(FrameError::TooLarge { size })
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("tag", &self.tag)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// The tag registry (§3; server design §3 and §8).
///
/// | Tag | Frame | Carried by |
/// | --- | --- | --- |
/// | 1 | `SessionCall` | both adapters |
/// | 2 | `SessionReply` | both adapters |
/// | 3 | `SessionError` | both adapters |
/// | 4 | `SessionOpen` | byte streams |
/// | 5 | `SessionOpened` | byte streams |
/// | 6 | `SessionHello` | byte streams |
/// | 7 | `ServerControl` | byte streams |
/// | 8 | `ServerState` | byte streams |
/// | 16 to 31 | reserved for a transport lane | neither: a protocol error |
/// | any other | none | neither: a protocol error |
///
/// The registry says which tags a carrier carries. Which party may send a
/// tag, and when, is the session's rule, which core enforces: a call's
/// direction (§3) and the handshake's order (server design §3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum Tag {
    /// A call, `SessionCall` (§4.1).
    SessionCall = 1,
    /// A reply, `SessionReply` (§4.1).
    SessionReply = 2,
    /// An error reply, `SessionError` (§4.1).
    SessionError = 3,
    /// The client's handshake frame, `SessionOpen` (server design §3).
    SessionOpen = 4,
    /// The host's answer to it, `SessionOpened` (server design §3).
    SessionOpened = 5,
    /// The host's first frame, `SessionHello` (server design §3).
    SessionHello = 6,
    /// A socket server's control request, `ServerControl` (server design §3).
    ServerControl = 7,
    /// Its answer, `ServerState` (server design §3).
    ServerState = 8,
}

impl Tag {
    /// Every tag in the registry, in byte order.
    pub const ALL: [Tag; 8] = [
        Tag::SessionCall,
        Tag::SessionReply,
        Tag::SessionError,
        Tag::SessionOpen,
        Tag::SessionOpened,
        Tag::SessionHello,
        Tag::ServerControl,
        Tag::ServerState,
    ];

    /// The tag's byte.
    pub const fn byte(self) -> u8 {
        self as u8
    }

    /// The registry's tag for `byte`, whatever the carrier. A byte in
    /// [`RESERVED_TAGS`] is [`FrameError::ReservedTag`], and any other byte
    /// outside the registry [`FrameError::UnknownTag`].
    pub const fn from_byte(byte: u8) -> Result<Tag, FrameError> {
        match byte {
            1 => Ok(Tag::SessionCall),
            2 => Ok(Tag::SessionReply),
            3 => Ok(Tag::SessionError),
            4 => Ok(Tag::SessionOpen),
            5 => Ok(Tag::SessionOpened),
            6 => Ok(Tag::SessionHello),
            7 => Ok(Tag::ServerControl),
            8 => Ok(Tag::ServerState),
            16..=31 => Err(FrameError::ReservedTag(byte)),
            _ => Err(FrameError::UnknownTag(byte)),
        }
    }

    /// Whether only byte streams carry the tag: the server design's
    /// handshake and control frames, tags 4 to 8, which the in-process
    /// adapter never uses (server design §8).
    pub const fn byte_stream_only(self) -> bool {
        matches!(
            self,
            Tag::SessionOpen
                | Tag::SessionOpened
                | Tag::SessionHello
                | Tag::ServerControl
                | Tag::ServerState
        )
    }
}

/// The two adapters of §3. Both carry the same frames, under different parts
/// of the registry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Carrier {
    /// Two queues inside one process: tags 1 to 3.
    InProcess,
    /// A byte stream, such as standard streams, a socket or SSH: tags 1 to 8.
    ByteStream,
}

impl Carrier {
    /// The registry's tag for `byte`, if this carrier carries it. On the
    /// in-process adapter, tags 4 to 8 are [`FrameError::ByteStreamOnly`].
    pub const fn tag(self, byte: u8) -> Result<Tag, FrameError> {
        match Tag::from_byte(byte) {
            Ok(tag) if tag.byte_stream_only() && matches!(self, Carrier::InProcess) => {
                Err(FrameError::ByteStreamOnly(byte))
            }
            answer => answer,
        }
    }
}

/// A frame its carrier may not carry: a protocol error, which ends the
/// session (§3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameError {
    /// A tag outside the registry and the reserved range: "any other tag"
    /// (§3).
    UnknownTag(u8),
    /// A tag in [`RESERVED_TAGS`], which the contract does not use (§3).
    ReservedTag(u8),
    /// A byte-stream tag, 4 to 8, on the in-process adapter (server design
    /// §8).
    ByteStreamOnly(u8),
    /// A frame over [`MAX_FRAME_BYTES`]. `size` counts its tag byte and its
    /// body, as a byte stream's length prefix states them (§3).
    TooLarge {
        /// The frame's size.
        size: usize,
    },
    /// A byte stream's length prefix of zero, which leaves no room for the
    /// tag byte.
    Empty,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::UnknownTag(tag) => {
                write!(
                    formatter,
                    "frame tag {tag} is not in the session's tag registry"
                )
            }
            FrameError::ReservedTag(tag) => {
                write!(
                    formatter,
                    "frame tag {tag} is reserved for a transport lane"
                )
            }
            FrameError::ByteStreamOnly(tag) => {
                write!(formatter, "frame tag {tag} is carried only by byte streams")
            }
            FrameError::TooLarge { size } => write!(
                formatter,
                "a frame of {size} bytes exceeds the 64 MiB frame limit"
            ),
            FrameError::Empty => {
                formatter.write_str("a frame length of zero leaves no room for the tag byte")
            }
        }
    }
}

impl Error for FrameError {}
