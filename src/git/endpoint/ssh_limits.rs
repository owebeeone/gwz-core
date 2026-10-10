//! The two limits of libssh2 on Windows that TD5 keeps (gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md), and
//! the plain words for them.
//!
//! libssh2 built on WinCNG verifies RSA host keys only and reads one private-key form from memory: an RSA key in the
//! traditional PEM form. 1.0.17 has the same limits and says so badly (`failed to set hostkey preference`, and an empty
//! `failed to authenticate SSH session:`); the transport keeps the limits and replaces both texts (the migration
//! register's item 8). The wire's failure vocabulary is closed and carries no text, so a limit travels as
//! `UnsupportedOperation` and the host words it from what the open had offered ([`SshLimit::of_failure`]).
use gwz_transport::protocol::{AuthMethod, ErrorCode, Failure};
use std::io;

cfg_if::cfg_if! { if #[cfg(any(windows, test))] { use std::borrow::Cow; } }

/// A limit of the SSH library on this platform, which no retry can lift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SshLimit {
    /// `known_hosts` holds host keys of kinds the library cannot verify, and none it can.
    HostKeys,
    /// The selected key file is not in a form the library reads.
    KeyFile,
}

impl std::fmt::Display for SshLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HostKeys => "no host key in known_hosts is of a kind this SSH library can verify",
            Self::KeyFile => "the key file is not in a form this SSH library reads",
        })
    }
}
impl std::error::Error for SshLimit {}

impl SshLimit {
    /// The setup's error: kind `Unsupported`, with the limit as its source.
    pub(crate) fn into_error(self) -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, self)
    }

    /// The limit `error` stands for, if it was made by [`Self::into_error`].
    pub(crate) fn of_error(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }

    /// The failure a setup reports for this limit. Nothing is retried: it is not a transient error and no
    /// credential reached the server.
    pub(crate) fn failure(self) -> Failure {
        Failure {
            code: ErrorCode::UnsupportedOperation,
            ..Failure::default()
        }
    }

    /// The limit an endpoint failure reports, told by what the open had offered when it ended: the endpoint always
    /// attaches the open's facts, which the driver's own refusals (`UnsupportedOperation` with none) do not carry.
    /// A key file's limit is met after the key is offered, a host key's before any credential.
    pub(crate) fn of_failure(failure: &Failure) -> Option<Self> {
        if failure.code != ErrorCode::UnsupportedOperation {
            return None;
        }
        match failure.facts.as_ref()?.method {
            AuthMethod::None => Some(Self::HostKeys),
            AuthMethod::SshKey => Some(Self::KeyFile),
            _ => None,
        }
    }

    /// The message: the cause, then the fix. `host` and `port` are the open's destination.
    pub(crate) fn reason(self, host: &str, port: u16) -> String {
        match self {
            Self::HostKeys => {
                let port = if port == 22 {
                    String::new()
                } else {
                    format!("-p {port} ")
                };
                format!(
                    "gwz cannot verify {host}: the only host keys your known_hosts file holds for it are ECDSA or Ed25519 \
                     keys, and the SSH library gwz uses on Windows (libssh2 with WinCNG, as gwz 1.0) verifies RSA host keys \
                     only. Add an RSA host key for the server, for example with `ssh-keyscan -t rsa {port}{host}` (compare \
                     the fingerprint it prints with one you trust before adding it to your known_hosts file), then retry."
                )
            }
            Self::KeyFile => "gwz on Windows reads an explicit SSH key file only when it is an RSA key in PEM form, a file \
                              that starts with `-----BEGIN RSA PRIVATE KEY-----`, and this key file is not. For an RSA \
                              key, `ssh-keygen -p -m PEM -P \"\" -N \"\" -f <copy of the key>` rewrites a copy in that \
                              form. ECDSA and Ed25519 keys cannot be used from a file on Windows: load the key into an SSH \
                              agent (Pageant or the OpenSSH agent) instead."
                .to_owned(),
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(any(windows, test))] {
        /// Whether libssh2's WinCNG backend reads a private key held in memory: the traditional PEM form of an RSA
        /// key. Given any other form it does not return (found on dabeest, step 1.4), so a key is checked before the
        /// call. (A key with a passphrase never gets this far: the key snapshot admits unencrypted keys only.)
        pub(crate) fn cng_reads(text: &str) -> bool {
            text.trim_start().starts_with("-----BEGIN RSA PRIVATE KEY-----")
        }

        /// A file's bytes as libssh2 sees a `known_hosts` file on Windows, where it reads with `fopen(.., "r")`: the
        /// C runtime drops a carriage return before a line feed and takes Ctrl-Z for the end of the file. A
        /// carriage return on its own stays.
        pub(crate) fn crt_text(bytes: &[u8]) -> Cow<'_, [u8]> {
            let bytes = bytes
                .iter()
                .position(|&byte| byte == 0x1a)
                .map_or(bytes, |end| &bytes[..end]);
            if !bytes.windows(2).any(|pair| pair == b"\r\n") {
                return Cow::Borrowed(bytes);
            }
            let mut text = Vec::with_capacity(bytes.len());
            for (at, &byte) in bytes.iter().enumerate() {
                if byte != b'\r' || bytes.get(at + 1) != Some(&b'\n') {
                    text.push(byte);
                }
            }
            Cow::Owned(text)
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Whether this platform's library reads a key file of `text`.
        fn reads_key(text: &str) -> bool {
            cng_reads(text)
        }
    } else {
        /// Whether this platform's library reads a key file of `text`: OpenSSL's reads every form.
        fn reads_key(_text: &str) -> bool {
            true
        }
    }
}

/// The limit a key file of `text` meets on this platform, if any.
pub(crate) fn key_form_limit(text: &str) -> Option<SshLimit> {
    (!reads_key(text)).then_some(SshLimit::KeyFile)
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
