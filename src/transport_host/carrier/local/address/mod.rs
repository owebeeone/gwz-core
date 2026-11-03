//! A server address (plan step 2.3e; parked server design §3 "Addresses" and "The walk"; `GwzSplitCommandChannel.md`
//! §4.1): one type, [`ServerAddress`], that every connector, binder and launcher takes, so that no code path opens an
//! unparsed string.
//!
//! **Parsing** ([`grammar`]) happens before any file-system, pipe or network call takes the address. A refusal is
//! [`AddressRefused`], code `server_address_refused` (83), naming the source and the reason and showing the address with
//! control characters escaped.
//!
//! **The walk** (Linux and macOS, `walk_unix`) follows a parsed socket path from `/` one component at a time, reading
//! links rather than following them (8 in all, a ninth refuses), and refuses an automount trigger, an automounter's
//! mount or a network file system before it looks up the next component. It holds what it judged, and a connect or bind
//! goes only through it (`Walked::connect`, `Walked::bind`, which borrow what the walk holds, so no path outlives it);
//! macOS has `NoTrigger`, from a `NoTriggerGate`, for a process that is its own. **Windows** has no walk: its checks after the grammar are `GetFullPathNameW` (the name comes back unchanged) and
//! `QueryDosDeviceW(L"PIPE")` (the pipe device is `\Device\NamedPipe`), `walk_windows`. Neither is called from the
//! connect or bind path yet: plan step 2.3i plugs them in at the call site.
use super::super::handshake::escape;
use std::{ffi::OsStr, fmt};

mod grammar;

pub(crate) use grammar::{
    PIPE_DEVICE, PIPE_NAME_MAX, Platform, Reason, SshPart, WindowsFault, judge_full_path,
    judge_pipe_device, parse_for,
};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod walk_unix;
        pub(crate) use walk_unix::*;
    } else if #[cfg(windows)] {
        mod walk_windows;
        pub(crate) use walk_windows::{HOST, check_pipe};
    }
}

/// Error code `server_address_refused` of `GwzErrorCode` (`GwzErrorCode::ServerAddressRefused.wire()`).
pub(crate) const SERVER_ADDRESS_REFUSED: u64 = 83;
/// The most bytes of a refused address, or of a component, that a message shows.
pub(crate) const SHOWN_ADDRESS: usize = 512;

/// `text` escaped for a message: printable ASCII as it is, a backslash doubled, anything else `\xNN`.
pub(crate) fn escape_text(text: &str) -> String {
    escape(text.as_bytes(), SHOWN_ADDRESS)
}

/// Where an address came from. Each source accepts its own forms (split doc §4.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    /// `--server`: a socket path or pipe name, `stdio` and `ssh://`.
    Flag,
    /// `GWZ_SERVER`: a socket path or pipe name and `stdio`.
    Environment,
    /// `SocketCoreBridge(address)`: a socket path or pipe name.
    Bridge,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::Flag => "--server",
            Source::Environment => "GWZ_SERVER",
            Source::Bridge => "SocketCoreBridge",
        })
    }
}

/// The `ssh://` remote form (server design §16; `--server` only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SshDestination {
    pub(crate) user: Option<String>,
    /// A name of ASCII letters, digits, `.`, `-` and `_`, or an IPv6 literal without its brackets.
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    /// The remote working directory: absolute, UTF-8, taken as written.
    pub(crate) path: String,
}

/// A parsed server address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ServerAddress {
    /// Linux, macOS: an absolute socket path that passed the grammar (its bytes; no NUL).
    Socket(Vec<u8>),
    /// Windows: `\\.\pipe\` and one name.
    Pipe(String),
    /// The stdio local form.
    Stdio,
    /// The SSH remote form.
    Ssh(SshDestination),
}

impl ServerAddress {
    /// Parses `text` from `source` under this host's grammar.
    pub(crate) fn parse(source: Source, text: &OsStr) -> Result<Self, AddressRefused> {
        // On Unix these are the path's own bytes; on Windows WTF-8, whose non-ASCII bytes no accepted form holds.
        let bytes = text.as_encoded_bytes();
        parse_for(HOST, source, bytes).map_err(|reason| AddressRefused {
            source,
            shown: escape(bytes, SHOWN_ADDRESS),
            reason,
        })
    }

    /// `GWZ_SERVER`'s value: unset or empty is no address; anything else is parsed and refused rather than skipped.
    pub(crate) fn from_environment(value: Option<&OsStr>) -> Result<Option<Self>, AddressRefused> {
        match value {
            None => Ok(None),
            Some(value) if value.is_empty() => Ok(None),
            Some(value) => Self::parse(Source::Environment, value).map(Some),
        }
    }
}

/// An address the grammar refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AddressRefused {
    pub(crate) source: Source,
    /// The address, escaped.
    pub(crate) shown: String,
    pub(crate) reason: Reason,
}

impl AddressRefused {
    pub(crate) fn code(&self) -> u64 {
        SERVER_ADDRESS_REFUSED
    }
}

impl fmt::Display for AddressRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the server address \"{}\" from {} is refused: {}",
            self.shown, self.source, self.reason
        )
    }
}

impl std::error::Error for AddressRefused {}
