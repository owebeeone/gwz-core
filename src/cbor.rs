//! Minimal deterministic CBOR — the **fail-closed** Rust binding of the frozen
//! wire substrate, and taut's only Rust runtime (vendored as `cbor.rs` by
//! `tautc gen -l rust --with-runtime`).
//!
//! Byte-for-byte identical ENCODE to `taut/src/taut/wire/cbor.py` and the
//! TypeScript runtime: the same tiny subset (int, bytes, text, array, int-keyed
//! map, bool, null, float) in core deterministic encoding. Hand-rolled, zero
//! dependencies.
//!
//! Decode is fail-closed (none of it changes the bytes any value encodes to):
//!   1. `Cbor::Int` carries `i64` (the frozen wire int subset, `[-2^63, 2^63-1]`),
//!      and a CBOR integer OUTSIDE that subset (a major-0 argument above
//!      `i64::MAX`, or a major-1 value below `i64::MIN`, i.e. anything in the
//!      wire-representable `[-2^64, 2^64-1]` beyond `i64`) is a typed
//!      [`DecodeError::IntOverflow`], never a silent `n as i64` wrap and never a
//!      wider (128-bit) carry. Map KEYS are `i64` too (CBOR field tags are small;
//!      keeps `ext.rs` / `wire_residual` source-compatible).
//!   2. A typed [`DecodeError`] plus fallible [`try_decode`] and `try_*`
//!      accessors: **decode never panics on any byte input** (malformed,
//!      truncated, unknown enum arm, wrong type, trailing bytes, out-of-subset
//!      integer). This is the substrate the generated
//!      `from_cbor -> Result<_, DecodeError>` builds on, so a caller behind an
//!      untrusted wire boundary (a socket) needs no `catch_unwind` guard around
//!      decode. The legacy runtime's panicking `decode` and accessors were
//!      removed at taut v0.10.0.
//!   3. Decode is bounded (TautCheckedDecode.md §3). An array or map has depth
//!      one more than the arrays and maps around it, and one deeper than the
//!      call's depth bound is [`DecodeError::TooDeep`] once its head is read;
//!      recursion goes no deeper than the bound, so no input can exhaust the
//!      stack. With a length bound, longer input is [`DecodeError::TooLarge`]
//!      before a byte is read. [`try_decode`] applies [`DEFAULT_MAX_DEPTH`];
//!      [`try_decode_max`] adds a length bound, and [`try_decode_with`] takes
//!      both, its depth capped at [`MAX_DEPTH_CEILING`]. A generated message's
//!      `decode` passes its root's bounds (TautOptions.md OPT-D4).

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

/// The depth bound where the caller gives none: 32 nested arrays and maps
/// decode, and the 33rd is [`DecodeError::TooDeep`] (CD-B1). taut's
/// `DEFAULT_MAX_DEPTH`, the `max_depth` option's default.
pub const DEFAULT_MAX_DEPTH: usize = 32;

/// The deepest bound any decode applies: [`try_decode_with`] applies it in
/// place of a larger `max_depth` (CD-B3). taut's `MAX_DEPTH_CEILING`.
pub const MAX_DEPTH_CEILING: usize = 128;

/// A repeated map key, as [`DecodeError::DuplicateMapKey`] reports it: the int
/// key of a raw CBOR map or of a `map<int,V>` field, or the key of a `map<str,V>`
/// or `map<bool,V>` field. Its text (`Display`) is the one every taut language
/// reports (TautCheckedDecode.md question 9): an int in decimal, a str as
/// itself, a bool as `true` or `false`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapKey {
    Int(i64),
    Text(String),
    Bool(bool),
}

impl core::fmt::Display for MapKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MapKey::Int(n) => write!(f, "{n}"),
            MapKey::Text(s) => f.write_str(s),
            MapKey::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl From<i64> for MapKey {
    fn from(key: i64) -> Self {
        MapKey::Int(key)
    }
}

impl From<String> for MapKey {
    fn from(key: String) -> Self {
        MapKey::Text(key)
    }
}

impl From<bool> for MapKey {
    fn from(key: bool) -> Self {
        MapKey::Bool(key)
    }
}

