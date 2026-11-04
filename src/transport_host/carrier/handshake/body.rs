//! The handshake's bodies, deterministic CBOR (server design §3; `GwzSplitCommandChannel.md` §3.1).
//!
//! **Encoding.** Every body is a definite map whose keys are small unsigned integers in ascending order, and every
//! head is in its shortest form, so one value has one encoding. **Decoding is as strict:** a non-shortest head, an
//! indefinite length, a tag, a float, any simple value but the single bytes `0xf4` and `0xf5`, a key out of order or
//! repeated, a key the body does not define, and any byte after the item are refused, so decoding and re-encoding
//! gives the same bytes. One rule is the exception: an optional boolean (`force`, `transport_off`) spelled out as
//! `false` decodes as false and is encoded absent. The registry's `SessionOpen` is checked for its client-placement
//! shape by `validate_session_open`; this module reads its `protocol` (key 1) and `client` (key 2) under the same
//! canonical rule.
use super::super::wire::put_head;

/// Why a body is not accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyFault {
    /// Not well-formed definite CBOR of the body's shape: truncated, indefinite, a tag or float, too deep, not a map,
    /// text that is not UTF-8.
    Malformed,
    /// A head that is not in its shortest form.
    NonShortest,
    /// A key smaller than the one before it.
    Unsorted(u64),
    /// A key twice.
    Duplicate(u64),
    /// Bytes after the item.
    Trailing,
    /// A key the body does not define.
    Unknown(u64),
    /// A required key that is absent.
    Missing(u64),
    /// A key with a value of the wrong type.
    WrongType(u64),
    /// A key with a value outside its range.
    Range(u64),
}

/// The deepest nesting a body may hold (`limits` and `attributes` are one level).
const MAX_DEPTH: u32 = 4;
/// The most entries in a body's map.
const MAX_ENTRIES: u64 = 16;

struct Strict<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Strict<'a> {
    fn take(&mut self, count: u64) -> Result<&'a [u8], BodyFault> {
        let end = usize::try_from(count)
            .ok()
            .and_then(|count| self.at.checked_add(count))
            .ok_or(BodyFault::Malformed)?;
        let slice = self.bytes.get(self.at..end).ok_or(BodyFault::Malformed)?;
        self.at = end;
        Ok(slice)
    }

    /// The next head, refused unless definite and shortest.
    fn head(&mut self) -> Result<(u8, u64), BodyFault> {
        let first = self.take(1)?[0];
        // Major type 7 holds `false` and `true` only, as the single bytes 0xf4 and 0xf5: no float, no two-byte simple
        // value, no other simple value, whatever its argument.
        if first >> 5 == 7 && !matches!(first, 0xf4 | 0xf5) {
            return Err(BodyFault::Malformed);
        }
        let info = first & 0x1f;
        let (argument, floor) = match info {
            0..=23 => (u64::from(info), 0),
            24 => (u64::from(self.take(1)?[0]), 24),
            25 => (u64::from(u16::from_be_bytes(self.array()?)), 0x100),
            26 => (u64::from(u32::from_be_bytes(self.array()?)), 0x1_0000),
            27 => (u64::from_be_bytes(self.array()?), 0x1_0000_0000),
            _ => return Err(BodyFault::Malformed),
        };
        if argument < floor {
            return Err(BodyFault::NonShortest);
        }
        Ok((first >> 5, argument))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BodyFault> {
        Ok(self.take(N as u64)?.try_into().expect("N bytes"))
    }

    /// Steps over one item, refusing what a deterministic body never holds.
    fn skip(&mut self, depth: u32) -> Result<(), BodyFault> {
        if depth > MAX_DEPTH {
            return Err(BodyFault::Malformed);
        }
        let (major, argument) = self.head()?;
        match major {
            0 | 1 => Ok(()),
            2 => self.take(argument).map(|_| ()),
            3 => std::str::from_utf8(self.take(argument)?)
                .map(|_| ())
                .map_err(|_| BodyFault::Malformed),
            4 => {
                for _ in 0..argument.min(self.bytes.len() as u64 + 1) {
                    self.skip(depth + 1)?;
                }
                Ok(())
            }
            5 => self.entries(argument, depth).map(|_| ()),
            // `false` and `true`, the only major type 7 that `head` lets through; no floats, no tags.
            7 => Ok(()),
            _ => Err(BodyFault::Malformed),
        }
    }

    /// The `count` entries of a map: unsigned keys in ascending order, each with its value's raw encoding.
    fn entries(&mut self, count: u64, depth: u32) -> Result<Vec<(u64, &'a [u8])>, BodyFault> {
        if count > MAX_ENTRIES {
            return Err(BodyFault::Malformed);
        }
        let mut found: Vec<(u64, &[u8])> = Vec::new();
        for _ in 0..count {
            let (major, key) = self.head()?;
            if major != 0 {
                return Err(BodyFault::Malformed);
            }
            match found.last() {
                Some(&(last, _)) if key == last => return Err(BodyFault::Duplicate(key)),
                Some(&(last, _)) if key < last => return Err(BodyFault::Unsorted(key)),
                _ => {}
            }
            let start = self.at;
            self.skip(depth + 1)?;
            found.push((key, &self.bytes[start..self.at]));
        }
        Ok(found)
    }
}

