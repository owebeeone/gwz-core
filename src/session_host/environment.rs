//! The endpoint environment snapshot (session plan CS1.5), for the core
//! session contract (gwz-dev `dev-docs/GwzCoreSessionDesign.md`) §5.6 and O9,
//! as the server design amends §5.6.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Core never reads the environment: a driver reads it at its edge and passes the pairs in, to `from_os_pairs` from Rust or to `from_byte_pairs` as bytes | §5.6 "captured by the driver and passed in", "Core and the extension never read the environment themselves"; O9 |
//! | Byte-string pairs on every platform: raw bytes on POSIX, WTF-8 on Windows, so non-UTF-8 bytes and unpaired surrogates survive | §5.6 "captured losslessly" |
//! | Kept as captured, one value per name, the first occurrence winning, as a lookup of the live environment finds it | §5.6 "kept as captured" |
//! | Names follow the platform's rules: byte for byte on POSIX, and on Windows ordinally ignoring case, as the OS and std's `Command` compare them | plan C7 |
//! | Secret-bearing: a value has no `Debug` or `Display`, the snapshot formats as its entry count, and nothing serializes it | §5.6 "never serialized", §15.8 |
//! | A refused entry's error names its index, never its name or value | §15.8 |
//! | Dropped with the session, which drops the session context; it overwrites its own buffers first, not the copies std's `Command` and the OS make for a child | §5.6 "dropped when the session ends"; the server design's §5.6 amendment "zeroized when the session ends" |
//! | A child gets `env_clear()` plus the snapshot and nothing else from the live environment | §5.6 "Child processes", O9 |

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::Command;
use std::sync::atomic::{Ordering, compiler_fence};

use crate::model::{ErrorCode, ModelError, ModelResult};

/// The endpoint environment of one session (§5.6): the process environment a
/// driver read at its edge, as name and value pairs.
///
/// It is secret-bearing. Its `Debug` output is its entry count, it has no
/// `Display`, and nothing serializes it. It moves into the session with
/// `open`'s options and drops with the session context, overwriting its own
/// buffers as it drops. The copies that std's `Command` and the OS make for a
/// child are outside it.
pub struct EnvironmentSnapshot {
    entries: Vec<(Wiped, Wiped)>,
}

/// A name or value of the snapshot, overwritten when dropped.
struct Wiped(OsString);

impl Drop for Wiped {
    fn drop(&mut self) {
        wipe(std::mem::take(&mut self.0).into_encoded_bytes());
    }
}

/// One value of the snapshot. It has no `Debug`, `Display` or serialization,
/// so formatting a value takes an explicit `as_os_str` (§15.8).
#[allow(dead_code, reason = "CS3.2 derives the endpoint configuration from it")]
pub(crate) struct EnvValue<'a>(&'a OsStr);

#[allow(dead_code, reason = "CS3.2 derives the endpoint configuration from it")]
impl<'a> EnvValue<'a> {
    pub(crate) fn as_os_str(&self) -> &'a OsStr {
        self.0
    }
}

impl EnvironmentSnapshot {
    /// A snapshot a driver captured itself, as byte-string pairs: raw bytes
    /// on POSIX, such as Python's `os.environb`, and WTF-8 on Windows, such as
    /// `os.environ` encoded so that unpaired surrogates survive (§5.6).
    ///
    /// An entry no environment can hold is refused with `invalid_request`: a
    /// name that is empty, holds a NUL or holds `=` after its first character,
    /// a value that holds a NUL, or, on Windows, bytes that are not WTF-8.
    /// The error names the entry's index, never its name or value.
    pub fn from_byte_pairs<I>(pairs: I) -> ModelResult<Self>
    where
        I: IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    {
        Self::collect(pairs.into_iter().map(|(name, value)| entry(name, value)))
    }