/// A typed decode failure. Every variant is reachable only from *input* bytes;
/// the fallible decode path returns these instead of panicking, so an untrusted
/// wire boundary is fail-closed by construction.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodeError {
    /// Ran off the end of the input (a truncated argument, string, or item).
    Truncated,
    /// Trailing bytes after the top-level item (a decode consumed fewer bytes
    /// than were supplied).
    TrailingBytes,
    /// A text string's bytes were not valid UTF-8.
    InvalidUtf8,
    /// An additional-info / simple value outside the frozen subset.
    UnsupportedInfo(u8),
    /// A major type outside the frozen subset (major 6 = tags).
    UnsupportedMajor(u8),
    /// A map key that was not a (frozen-subset) integer.
    NonIntegerMapKey,
    /// The same key appeared twice in one CBOR map, or in two entries of a
    /// generated `map<K,V>` field: the key itself (see [`MapKey`]).
    DuplicateMapKey(MapKey),
    /// A CBOR integer on the wire outside the frozen `i64` subset — a major-0
    /// value above `i64::MAX`, a major-1 value below `i64::MIN`, or a map key
    /// wider than `i64`. Rejected here rather than silently wrapped or widened.
    IntOverflow,
    /// A multi-byte integer argument that would fit a shorter form (non-minimal).
    /// The canonical encoder never emits it, so strict-canonical decode (D2)
    /// rejects it — `decode(bytes)` ok ⇒ `encode(decode(bytes)) == bytes`.
    NonCanonicalInt(u64),
    /// A raw CBOR map key that was a negative integer. Canonical taut field tags
    /// are non-negative, so a negative raw key is out-of-contract (D2). Distinct
    /// from [`DecodeError::NonIntegerMapKey`] (a non-integer key).
    NegativeMapKey(i64),
    /// A required map key was absent (missing field).
    MissingKey(i64),
    /// A value had the wrong CBOR type for the field being decoded.
    WrongType {
        /// What the decoder expected ("int", "text", "map", …).
        expected: &'static str,
    },
    /// A wire value with no member in the named generated enum.
    UnknownEnum {
        /// The generated enum's Rust name.
        enum_name: &'static str,
        /// The offending wire value.
        value: i64,
    },
    /// An array or map nested deeper than the call's depth bound, refused once
    /// its head is read and before its first item (CD-B1, CD-B2).
    TooDeep {
        /// The depth bound the call applied.
        limit: usize,
    },
    /// Input longer than the call's length bound, refused before any byte of it
    /// is read (CD-B4).
    TooLarge {
        /// The input's length in bytes.
        len: usize,
        /// The length bound the call applied.
        limit: usize,
    },
}

impl DecodeError {
    /// The failure's canonical tag, the variant's name: the tag every taut
    /// language reports for it, which the parity gate compares along with the
    /// payload (TautCheckedDecode.md CD-E2).
    pub fn tag(&self) -> &'static str {
        match self {
            DecodeError::Truncated => "Truncated",
            DecodeError::TrailingBytes => "TrailingBytes",
            DecodeError::InvalidUtf8 => "InvalidUtf8",
            DecodeError::UnsupportedInfo(_) => "UnsupportedInfo",
            DecodeError::UnsupportedMajor(_) => "UnsupportedMajor",
            DecodeError::NonIntegerMapKey => "NonIntegerMapKey",
            DecodeError::DuplicateMapKey(_) => "DuplicateMapKey",
            DecodeError::IntOverflow => "IntOverflow",
            DecodeError::NonCanonicalInt(_) => "NonCanonicalInt",
            DecodeError::NegativeMapKey(_) => "NegativeMapKey",
            DecodeError::MissingKey(_) => "MissingKey",
            DecodeError::WrongType { .. } => "WrongType",
            DecodeError::UnknownEnum { .. } => "UnknownEnum",
            DecodeError::TooDeep { .. } => "TooDeep",
            DecodeError::TooLarge { .. } => "TooLarge",
        }
    }
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "truncated CBOR input"),
            DecodeError::TrailingBytes => write!(f, "trailing bytes after top-level CBOR item"),
            DecodeError::InvalidUtf8 => write!(f, "invalid UTF-8 in CBOR text string"),
            DecodeError::UnsupportedInfo(i) => write!(f, "unsupported additional-info {i}"),
            DecodeError::UnsupportedMajor(m) => write!(f, "unsupported major type {m}"),
            DecodeError::NonIntegerMapKey => write!(f, "non-integer map key"),
            DecodeError::DuplicateMapKey(k) => write!(f, "duplicate map key {k}"),
            DecodeError::IntOverflow => write!(f, "integer out of range for target"),
            DecodeError::NonCanonicalInt(v) => write!(f, "non-canonical integer encoding of {v}"),
            DecodeError::NegativeMapKey(k) => write!(f, "negative map key {k}"),
            DecodeError::MissingKey(k) => write!(f, "missing map key {k}"),
            DecodeError::WrongType { expected } => write!(f, "expected CBOR {expected}"),
            DecodeError::UnknownEnum { enum_name, value } => {
                write!(f, "unknown {enum_name} wire value {value}")
            }
            DecodeError::TooDeep { limit } => write!(f, "CBOR nested deeper than {limit}"),
            DecodeError::TooLarge { len, limit } => {
                write!(f, "CBOR input of {len} bytes is longer than {limit}")
            }
        }
    }
}