/// The entries of a body's top-level map, each key checked against `defined`.
struct Fields<'a>(Vec<(u64, &'a [u8])>);

impl<'a> Fields<'a> {
    fn parse(body: &'a [u8], defined: &[u64]) -> Result<Self, BodyFault> {
        let mut reader = Strict { bytes: body, at: 0 };
        let (major, count) = reader.head()?;
        if major != 5 {
            return Err(BodyFault::Malformed);
        }
        let entries = reader.entries(count, 0)?;
        if reader.at != body.len() {
            return Err(BodyFault::Trailing);
        }
        if let Some(&(key, _)) = entries.iter().find(|(key, _)| !defined.contains(key)) {
            return Err(BodyFault::Unknown(key));
        }
        Ok(Self(entries))
    }

    fn value<T>(
        &self,
        key: u64,
        read: impl Fn(&'a [u8]) -> Option<T>,
    ) -> Result<Option<T>, BodyFault> {
        match self.0.iter().find(|(found, _)| *found == key) {
            None => Ok(None),
            Some((_, raw)) => read(raw).map(Some).ok_or(BodyFault::WrongType(key)),
        }
    }

    fn need<T>(&self, key: u64, read: impl Fn(&'a [u8]) -> Option<T>) -> Result<T, BodyFault> {
        self.value(key, read)?.ok_or(BodyFault::Missing(key))
    }

    fn uint32(&self, key: u64) -> Result<u32, BodyFault> {
        let value = self.need(key, as_uint)?;
        u32::try_from(value).map_err(|_| BodyFault::Range(key))
    }

    fn opt_uint32(&self, key: u64) -> Result<Option<u32>, BodyFault> {
        self.value(key, as_uint)?
            .map(|value| u32::try_from(value).map_err(|_| BodyFault::Range(key)))
            .transpose()
    }

    fn text(&self, key: u64) -> Result<String, BodyFault> {
        self.need(key, as_text).map(str::to_owned)
    }

    fn raw(&self, key: u64) -> Option<&'a [u8]> {
        self.0
            .iter()
            .find_map(|&(found, raw)| (found == key).then_some(raw))
    }
}

/// The value of an unsigned integer, already known to be canonical.
fn as_uint(raw: &[u8]) -> Option<u64> {
    match (Strict { bytes: raw, at: 0 }).head() {
        Ok((0, value)) => Some(value),
        _ => None,
    }
}

fn as_text(raw: &[u8]) -> Option<&str> {
    let mut reader = Strict { bytes: raw, at: 0 };
    let (major, length) = reader.head().ok()?;
    (major == 3)
        .then(|| std::str::from_utf8(reader.take(length).ok()?).ok())
        .flatten()
}

fn as_bool(raw: &[u8]) -> Option<bool> {
    match raw {
        [0xf4] => Some(false),
        [0xf5] => Some(true),
        _ => None,
    }
}

/// A value to encode.
enum Val<'a> {
    Uint(u64),
    Text(&'a str),
    Bool(bool),
    /// One item already encoded: a map the caller built.
    Raw(&'a [u8]),
}

/// A definite map of `entries`, written in ascending key order whatever order they were given.
fn encode(mut entries: Vec<(u64, Val<'_>)>) -> Vec<u8> {
    entries.sort_by_key(|(key, _)| *key);
    let mut out = put_head(5, entries.len() as u64);
    for (key, value) in entries {
        out.extend(put_head(0, key));
        match value {
            Val::Uint(value) => out.extend(put_head(0, value)),
            Val::Text(text) => {
                out.extend(put_head(3, text.len() as u64));
                out.extend_from_slice(text.as_bytes());
            }
            Val::Bool(value) => out.push(if value { 0xf5 } else { 0xf4 }),
            Val::Raw(raw) => out.extend_from_slice(raw),
        }
    }
    out
}

/// `SessionHello`: the range of protocol versions the host serves, and who it is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Hello {
    pub(crate) protocol_min: u32,
    pub(crate) protocol_max: u32,
    pub(crate) server: String,
    pub(crate) core: String,
    pub(crate) server_pid: u32,
}

impl Hello {
    pub(crate) fn encode(&self) -> Vec<u8> {
        encode(vec![
            (1, Val::Uint(self.protocol_min.into())),
            (2, Val::Uint(self.protocol_max.into())),
            (3, Val::Text(&self.server)),
            (4, Val::Text(&self.core)),
            (5, Val::Uint(self.server_pid.into())),
        ])
    }

    /// The range starts at 1 and is not empty.
    pub(crate) fn decode(body: &[u8]) -> Result<Self, BodyFault> {
        let fields = Fields::parse(body, &[1, 2, 3, 4, 5])?;
        let hello = Self {
            protocol_min: fields.uint32(1)?,
            protocol_max: fields.uint32(2)?,
            server: fields.text(3)?,
            core: fields.text(4)?,
            server_pid: fields.uint32(5)?,
        };
        match (hello.protocol_min, hello.protocol_max) {
            (0, _) => Err(BodyFault::Range(1)),
            (min, max) if max < min => Err(BodyFault::Range(2)),
            _ => Ok(hello),
        }
    }
}

/// `SessionOpened`: the host's answer to a `SessionOpen`, repeating its identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Opened {
    pub(crate) protocol: u32,
    pub(crate) server: String,
    pub(crate) core: String,
    pub(crate) server_pid: u32,
    pub(crate) open_sessions: u32,
}

impl Opened {
    pub(crate) fn encode(&self) -> Vec<u8> {
        encode(vec![
            (1, Val::Uint(self.protocol.into())),
            (2, Val::Text(&self.server)),
            (3, Val::Text(&self.core)),
            (4, Val::Uint(self.server_pid.into())),
            (5, Val::Uint(self.open_sessions.into())),
        ])
    }