    /// A snapshot of the pairs a Rust driver read at its edge, in its own
    /// crate: its process environment as std's `vars_os` yields it, passed
    /// straight in (`docs/RustApi.md` shows the call). Core itself never
    /// reads the environment (§5.6).
    ///
    /// It refuses what `from_byte_pairs` refuses, with the same error naming
    /// only the entry's index: a name that is empty, holds a NUL or holds `=`
    /// after its first character, and a value that holds a NUL. The platform
    /// yields no such entry, but an arbitrary caller's pairs can hold one.
    pub fn from_os_pairs<I>(pairs: I) -> ModelResult<Self>
    where
        I: IntoIterator<Item = (OsString, OsString)>,
    {
        Self::collect(
            pairs
                .into_iter()
                .map(|(name, value)| checked(Wiped(name), Wiped(value))),
        )
    }

    /// Builds a snapshot from checked entries, in order. At the first refusal
    /// it drops, and so overwrites, everything else it was given.
    fn collect<I>(mut entries: I) -> ModelResult<Self>
    where
        I: Iterator<Item = Result<(Wiped, Wiped), &'static str>>,
    {
        let mut snapshot = Self {
            entries: Vec::new(),
        };
        for (index, entry) in entries.by_ref().enumerate() {
            match entry {
                Ok((name, value)) => snapshot.insert(name, value),
                Err(reason) => {
                    entries.for_each(drop);
                    return Err(ModelError::new(
                        ErrorCode::InvalidRequest,
                        format!("environment snapshot entry {index} {reason}"),
                    ));
                }
            }
        }
        Ok(snapshot)
    }

    /// Keeps the first value of each name, under the platform's name rules.
    fn insert(&mut self, name: Wiped, value: Wiped) {
        if self.find(&name.0).is_none() {
            self.entries.push((name, value));
        }
    }

    fn find(&self, name: &OsStr) -> Option<&(Wiped, Wiped)> {
        self.entries
            .iter()
            .find(|(entry, _)| platform::same_name(&entry.0, name))
    }

    #[allow(dead_code, reason = "CS3.2 derives the endpoint configuration from it")]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The value of `name`, under the platform's name rules (C7).
    #[allow(dead_code, reason = "CS3.2 derives the endpoint configuration from it")]
    pub(crate) fn get(&self, name: impl AsRef<OsStr>) -> Option<EnvValue<'_>> {
        self.find(name.as_ref())
            .map(|(_, value)| EnvValue(&value.0))
    }

    /// Gives a child process exactly the snapshot: `env_clear()`, then every
    /// entry (§5.6). A spawn may then remove prompt hooks and add
    /// prompt-disabling settings, as §5.8 allows. `Command` keeps its own copy
    /// of each entry until it drops.
    #[allow(
        dead_code,
        reason = "CS3.3 and CS3.4 spawn session-path children with it"
    )]
    pub(crate) fn apply_to<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        command.env_clear();
        for (name, value) in &self.entries {
            command.env(&name.0, &value.0);
        }
        command
    }
}

impl fmt::Debug for EnvironmentSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentSnapshot")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// Decodes one driver-supplied byte pair, then checks it. What it drops on
/// the way, it overwrites.
fn entry(name: Vec<u8>, value: Vec<u8>) -> Result<(Wiped, Wiped), &'static str> {
    let name = platform::from_bytes(name).map(Wiped);
    let value = platform::from_bytes(value).map(Wiped);
    let (Some(name), Some(value)) = (name, value) else {
        return Err("is not WTF-8");
    };
    checked(name, value)
}

/// Checks that an environment can hold the pair: a name that is not empty
/// and holds neither a NUL nor a `=` after its first character, and a value
/// without a NUL. Both encodings keep ASCII bytes as themselves, so the checks
/// read the encoded bytes on every platform. A refused pair drops, and so is
/// overwritten.
fn checked(name: Wiped, value: Wiped) -> Result<(Wiped, Wiped), &'static str> {
    let (bytes, value_bytes) = (name.0.as_encoded_bytes(), value.0.as_encoded_bytes());
    if bytes.is_empty() {
        return Err("has an empty name");
    }
    if bytes.contains(&0) || value_bytes.contains(&0) {
        return Err("holds a NUL");
    }
    if bytes[1..].contains(&b'=') {
        return Err("has '=' in its name");
    }
    Ok((name, value))
}