/// Exact `2.0f64.powi(exp)` for an integer exponent, `core`-only (no libm).
fn pow2(exp: i32) -> f64 {
    if (-1022..=1023).contains(&exp) {
        let biased = (exp + 1023) as u64;
        f64::from_bits(biased << 52)
    } else if exp < -1022 {
        let mut v = f64::from_bits(1u64 << 52); // 2^-1022
        let mut e = -1022;
        while e > exp {
            v *= 0.5;
            e -= 1;
        }
        v
    } else {
        f64::INFINITY
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cbor {
    Int(i64),
    Float(f64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(i64, Cbor)>),
    Bool(bool),
    Null,
}

impl Cbor {
    pub fn is_map(&self) -> bool {
        matches!(self, Cbor::Map(_))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Cbor::Null)
    }
    /// All (key, value) pairs of a map (empty if not a map). Used to capture
    /// forward-compat residual: tags the schema doesn't name.
    pub fn map_entries(&self) -> &[(i64, Cbor)] {
        if let Cbor::Map(m) = self {
            m
        } else {
            &[]
        }
    }

    // --- fallible accessors (the fail-closed decode surface) ----------------

    /// Value for an integer map key, or [`DecodeError::MissingKey`] if absent /
    /// [`DecodeError::WrongType`] if the receiver is not a map.
    pub fn try_get(&self, key: i64) -> Result<&Cbor, DecodeError> {
        if let Cbor::Map(m) = self {
            for (k, v) in m {
                if *k == key {
                    return Ok(v);
                }
            }
            Err(DecodeError::MissingKey(key))
        } else {
            Err(DecodeError::WrongType { expected: "map" })
        }
    }

    /// Value for an integer map key, accepting an absent key while still
    /// rejecting a non-map input.
    pub fn try_get_opt(&self, key: i64) -> Result<Option<&Cbor>, DecodeError> {
        if let Cbor::Map(m) = self {
            for (k, v) in m {
                if *k == key {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        } else {
            Err(DecodeError::WrongType { expected: "map" })
        }
    }
    /// Integer value. The carrier is `i64` (the frozen wire int subset); an
    /// out-of-subset wire int was already rejected by `dec`, so this never
    /// truncates or widens.
    pub fn try_int(&self) -> Result<i64, DecodeError> {
        if let Cbor::Int(n) = self {
            Ok(*n)
        } else {
            Err(DecodeError::WrongType { expected: "int" })
        }
    }
    pub fn try_float(&self) -> Result<f64, DecodeError> {
        if let Cbor::Float(x) = self {
            Ok(*x)
        } else {
            Err(DecodeError::WrongType { expected: "float" })
        }
    }
    pub fn try_text(&self) -> Result<String, DecodeError> {
        if let Cbor::Text(s) = self {
            Ok(s.clone())
        } else {
            Err(DecodeError::WrongType { expected: "text" })
        }
    }
    pub fn try_bytes(&self) -> Result<Vec<u8>, DecodeError> {
        if let Cbor::Bytes(b) = self {
            Ok(b.clone())
        } else {
            Err(DecodeError::WrongType { expected: "bytes" })
        }
    }
    pub fn try_bool(&self) -> Result<bool, DecodeError> {
        if let Cbor::Bool(b) = self {
            Ok(*b)
        } else {
            Err(DecodeError::WrongType { expected: "bool" })
        }
    }
    pub fn try_array(&self) -> Result<&[Cbor], DecodeError> {
        if let Cbor::Array(a) = self {
            Ok(a)
        } else {
            Err(DecodeError::WrongType { expected: "array" })
        }
    }
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let mt = major << 5;
    if n < 24 {
        out.push(mt | n as u8);
    } else if n < 0x100 {
        out.push(mt | 24);
        out.push(n as u8);
    } else if n < 0x1_0000 {
        out.push(mt | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n < 0x1_0000_0000 {
        out.push(mt | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn round_shift_right(value: u128, shift: u32) -> u128 {
    if shift == 0 {
        return value;
    }
    if shift >= 128 {
        return 0;
    }
    let quotient = value >> shift;
    let remainder = value & ((1u128 << shift) - 1);
    let halfway = 1u128 << (shift - 1);
    if remainder > halfway || (remainder == halfway && (quotient & 1) == 1) {
        quotient + 1
    } else {
        quotient
    }
}

fn f64_to_f16_bits(value: f64) -> Option<u16> {
    let bits = value.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & 0x000f_ffff_ffff_ffff;

    if exp == 0x7ff {
        return Some(if frac == 0 { sign | 0x7c00 } else { 0x7e00 });
    }
    if exp == 0 {
        return Some(sign);
    }

    let e = exp - 1023;
    let mant = (1u128 << 52) | frac as u128;
    if e < -14 {
        let sub = round_shift_right(mant, (28 - e) as u32);
        if sub == 0 {
            return Some(sign);
        }
        if sub >= 0x400 {
            return Some(sign | 0x0400);
        }
        return Some(sign | sub as u16);
    }
    if e > 15 {
        return None;
    }

    let mut half_exp = e + 15;
    let mut sig = round_shift_right(mant, 42);
    if sig == 0x800 {
        half_exp += 1;
        sig = 0x400;
        if half_exp >= 31 {
            return None;
        }
    }
    Some(sign | ((half_exp as u16) << 10) | (sig as u16 - 0x400))
}

fn f16_bits_to_f64(bits: u16) -> f64 {
    let sign = ((bits as u64 & 0x8000) << 48) != 0;
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x03ff;
    match exp {
        0 => {
            if frac == 0 {
                f64::from_bits(if sign { 1u64 << 63 } else { 0 })
            } else {
                let v = (frac as f64) * pow2(-24);
                if sign {
                    -v
                } else {
                    v
                }
            }
        }
        0x1f => {
            if frac == 0 {
                f64::from_bits((if sign { 1u64 << 63 } else { 0 }) | 0x7ff0_0000_0000_0000)
            } else {
                f64::NAN
            }
        }
        _ => {
            let v = (1.0 + (frac as f64) / 1024.0) * pow2(exp as i32 - 15);
            if sign {
                -v
            } else {
                v
            }
        }
    }
}

fn enc_float(value: f64, out: &mut Vec<u8>) {
    if value.is_nan() {
        out.extend_from_slice(&[0xf9, 0x7e, 0x00]);
        return;
    }
    if let Some(bits) = f64_to_f16_bits(value) {
        if f16_bits_to_f64(bits).to_bits() == value.to_bits() {
            out.push(0xf9);
            out.extend_from_slice(&bits.to_be_bytes());
            return;
        }
    }
    let single = value as f32;
    if (single as f64).to_bits() == value.to_bits() {
        out.push(0xfa);
        out.extend_from_slice(&single.to_bits().to_be_bytes());
    } else {
        out.push(0xfb);
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }
}

pub fn encode(v: &Cbor) -> Vec<u8> {
    let mut out = Vec::new();
    enc(v, &mut out);
    out
}

fn enc(v: &Cbor, out: &mut Vec<u8>) {
    match v {
        // The `i64` carrier IS the encode-side subset guarantee: every `i64`
        // is in the frozen wire subset, so there is no out-of-subset value to
        // reject and both casts are total — a non-negative `n` fits `u64`, and
        // `-1 - *n` for `*n` in `[i64::MIN, -1]` lands in `[0, i64::MAX]`.
        // Neither can wrap (unlike the reverted i128 carrier, where `*n as u64`
        // could wrap for `|n| > 2^64`). Byte-identical to the Python runtime.
        Cbor::Int(n) => {
            if *n >= 0 {
                head(out, 0, *n as u64);
            } else {
                head(out, 1, (-1 - *n) as u64);
            }
        }
        Cbor::Float(x) => enc_float(*x, out),
        Cbor::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend_from_slice(b);
        }
        Cbor::Text(s) => {
            let b = s.as_bytes();
            head(out, 3, b.len() as u64);
            out.extend_from_slice(b);
        }
        Cbor::Array(a) => {
            head(out, 4, a.len() as u64);
            for x in a {
                enc(x, out);
            }
        }
        Cbor::Map(m) => {
            let mut entries: Vec<&(i64, Cbor)> = m.iter().collect();
            entries.sort_by_key(|(k, _)| *k); // deterministic: ascending keys
            head(out, 5, m.len() as u64);
            for (k, val) in entries {
                head(out, 0, *k as u64);
                enc(val, out);
            }
        }
        Cbor::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Cbor::Null => out.push(0xf6),
    }
}

/// Fail-closed decode at the default depth bound, [`DEFAULT_MAX_DEPTH`], with no
/// length bound: returns [`DecodeError`] — never panics — on any byte input
/// (malformed, truncated, too deep, unknown value, wrong shape, trailing bytes).
pub fn try_decode(data: &[u8]) -> Result<Cbor, DecodeError> {
    try_decode_with(data, DEFAULT_MAX_DEPTH, None)
}

/// [`try_decode`] with a length bound: input longer than `max_encoded_len` is
/// [`DecodeError::TooLarge`] before any byte is read (CD-B4).
pub fn try_decode_max(data: &[u8], max_encoded_len: usize) -> Result<Cbor, DecodeError> {
    try_decode_with(data, DEFAULT_MAX_DEPTH, Some(max_encoded_len))
}

/// Fail-closed decode under the caller's bounds (CD-B3). An array or map at
/// depth `max_depth` + 1 is [`DecodeError::TooDeep`] (a top-level one has depth
/// 1); a `max_depth` above [`MAX_DEPTH_CEILING`] applies the ceiling, and
/// `TooDeep`'s `limit` names the bound applied. With `max_encoded_len`, longer
/// input is [`DecodeError::TooLarge`], checked first (CD-E5).
///
/// # Panics
///
/// If `max_depth` is 0, the caller's error, not the input's: it panics before
/// it reads the input.
pub fn try_decode_with(
    data: &[u8],
    max_depth: usize,
    max_encoded_len: Option<usize>,
) -> Result<Cbor, DecodeError> {
    assert!(max_depth >= 1, "max_depth must be at least 1, not {max_depth}");
    let limit = max_depth.min(MAX_DEPTH_CEILING);
    if let Some(bound) = max_encoded_len {
        if data.len() > bound {
            return Err(DecodeError::TooLarge { len: data.len(), limit: bound });
        }
    }
    let (v, off) = dec(data, 0, 0, limit)?;
    if off != data.len() {
        return Err(DecodeError::TrailingBytes);
    }
    Ok(v)
}

/// Read `len` bytes at `off`, or [`DecodeError::Truncated`] if the slice is short.
fn take(data: &[u8], off: usize, len: usize) -> Result<&[u8], DecodeError> {
    // Guard the add against overflow before indexing (untrusted lengths).
    let end = off.checked_add(len).ok_or(DecodeError::Truncated)?;
    data.get(off..end).ok_or(DecodeError::Truncated)
}

fn read_arg(data: &[u8], off: usize, info: u8) -> Result<(u64, usize), DecodeError> {
    let (value, next) = match info {
        n if n < 24 => return Ok((n as u64, off)),
        24 => {
            let b = take(data, off, 1)?;
            (b[0] as u64, off + 1)
        }
        25 => {
            let b = take(data, off, 2)?;
            (u16::from_be_bytes([b[0], b[1]]) as u64, off + 2)
        }
        26 => {
            let b = take(data, off, 4)?;
            (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64, off + 4)
        }
        27 => {
            let b = take(data, off, 8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            (u64::from_be_bytes(a), off + 8)
        }
        _ => return Err(DecodeError::UnsupportedInfo(info)),
    };
    // Strict-canonical (D2): a multi-byte argument whose value fits a shorter
    // width is non-minimal — the canonical encoder never emits it, so reject it.
    let fits_shorter = match info {
        24 => value < 24,
        25 => value <= 0xFF,
        26 => value <= 0xFFFF,
        27 => value <= 0xFFFF_FFFF,
        _ => false,
    };
    if fits_shorter {
        return Err(DecodeError::NonCanonicalInt(value));
    }
    Ok((value, next))
}

/// A container whose head is read, inside `depth` others: [`DecodeError::TooDeep`]
/// if it would sit deeper than `limit`, before its first item is read (CD-B2).
/// Checked before `dec` recurses, it bounds the recursion by `limit`.
fn enter(depth: usize, limit: usize) -> Result<(), DecodeError> {
    if depth >= limit {
        return Err(DecodeError::TooDeep { limit });
    }
    Ok(())
}

/// The item at `off`, inside `depth` arrays and maps, under depth bound `limit`.
fn dec(data: &[u8], off: usize, depth: usize, limit: usize) -> Result<(Cbor, usize), DecodeError> {
    let initial = *data.get(off).ok_or(DecodeError::Truncated)?;
    let major = initial >> 5;
    let info = initial & 0x1f;
    let off = off + 1;
    match major {
        0 => {
            let (n, o) = read_arg(data, off, info)?;
            // Frozen wire int subset is i64: a major-0 argument above i64::MAX
            // is out-of-subset — a typed error, never a silent wrap or a wider
            // (128-bit) carry.
            let n = i64::try_from(n).map_err(|_| DecodeError::IntOverflow)?;
            Ok((Cbor::Int(n), o))
        }
        1 => {
            let (n, o) = read_arg(data, off, info)?;
            // major-1 encodes -(1 + n); in-subset iff n <= i64::MAX (so the
            // decoded value is >= i64::MIN). Out-of-subset -> IntOverflow, and
            // `-1 - n` for n in [0, i64::MAX] lands in [i64::MIN, -1] (no wrap).
            let n = i64::try_from(n).map_err(|_| DecodeError::IntOverflow)?;
            Ok((Cbor::Int(-1 - n), o))
        }
        2 => {
            let (n, o) = read_arg(data, off, info)?;
            // A length beyond `usize` (2^32 or more on a 32-bit target) is beyond
            // the input too: `Truncated`, never cut to its low bits.
            let n = usize::try_from(n).map_err(|_| DecodeError::Truncated)?;
            let b = take(data, o, n)?;
            Ok((Cbor::Bytes(b.to_vec()), o + n))
        }
        3 => {
            let (n, o) = read_arg(data, off, info)?;
            let n = usize::try_from(n).map_err(|_| DecodeError::Truncated)?;
            let b = take(data, o, n)?;
            let s = core::str::from_utf8(b).map_err(|_| DecodeError::InvalidUtf8)?;
            Ok((Cbor::Text(String::from(s)), o + n))
        }
        4 => {
            let (n, mut o) = read_arg(data, off, info)?;
            enter(depth, limit)?;
            let mut a = Vec::new();
            for _ in 0..n {
                let (v, o2) = dec(data, o, depth + 1, limit)?;
                a.push(v);
                o = o2;
            }
            Ok((Cbor::Array(a), o))
        }
        5 => {
            let (n, mut o) = read_arg(data, off, info)?;
            enter(depth, limit)?;
            let mut m = Vec::new();
            // The keys read so far: a repeated key costs a lookup, not a scan.
            let mut seen = BTreeSet::new();
            for _ in 0..n {
                // An entry's key is read and checked before its value is read
                // (CD-E5), so a bad key is reported even when the value is
                // missing or malformed.
                let (k, o2) = dec(data, o, depth + 1, limit)?;
                let ki = match k {
                    // Map keys are i64 (CBOR field tags). An out-of-i64 key was
                    // already rejected as IntOverflow when `dec` read it above,
                    // so here it is simply the decoded value.
                    Cbor::Int(i) if i < 0 => return Err(DecodeError::NegativeMapKey(i)),
                    Cbor::Int(i) => i,
                    _ => return Err(DecodeError::NonIntegerMapKey),
                };
                if !seen.insert(ki) {
                    return Err(DecodeError::DuplicateMapKey(MapKey::Int(ki)));
                }
                let (v, o3) = dec(data, o2, depth + 1, limit)?;
                m.push((ki, v));
                o = o3;
            }
            Ok((Cbor::Map(m), o))
        }
        7 => match info {
            20 => Ok((Cbor::Bool(false), off)),
            21 => Ok((Cbor::Bool(true), off)),
            22 => Ok((Cbor::Null, off)),
            25 => {
                let b = take(data, off, 2)?;
                let bits = u16::from_be_bytes([b[0], b[1]]);
                Ok((Cbor::Float(f16_bits_to_f64(bits)), off + 2))
            }
            26 => {
                let b = take(data, off, 4)?;
                let bits = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                Ok((Cbor::Float(f32::from_bits(bits) as f64), off + 4))
            }
            27 => {
                let b = take(data, off, 8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                Ok((Cbor::Float(f64::from_bits(u64::from_be_bytes(a))), off + 8))
            }
            _ => Err(DecodeError::UnsupportedInfo(info)),
        },
        _ => Err(DecodeError::UnsupportedMajor(major)),
    }
}
