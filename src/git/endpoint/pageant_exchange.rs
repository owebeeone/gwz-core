//! Pageant's request exchange (GwzTransportWindowsParityPlan.md, step 3.5; TR1.8 §5; TR4.8, part 1).
//!
//! Pageant is PuTTY's SSH agent. Its protocol is not a socket: the client creates a shared-memory mapping, writes one
//! agent request into it, and sends the Pageant window a `WM_COPYDATA` message that names the mapping; Pageant opens
//! the mapping, answers into it and returns. [`Exchange::send`] is one such round trip and nothing more. It selects no
//! window, applies no policy about which agent to use, and knows no agent message beyond the lengths and reply types
//! it checks; the session's agent source (step 3.6) is built on it.
//!
//! The rules that make it safe to share with a program the user may not control:
//! - the mapping is a `Local\` name from the runtime's [`IdSource`], created exclusively (a name that already exists is
//!   refused before anything is written), owned by the caller and open to the caller and SYSTEM only;
//! - the frame is a four-byte big-endian length and the request, at most [`MAPPING_SIZE`] bytes in all, so a request
//!   over [`MAX_PAYLOAD`] bytes is refused before it is sent;
//! - one `SendMessageTimeoutW`, with `SMTO_BLOCK | SMTO_ERRORONEXIT`, to the window it is given and never to a
//!   broadcast, and no resend: a sign request that timed out may still be running in Pageant;
//! - after a timeout the mapping is never read and its name is never used again: Pageant may still hold the mapping and
//!   write to it, and a name is used for one request only (the caller draws a new one);
//! - the reply's length must be 1 to [`MAX_PAYLOAD`] bytes and its first byte a message type an agent answers with;
//! - a window of the calling thread is refused, because `SendMessageTimeoutW` ignores its timeout for one.
//!
//! The pure rules below compile everywhere, so that every platform's test run covers them; the OS calls are Windows'.
use gwz_ids::IdSource;
use std::{io, time::Duration};

/// `dwData` of the `COPYDATASTRUCT` Pageant answers (`AGENT_COPYDATA_ID` in PuTTY).
#[allow(dead_code)]
pub(crate) const COPYDATA_ID: usize = 0x804e_50ba;
/// The size of the mapping, which holds the frame of the request and then of the reply.
pub(crate) const MAPPING_SIZE: usize = 8192;
/// The most an agent message (a request or a reply) may hold: the mapping less its four-byte length.
pub(crate) const MAX_PAYLOAD: usize = MAPPING_SIZE - 4;
/// The mapping names this module accepts start with this, in the session-local namespace.
const NAME_PREFIX: &str = "Local\\PageantRequest-";
/// The longest mapping name accepted; ours are about 60 bytes.
const MAX_NAME: usize = 120;
/// The message types an agent answers with: failure, success, identities answer, sign response, extension failure.
const REPLY_TYPES: [u8; 5] = [5, 6, 12, 14, 28];

/// Why an exchange did not produce a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PageantError {
    /// The request is empty or does not fit the mapping's frame. Nothing was sent.
    BadRequest,
    /// The mapping name is not one this module makes. Nothing was sent.
    BadName,
    /// The window belongs to the calling thread, for which the timeout would not apply. Nothing was sent.
    SameQueue,
    /// The window is gone. Nothing was sent.
    NoWindow,
    /// A mapping of that name exists already. It was neither written nor read.
    NameInUse,
    /// The mapping, its security descriptor or the caller's identity could not be made; the Windows error.
    Os(u32),
    /// Pageant did not answer within the bound. The mapping is not read, and Pageant may still be working.
    Timeout,
    /// Pageant answered that it would not serve the request.
    Rejected,
    /// The reply's length is not 1 to [`MAX_PAYLOAD`].
    BadReplyLength(u32),
    /// The reply's first byte is not a message type an agent answers with.
    BadReplyType(u8),
}