/// Overwrites a buffer's whole allocation, spare capacity included, before
/// it is freed.
fn wipe<T: Copy + Default>(mut buffer: Vec<T>) {
    let start = buffer.as_mut_ptr();
    for offset in 0..buffer.capacity() {
        // SAFETY: `start` addresses `buffer`'s allocation of `capacity`
        // elements, which only `buffer` owns. `T` is `Copy`, so it has no drop
        // to skip, and a write needs no initialized element.
        unsafe { std::ptr::write_volatile(start.add(offset), T::default()) };
    }
    compiler_fence(Ordering::SeqCst);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod platform {
            use std::ffi::{OsStr, OsString};
            use std::os::unix::ffi::OsStringExt;

            /// POSIX environment strings are bytes already.
            pub(super) fn from_bytes(bytes: Vec<u8>) -> Option<OsString> {
                Some(OsString::from_vec(bytes))
            }

            /// POSIX names compare byte for byte.
            pub(super) fn same_name(left: &OsStr, right: &OsStr) -> bool {
                left == right
            }
        }
    } else if #[cfg(windows)] {
        mod platform {
            use std::ffi::{OsStr, OsString};
            use std::os::windows::ffi::{OsStrExt, OsStringExt};

            use windows_sys::Win32::Foundation::TRUE;
            use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

            use super::{wide, wipe};

            /// Decodes WTF-8, overwriting the buffers it used.
            pub(super) fn from_bytes(bytes: Vec<u8>) -> Option<OsString> {
                let mut units = Vec::with_capacity(bytes.len());
                let decoded = wide::decode_wtf8(&bytes, &mut units);
                wipe(bytes);
                let text = decoded.then(|| OsString::from_wide(&units));
                wipe(units);
                text
            }

            /// Windows names compare as the OS and std's `Command` compare
            /// them: ordinally, ignoring case, with `CompareStringOrdinal`
            /// (C7). Like std's `EnvKey`, names of different lengths differ.
            pub(super) fn same_name(left: &OsStr, right: &OsStr) -> bool {
                let left: Vec<u16> = left.encode_wide().collect();
                let right: Vec<u16> = right.encode_wide().collect();
                if left.len() != right.len() {
                    return false;
                }
                let Ok(length) = i32::try_from(left.len()) else {
                    return false;
                };
                // SAFETY: both pointers address `length` initialized UTF-16
                // units that outlive the call; with explicit lengths, neither
                // needs a terminator.
                let order = unsafe {
                    CompareStringOrdinal(left.as_ptr(), length, right.as_ptr(), length, TRUE)
                };
                order == CSTR_EQUAL
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(any(windows, test))] {
        /// The Windows arm's WTF-8 decoding. Tests build it on every platform,
        /// so every run checks it.
        mod wide {
            /// Decodes WTF-8 into UTF-16 units, keeping unpaired surrogates.
            /// It accepts generalized UTF-8: a surrogate pair written as two
            /// three-byte surrogates, as Python's `surrogatepass` writes a split
            /// pair, decodes to the same units as the four-byte form. It refuses
            /// overlong forms, values above U+10FFFF and broken sequences.
            pub(super) fn decode_wtf8(bytes: &[u8], units: &mut Vec<u16>) -> bool {
                let mut index = 0;
                while let Some(&lead) = bytes.get(index) {
                    let (width, minimum, mut point) = match lead {
                        0x00..=0x7F => (1, 0, u32::from(lead)),
                        0xC2..=0xDF => (2, 0x80, u32::from(lead & 0x1F)),
                        0xE0..=0xEF => (3, 0x800, u32::from(lead & 0x0F)),
                        0xF0..=0xF4 => (4, 0x1_0000, u32::from(lead & 0x07)),
                        _ => return false,
                    };
                    let Some(tail) = bytes.get(index + 1..index + width) else {
                        return false;
                    };
                    for &byte in tail {
                        if byte & 0xC0 != 0x80 {
                            return false;
                        }
                        point = (point << 6) | u32::from(byte & 0x3F);
                    }
                    if point < minimum || point > 0x10_FFFF {
                        return false;
                    }
                    match u16::try_from(point) {
                        Ok(unit) => units.push(unit),
                        Err(_) => {
                            let offset = point - 0x1_0000;
                            units.extend([0xD800 | (offset >> 10) as u16, 0xDC00 | (offset & 0x3FF) as u16]);
                        }
                    }
                    index += width;
                }
                true
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
