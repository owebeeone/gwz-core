//! The address grammar (parked server design §3 "Addresses, parsed before any open"; `GwzSplitCommandChannel.md`
//! §4.1). Pure: it takes the address's bytes, the source that gave it and the platform whose grammar applies, and
//! touches no file, pipe or network, so a refusal comes before anything takes the address. The platform is a value,
//! not a build switch, so that every platform's spellings are judged on every host.
//!
//! | Form | Sources | Grammar |
//! | --- | --- | --- |
//! | socket path (Linux, macOS) | all three | starts with `/`; no NUL; no empty, `.` or `..` component (so no trailing `/`); at most 107 bytes (Linux) or 103 (macOS); macOS: not under `/.vol/`, `/.nofollow/` or `/.resolve/`; Linux: not abstract (`@name`, a leading NUL) |
//! | pipe name (Windows) | all three | the exact-case prefix `\\.\pipe\` and one component of ASCII letters, digits, `.`, `-` and `_`, not ending in `.`; at most 256 characters in all |
//! | `stdio` | `--server`, `GWZ_SERVER` | exactly `stdio` |
//! | `ssh://[user@]host[:port]/path` | `--server` | server design §16 question 2 |
//!
//! Everything else is refused, `auto` included: there is no `auto` address in this release (split doc §4.1).
use super::{ServerAddress, Source, SshDestination};
use std::{fmt, net::Ipv6Addr};

/// Whose grammar applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Platform {
    Linux,
    Macos,
    Windows,
}

/// The longest pipe name, in characters (all ASCII, so bytes).
pub(crate) const PIPE_NAME_MAX: usize = 256;
/// The exact spelling a local pipe name starts with.
pub(crate) const PIPE_PREFIX: &[u8] = br"\\.\pipe\";
/// What `QueryDosDeviceW(L"PIPE")` must answer first: the named-pipe file system, not a device the caller's logon session
/// has redefined.
pub(crate) const PIPE_DEVICE: &str = r"\Device\NamedPipe";
/// The macOS prefixes the kernel rewrites during lookup (vfs_lookup.c), so a walk by components would reach another
/// object than the kernel's own lookup.
const REWRITTEN: [&[u8]; 3] = [b"/.vol/", b"/.nofollow/", b"/.resolve/"];

impl Platform {
    /// The longest socket path: `sun_path` less its NUL, as Rust's standard library enforces it. On Linux a walked
    /// bind goes by `/proc/self/fd/<n>/<name>` (design §3 "The walk and the connect"), which spends 15 bytes and the
    /// digits of `n` before the last component: in a short directory a last component of more than about 90 bytes
    /// passes this limit and is refused at bind (`Kind::TooLong`, which names both lengths). A connect holds the
    /// socket's descriptor, so its path is short, and a missing last name gives no connect path at all.
    pub(crate) fn socket_path_max(self) -> Option<usize> {
        match self {
            Platform::Linux => Some(107),
            Platform::Macos => Some(103),
            Platform::Windows => None,
        }
    }
}

/// What was wrong with the `ssh://` form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SshPart {
    User,
    Host,
    Port,
    Path,
}

impl fmt::Display for SshPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SshPart::User => "user name",
            SshPart::Host => "host",
            SshPart::Port => "port",
            SshPart::Path => "path",
        })
    }
}