    pub(crate) fn decode(body: &[u8]) -> Result<Self, BodyFault> {
        let fields = Fields::parse(body, &[1, 2, 3, 4, 5])?;
        Ok(Self {
            protocol: fields.uint32(1)?,
            server: fields.text(2)?,
            core: fields.text(3)?,
            server_pid: fields.uint32(4)?,
            open_sessions: fields.uint32(5)?,
        })
    }
}

/// What a `ServerControl` asks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Status,
    Stop,
}

/// `ServerControl`: a socket server's status or stop request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Control {
    pub(crate) action: Action,
    pub(crate) force: bool,
}

impl Control {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let action = u64::from(self.action == Action::Stop);
        let mut entries = vec![(1, Val::Uint(action))];
        if self.force {
            entries.push((2, Val::Bool(true)));
        }
        encode(entries)
    }

    /// `force` absent means false, and is written only when true. A spelled-out `false` decodes as false, as the frozen
    /// `SessionOpen`'s `transport_off` does; encoding writes neither (one rule for the optional booleans).
    pub(crate) fn decode(body: &[u8]) -> Result<Self, BodyFault> {
        let fields = Fields::parse(body, &[1, 2])?;
        let action = match fields.need(1, as_uint)? {
            0 => Action::Status,
            1 => Action::Stop,
            _ => return Err(BodyFault::Range(1)),
        };
        let force = fields.value(2, as_bool)?.unwrap_or(false);
        Ok(Self { action, force })
    }
}

/// `ServerState`: a socket server's answer to a `ServerControl`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct State {
    pub(crate) server_pid: u32,
    pub(crate) open_sessions: u32,
    pub(crate) max_sessions: u32,
    pub(crate) idle_exit: Option<u32>,
    pub(crate) log: Option<String>,
    pub(crate) stopping: bool,
}