impl std::fmt::Display for PageantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest => f.write_str("Pageant request is empty or too large"),
            Self::BadName => f.write_str("Pageant mapping name is not valid"),
            Self::SameQueue => f.write_str("Pageant window belongs to the calling thread"),
            Self::NoWindow => f.write_str("Pageant window is gone"),
            Self::NameInUse => f.write_str("Pageant mapping name is already in use"),
            Self::Os(code) => write!(f, "Pageant mapping failed (Windows error {code})"),
            Self::Timeout => f.write_str("Pageant did not answer in time"),
            Self::Rejected => f.write_str("Pageant refused the request"),
            Self::BadReplyLength(length) => write!(f, "Pageant reply length {length} is not valid"),
            Self::BadReplyType(kind) => {
                write!(f, "Pageant reply type {kind} is not an agent reply")
            }
        }
    }
}
impl std::error::Error for PageantError {}

impl From<PageantError> for io::Error {
    fn from(error: PageantError) -> Self {
        let kind = match error {
            PageantError::Timeout => io::ErrorKind::TimedOut,
            PageantError::BadRequest | PageantError::BadName | PageantError::SameQueue => {
                io::ErrorKind::InvalidInput
            }
            PageantError::NoWindow | PageantError::Rejected => io::ErrorKind::ConnectionAborted,
            PageantError::NameInUse => io::ErrorKind::AlreadyExists,
            PageantError::BadReplyLength(_) | PageantError::BadReplyType(_) => {
                io::ErrorKind::InvalidData
            }
            PageantError::Os(_) => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    }
}

/// The mapping name for the next request of this runtime: `Local\PageantRequest-<pid>-<runtime>-<request>`, where the
/// runtime and request numbers are the [`IdSource`]'s prefix and its next number, so no name is drawn twice.
#[allow(dead_code)]
pub(crate) fn mapping_name(process: u32, ids: &IdSource) -> String {
    format!("{NAME_PREFIX}{process}-{}", ids.unique())
}

/// Whether `name` is a mapping name this module makes: ASCII, bounded, in the `Local\` namespace under Pageant's prefix.
#[allow(dead_code)]
fn valid_name(name: &str) -> bool {
    name.len() <= MAX_NAME
        && name.starts_with(NAME_PREFIX)
        && name.len() > NAME_PREFIX.len()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"\\-_.".contains(&byte))
}

/// The frame for `request`: its length, then the request. The request must hold a message type at least and fit.
#[allow(dead_code)]
fn frame(request: &[u8]) -> Result<Vec<u8>, PageantError> {
    if request.is_empty() || request.len() > MAX_PAYLOAD {
        return Err(PageantError::BadRequest);
    }
    let mut frame = Vec::with_capacity(4 + request.len());
    frame.extend_from_slice(&(request.len() as u32).to_be_bytes());
    frame.extend_from_slice(request);
    Ok(frame)
}

/// The reply in `mapping`, the whole mapping as Pageant left it: the length at its head, then that many bytes of reply,
/// the first of which is a message type an agent answers with.
#[allow(dead_code)]
fn reply(mapping: &[u8]) -> Result<&[u8], PageantError> {
    let head: [u8; 4] = mapping
        .get(..4)
        .and_then(|head| head.try_into().ok())
        .ok_or(PageantError::BadReplyLength(0))?;
    let length = u32::from_be_bytes(head);
    if length == 0 || length as usize > MAX_PAYLOAD {
        return Err(PageantError::BadReplyLength(length));
    }
    let payload = mapping
        .get(4..4 + length as usize)
        .ok_or(PageantError::BadReplyLength(length))?;
    if !REPLY_TYPES.contains(&payload[0]) {
        return Err(PageantError::BadReplyType(payload[0]));
    }
    Ok(payload)
}

/// The security descriptor of a request's mapping: owned by `sid`, with no inherited access, open to SYSTEM and `sid`
/// alone. `sid` is the caller's user SID in its string form.
#[allow(dead_code)]
fn descriptor(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;SY)(A;;GA;;;{sid})")
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod sys;
        // Nothing outside the tests names it until step 3.6's session channel does.
        #[allow(unused_imports)]
        pub(crate) use sys::Exchange;
    }
}

/// A bound for one exchange, from the setup clock's remaining allowance: at least a millisecond, and a whole number of
/// them, as `SendMessageTimeoutW` takes it.
#[allow(dead_code)]
fn timeout_ms(bound: Duration) -> u32 {
    u32::try_from(bound.as_millis().max(1)).unwrap_or(u32::MAX)
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
cfg_if::cfg_if! { if #[cfg(all(test, windows))] { mod pageant_083; mod receiver; mod windows_tests; } }