/// Why an address is refused. Closed: a refusal names one of these and the address, escaped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reason {
    /// An empty `--server` (an empty `GWZ_SERVER` is the same as an unset one and is never parsed).
    Empty,
    /// `auto`: no default server address in this release.
    Auto,
    /// `stdio` from `SocketCoreBridge`, or `ssh://` from `GWZ_SERVER` or `SocketCoreBridge`.
    NotFromSource,
    /// A relative path, or a word other than `stdio`.
    Relative,
    /// A NUL byte.
    Nul,
    /// Linux: an abstract socket name (`@name` or a leading NUL).
    Abstract,
    /// An empty, `.` or `..` component, or a trailing `/`.
    Component,
    /// Longer than the platform's limit.
    TooLong { length: usize, limit: usize },
    /// macOS: under `/.vol/`, `/.nofollow/` or `/.resolve/`.
    Rewritten,
    /// Windows: a form that reaches the network redirector (MUP) or another machine.
    Remote,
    /// Windows: a drive in the device namespace (`\\.\Z:\`), which may be mapped to a network share.
    Drive,
    /// Windows: a device other than the pipe file system.
    Device,
    /// Windows: another spelling of a local pipe, or a name outside the one-component allowlist.
    PipeSpelling,
    /// Windows: a path, which is not a server address there.
    NotAPipe,
    /// The `ssh://` form's part named.
    Ssh(SshPart),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Empty => f.write_str("it is empty"),
            Reason::Auto => f.write_str(
                "there is no auto address in this release; name a socket path, a pipe name or stdio",
            ),
            Reason::NotFromSource => f.write_str("this form is not accepted from this source"),
            Reason::Relative => f.write_str("it is not an absolute path or a known word"),
            Reason::Nul => f.write_str("it holds a NUL byte"),
            Reason::Abstract => f.write_str("abstract socket names are not accepted"),
            Reason::Component => f.write_str("it has an empty, '.' or '..' component"),
            Reason::TooLong { length, limit } => {
                write!(f, "it is {length} bytes and at most {limit} are accepted")
            }
            Reason::Rewritten => f.write_str(
                "the kernel rewrites paths under /.vol/, /.nofollow/ and /.resolve/",
            ),
            Reason::Remote => {
                f.write_str("it reaches the network redirector or another machine")
            }
            Reason::Drive => f.write_str("a drive may be mapped to a network share"),
            Reason::Device => f.write_str("it names a device other than the pipe file system"),
            Reason::PipeSpelling => f.write_str(
                r"a pipe name is \\.\pipe\ and one name of letters, digits, '.', '-' and '_', not ending in '.'",
            ),
            Reason::NotAPipe => {
                f.write_str(r"on Windows a server address is a pipe name, \\.\pipe\<name>")
            }
            Reason::Ssh(part) => write!(f, "the ssh:// form's {part} is not accepted"),
        }
    }
}

/// Parses `text`, given by `source`, under `platform`'s grammar.
pub(crate) fn parse_for(
    platform: Platform,
    source: Source,
    text: &[u8],
) -> Result<ServerAddress, Reason> {
    if text.is_empty() {
        return Err(Reason::Empty);
    }
    if text == b"auto" {
        return Err(Reason::Auto);
    }
    if text == b"stdio" {
        if source == Source::Bridge {
            return Err(Reason::NotFromSource);
        }
        return Ok(ServerAddress::Stdio);
    }
    if let Some(rest) = text.strip_prefix(b"ssh://") {
        if source != Source::Flag {
            return Err(Reason::NotFromSource);
        }
        return ssh(rest).map(ServerAddress::Ssh);
    }
    match platform {
        Platform::Windows => pipe(text).map(ServerAddress::Pipe),
        Platform::Linux | Platform::Macos => {
            socket(platform, text).map(|()| ServerAddress::Socket(text.to_vec()))
        }
    }
}

fn socket(platform: Platform, text: &[u8]) -> Result<(), Reason> {
    if platform == Platform::Linux && matches!(text.first(), Some(b'@' | 0)) {
        return Err(Reason::Abstract);
    }
    if text.contains(&0) {
        return Err(Reason::Nul);
    }
    let Some(rest) = text.strip_prefix(b"/") else {
        return Err(Reason::Relative);
    };
    if rest
        .split(|&byte| byte == b'/')
        .any(|part| part.is_empty() || part == b"." || part == b"..")
    {
        return Err(Reason::Component);
    }
    if platform == Platform::Macos && REWRITTEN.iter().any(|prefix| text.starts_with(prefix)) {
        return Err(Reason::Rewritten);
    }
    let limit = platform.socket_path_max().unwrap_or(0);
    if text.len() > limit {
        return Err(Reason::TooLong {
            length: text.len(),
            limit,
        });
    }
    Ok(())
}

fn pipe_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