impl State {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut entries = vec![
            (1, Val::Uint(self.server_pid.into())),
            (2, Val::Uint(self.open_sessions.into())),
            (3, Val::Uint(self.max_sessions.into())),
            (6, Val::Bool(self.stopping)),
        ];
        if let Some(seconds) = self.idle_exit {
            entries.push((4, Val::Uint(seconds.into())));
        }
        if let Some(log) = &self.log {
            entries.push((5, Val::Text(log)));
        }
        encode(entries)
    }

    pub(crate) fn decode(body: &[u8]) -> Result<Self, BodyFault> {
        let fields = Fields::parse(body, &[1, 2, 3, 4, 5, 6])?;
        Ok(Self {
            server_pid: fields.uint32(1)?,
            open_sessions: fields.uint32(2)?,
            max_sessions: fields.uint32(3)?,
            idle_exit: fields.opt_uint32(4)?,
            log: fields.value(5, as_text)?.map(str::to_owned),
            stopping: fields.need(6, as_bool)?,
        })
    }
}

/// A `SessionOpen` of the client placement: the one shape `wire::validate_session_open` admits. It has no field for an
/// environment, `umask`, `openssl` or `transport_off`, so none can be written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenRequest {
    pub(crate) protocol: u32,
    pub(crate) client: String,
    /// The `SessionLimits` map (key 4), already encoded, when the client states limits.
    pub(crate) limits: Option<Vec<u8>>,
}

impl OpenRequest {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut entries = vec![
            (1, Val::Uint(self.protocol.into())),
            (2, Val::Text(&self.client)),
            // `attributes`: an empty `ProcessAttributes`; `host_environment`: true.
            (5, Val::Raw(&[0xa0])),
            (6, Val::Bool(true)),
        ];
        if let Some(limits) = &self.limits {
            entries.push((4, Val::Raw(limits)));
        }
        encode(entries)
    }

    /// Reads a body that `validate_session_open` has passed, under the canonical rule.
    pub(crate) fn decode(body: &[u8]) -> Result<Self, BodyFault> {
        let fields = Fields::parse(body, &[1, 2, 3, 4, 5, 6])?;
        Ok(Self {
            protocol: fields.uint32(1)?,
            client: fields.text(2)?,
            limits: fields.raw(4).map(<[u8]>::to_vec),
        })
    }
}

/// The keys of `SessionLimits`, 1 to 10.
pub(crate) const SESSION_LIMIT_KEYS: usize = 10;

/// `SessionLimits`, the `SessionOpen`'s key 4 (the taut message of that name in `protocol/gwz.taut.py`; operator decision
/// Q1 (a) of plan step 2.3g): the core session contract's §1 limits in its table order, `running_operations` (1),
/// `queued_operations` (2), `operation_table` (3), `direct_workers` (4), `event_log` (5), `open_logs` (6),
/// `outstanding_calls` (7), `control_reserve` (8), `read_bytes` (9) and `close_wait_ms` (10). Every key is present and
/// every value an unsigned integer: a missing key, a key outside 1 to 10, a negative, null or other value, and a body
/// that breaks the canonical rule are refused. The values come back by key, the value of key `k` at index `k - 1`; what
/// they may be is the host's to judge.
pub(crate) fn decode_session_limits(body: &[u8]) -> Result<[u64; SESSION_LIMIT_KEYS], BodyFault> {
    const KEYS: [u64; SESSION_LIMIT_KEYS] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let fields = Fields::parse(body, &KEYS)?;
    let mut values = [0; SESSION_LIMIT_KEYS];
    for (value, key) in values.iter_mut().zip(KEYS) {
        *value = fields.need(key, as_uint)?;
    }
    Ok(values)
}

/// The code and message of a `SessionError` for call ID 0, the refusal a host writes before `SessionOpened`.
pub(crate) fn decode_refusal(body: &[u8]) -> Result<(u64, String), BodyFault> {
    let fields = Fields::parse(body, &[1, 2])?;
    if fields.need(1, as_uint)? != 0 {
        return Err(BodyFault::Range(1));
    }
    let error = Fields::parse(fields.raw(2).ok_or(BodyFault::Missing(2))?, &[1, 2])?;
    Ok((error.need(1, as_uint)?, error.text(2)?))
}
