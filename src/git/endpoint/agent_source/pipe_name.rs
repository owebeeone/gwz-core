//! The local named pipes the transport connects an agent on, and the refusal of every other form of `SSH_AUTH_SOCK`
//! (TR1.8 §4; the design's L17 and §14 item 13, S5).
//!
//! The grammar is exactly `\\.\pipe\<name>`: the server part is `.`, so a pipe on another machine, `localhost`
//! included, is refused and no SMB authentication can occur; the namespace is `pipe` (Windows matches it whatever its
//! case); and the name is non-empty and holds no separator, so it cannot reach another object by traversal. A MinGW
//! Unix socket path or a file path is not a pipe at all. Windows allows any other character in a pipe name, and a
//! pipe path is at most 256 characters.
use std::{ffi::OsStr, fmt};

/// The OpenSSH Authentication Agent service's pipe, which libssh2 on Windows uses when `SSH_AUTH_SOCK` is unset.
const DEFAULT: &str = r"\\.\pipe\openssh-ssh-agent";
/// What every admitted path starts with.
const NAMESPACE: &str = r"\\.\pipe\";
/// The longest pipe path Windows accepts, in UTF-16 code units.
const MAX_UNITS: usize = 256;
/// How much of a refused value its message shows.
const SHOWN: usize = 100;

/// A pipe path in the form `\\.\pipe\<name>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PipeName(String);

impl PipeName {
    /// The OpenSSH agent's default pipe.
    pub(crate) fn default_pipe() -> Self {
        Self(DEFAULT.to_owned())
    }

    /// The pipe `value` names, or the refusal that names `SSH_AUTH_SOCK`.
    pub(crate) fn parse(value: &OsStr) -> Result<Self, AgentRefusal> {
        match value.to_str() {
            Some(text) if admitted(text) => Ok(Self(text.to_owned())),
            _ => Err(AgentRefusal::of(value)),
        }
    }

    /// The path as written.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the default pipe, however its letters are cased.
    pub(crate) fn is_default(&self) -> bool {
        self.0.eq_ignore_ascii_case(DEFAULT)
    }
}

fn admitted(text: &str) -> bool {
    let Some((namespace, name)) = text.get(..NAMESPACE.len()).zip(text.get(NAMESPACE.len()..))
    else {
        return false;
    };
    namespace.eq_ignore_ascii_case(NAMESPACE)
        && !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['\\', '/', '\0'])
        && text.encode_utf16().count() <= MAX_UNITS
}

/// `SSH_AUTH_SOCK` held a value that is not a local pipe. Its text is the whole message: what was set, what is
/// accepted, and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRefusal {
    shown: String,
}

impl AgentRefusal {
    /// The refusal of `value`, which the message shows shortened and with its control characters and quotes escaped.
    pub(crate) fn of(value: &OsStr) -> Self {
        let text = value.to_string_lossy();
        let mut shown = String::new();
        for (at, ch) in text.chars().enumerate() {
            if at == SHOWN {
                shown.push_str("...");
                break;
            }
            match ch {
                '"' => shown.push_str("\\\""),
                '\n' => shown.push_str("\\n"),
                '\r' => shown.push_str("\\r"),
                '\t' => shown.push_str("\\t"),
                ch if ch.is_control() => shown.push_str(&format!("\\u{{{:x}}}", ch as u32)),
                ch => shown.push(ch),
            }
        }
        Self { shown }
    }
}

impl fmt::Display for AgentRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SSH_AUTH_SOCK is set to \"{}\", which GWZ cannot use as an SSH agent on Windows. GWZ accepts only a pipe \
             on this computer, in the form \\\\.\\pipe\\<name>: a pipe on another machine (\\\\server\\pipe\\..., \
             including \\\\localhost\\pipe\\...), a Git Bash socket path and a file path are refused. Unset \
             SSH_AUTH_SOCK to use the OpenSSH agent's default pipe ({DEFAULT}), or set it to the pipe your agent \
             serves.",
            self.shown
        )
    }
}

impl std::error::Error for AgentRefusal {}