fn pipe(text: &[u8]) -> Result<String, Reason> {
    if let Some(name) = text.strip_prefix(PIPE_PREFIX) {
        let mut parts = name.split(|&byte| byte == b'\\' || byte == b'/');
        if parts.clone().count() > 1 {
            if parts.any(|part| part == b"..") {
                return Err(Reason::Remote);
            }
            return Err(Reason::PipeSpelling);
        }
        if name.is_empty() || !name.iter().all(|&byte| pipe_character(byte)) || name.ends_with(b".")
        {
            return Err(Reason::PipeSpelling);
        }
        if text.len() > PIPE_NAME_MAX {
            return Err(Reason::TooLong {
                length: text.len(),
                limit: PIPE_NAME_MAX,
            });
        }
        return Ok(String::from_utf8_lossy(text).into_owned());
    }
    // Windows turns every `/` into `\` before it looks a name up, so the classification does too.
    let folded: Vec<u8> = text
        .iter()
        .map(|&byte| {
            if byte == b'/' {
                b'\\'
            } else {
                byte.to_ascii_uppercase()
            }
        })
        .collect();
    let namespace = [&br"\\.\"[..], br"\\?\", br"\??\"]
        .into_iter()
        .find_map(|prefix| folded.strip_prefix(prefix));
    if let Some(rest) = namespace {
        let first = rest.split(|&byte| byte == b'\\').next().unwrap_or_default();
        return Err(match first {
            b"UNC" | b"GLOBALROOT" => Reason::Remote,
            b"PIPE" => Reason::PipeSpelling,
            [letter, b':'] if letter.is_ascii_alphabetic() => Reason::Drive,
            _ => Reason::Device,
        });
    }
    if folded.starts_with(br"\\") {
        return Err(Reason::Remote);
    }
    Err(Reason::NotAPipe)
}

fn word(text: &str, character: fn(char) -> bool) -> bool {
    !text.is_empty() && !text.starts_with('-') && text.chars().all(character)
}

fn ssh_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

/// `[user@]host[:port]/path` after `ssh://` (server design §16 question 2).
fn ssh(rest: &[u8]) -> Result<SshDestination, Reason> {
    let rest = std::str::from_utf8(rest).map_err(|_| Reason::Ssh(SshPart::Path))?;
    let Some(slash) = rest.find('/') else {
        return Err(Reason::Ssh(SshPart::Path));
    };
    let (authority, path) = rest.split_at(slash);
    if path.contains(['\0', '?', '#']) {
        return Err(Reason::Ssh(SshPart::Path));
    }
    let (user, host_port) = match authority.split_once('@') {
        Some((user, host_port)) => {
            if !word(user, ssh_character) {
                return Err(Reason::Ssh(SshPart::User));
            }
            (Some(user.to_owned()), host_port)
        }
        None => (None, authority),
    };
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (literal, after) = bracketed
            .split_once(']')
            .ok_or(Reason::Ssh(SshPart::Host))?;
        literal
            .parse::<Ipv6Addr>()
            .map_err(|_| Reason::Ssh(SshPart::Host))?;
        if !after.is_empty() && !after.starts_with(':') {
            return Err(Reason::Ssh(SshPart::Host));
        }
        (literal, after.strip_prefix(':'))
    } else {
        match host_port.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    if !host_port.starts_with('[') && !word(host, ssh_character) {
        return Err(Reason::Ssh(SshPart::Host));
    }
    let port = match port {
        None => None,
        Some(digits) => {
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(Reason::Ssh(SshPart::Port));
            }
            match digits.parse::<u16>() {
                Ok(port) if port != 0 => Some(port),
                _ => return Err(Reason::Ssh(SshPart::Port)),
            }
        }
    };
    Ok(SshDestination {
        user,
        host: host.to_owned(),
        port,
        path: path.to_owned(),
    })
}

/// Windows' first check after the grammar: `GetFullPathNameW` must give the name back unchanged (for a `\\.\` name that
/// is string processing alone). `full` is what it gave for `name`.
pub(crate) fn judge_full_path(name: &str, full: &str) -> Result<(), WindowsFault> {
    if name == full {
        return Ok(());
    }
    Err(WindowsFault::Renamed {
        to: super::escape_text(full),
    })
}

/// Windows' second check: the first target `QueryDosDeviceW(L"PIPE")` lists must be the named-pipe file system, so that
/// a `PIPE` device redefined in the caller's logon session is not followed. `targets` is its NUL-separated list.
pub(crate) fn judge_pipe_device(targets: &[u16]) -> Result<(), WindowsFault> {
    let first = targets.split(|&unit| unit == 0).next().unwrap_or_default();
    let first = String::from_utf16_lossy(first);
    if first == PIPE_DEVICE {
        return Ok(());
    }
    Err(WindowsFault::PipeDevice {
        target: super::escape_text(&first),
    })
}

/// Why Windows' checks after the grammar refused a pipe name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WindowsFault {
    /// `GetFullPathNameW` changed the name (escaped).
    Renamed { to: String },
    /// `PIPE` is not `\Device\NamedPipe` (the target, escaped).
    PipeDevice { target: String },
    /// A call failed; the check could not be made.
    Os {
        call: &'static str,
        code: Option<i32>,
    },
}

impl WindowsFault {
    /// `server_address_refused`, as for a refusal by the grammar.
    pub(crate) fn code(&self) -> u64 {
        super::SERVER_ADDRESS_REFUSED
    }
}

impl std::error::Error for WindowsFault {}

impl fmt::Display for WindowsFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowsFault::Renamed { to } => write!(f, "GetFullPathNameW gives {to}"),
            WindowsFault::PipeDevice { target } => {
                write!(f, "the PIPE device is {target}, not {PIPE_DEVICE}")
            }
            WindowsFault::Os { call, code } => write!(f, "{call} failed ({code:?})"),
        }
    }
}
