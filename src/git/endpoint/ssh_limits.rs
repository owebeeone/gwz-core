//! The limits of the SSH library that TD5 keeps (gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md), and the plain
//! words for them: two of libssh2 on Windows, and one on every platform, the key file with a passphrase (D17).
//!
//! libssh2 built on WinCNG verifies RSA host keys only and reads one private-key form from memory: an RSA key in the
//! traditional PEM form. 1.0.17 has the same limits and says so badly (`failed to set hostkey preference`, and an empty
//! `failed to authenticate SSH session:`); the transport keeps the limits and replaces both texts (the migration
//! register's item 8). The wire's failure vocabulary is closed and carries no text, so a limit travels as
//! `UnsupportedOperation` and the host words it from what the open had offered ([`SshLimit::of_failure`]). A key file
//! with a passphrase is refused as it is admitted (`ssh_key_container`), on every platform, because libssh2 is never
//! given a passphrase and gwz cannot ask for one.
use super::ssh_home::KnownHostsFile;
use gwz_transport::protocol::{AuthMethod, ErrorCode, Failure};
use std::io;

cfg_if::cfg_if! { if #[cfg(any(windows, test))] { use std::borrow::Cow; } }

/// A limit of the SSH library, or of what gwz gives it, which no retry can lift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SshLimit {
    /// `known_hosts` holds host keys of kinds the library cannot verify, and none it can.
    HostKeys,
    /// The selected key file is not in a form the library reads.
    KeyFile,
    /// The selected key file is protected by a passphrase, which gwz cannot ask for and the library is never given.
    /// Met on every platform, when the key file is admitted.
    EncryptedKeyFile,
}

impl std::fmt::Display for SshLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HostKeys => "no host key in known_hosts is of a kind this SSH library can verify",
            Self::KeyFile => "the key file is not in a form this SSH library reads",
            Self::EncryptedKeyFile => "the key file is protected by a passphrase",
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
    /// A key file's form is met after the key is offered, a host key's before any credential, and a key file's
    /// passphrase when the file is admitted, which is after the key file was selected (its method) and before it
    /// is offered, as the agent step is marked before its first request (`agent_reason`).
    pub(crate) fn of_failure(failure: &Failure) -> Option<Self> {
        if failure.code != ErrorCode::UnsupportedOperation {
            return None;
        }
        let facts = failure.facts.as_ref()?;
        match facts.method {
            AuthMethod::None => Some(Self::HostKeys),
            AuthMethod::SshKey if facts.credential_offered => Some(Self::KeyFile),
            AuthMethod::SshKey => Some(Self::EncryptedKeyFile),
            _ => None,
        }
    }

    /// The message: the cause, then the fix. `host` and `port` are the open's destination, and `known_hosts` is the
    /// file the endpoint read with the variable that named its home (design section 14, item 13, U8): a host-key limit
    /// prints both, so the user edits the file gwz actually read. `key_file` is the key file the open selected, as the
    /// user can name it to a command, which a passphrase's words print.
    pub(crate) fn reason(
        self,
        host: &str,
        port: u16,
        known_hosts: Option<&KnownHostsFile>,
        key_file: Option<&str>,
    ) -> String {
        match self {
            Self::HostKeys => {
                let port = if port == 22 {
                    String::new()
                } else {
                    format!("-p {port} ")
                };
                let cause = format!(
                    "gwz cannot verify {host}: the only host keys your known_hosts file holds for it are ECDSA or Ed25519 \
                     keys, and the SSH library gwz uses on Windows (libssh2 with WinCNG, as gwz 1.0) verifies RSA host keys \
                     only."
                );
                let Some(file) = known_hosts else {
                    return format!(
                        "{cause} Add an RSA host key for the server, for example with `ssh-keyscan -t rsa {port}{host}` \
                         (compare the fingerprint it prints with one you trust before adding it to your known_hosts file), \
                         then retry."
                    );
                };
                let path = file.path.display();
                format!(
                    "{cause} gwz read {path}; its home came from {}. Add an RSA host key for the server. In PowerShell: \
                     ssh-keyscan -t rsa {port}{host} | Add-Content -Encoding ascii \"{path}\"  In Git Bash: \
                     ssh-keyscan -t rsa {port}{host} >> \"{path}\"  Then compare the fingerprint, ssh-keygen -lf \"{path}\", \
                     with one you trust before you rely on it (a key fetched over the network proves nothing by itself), \
                     and retry.",
                    file.source.variables()
                )
            }
            Self::KeyFile => "gwz on Windows reads an explicit SSH key file only when it is an RSA key in PEM form, a file \
                              that starts with `-----BEGIN RSA PRIVATE KEY-----`, and this key file is not. For an RSA \
                              key, `ssh-keygen -p -m PEM -P \"\" -N \"\" -f <copy of the key>` rewrites a copy in that \
                              form. ECDSA and Ed25519 keys cannot be used from a file on Windows: load the key into an SSH \
                              agent (Pageant or the OpenSSH agent) instead."
                .to_owned(),
            Self::EncryptedKeyFile => encrypted_key_words(key_file),
        }
    }
}

/// The words for a key file with a passphrase (D17): the cause, then the two ways out. They are the same on every
/// platform, with the Windows differences said where they apply, because the refusal is made on all of them. The
/// `-m PEM` form is the Windows RSA-PEM limit's own ([`SshLimit::KeyFile`]).
fn encrypted_key_words(key_file: Option<&str>) -> String {
    let (named, add) = match key_file {
        Some(path) => (
            format!("The SSH key file {path}"),
            format!("ssh-add \"{path}\""),
        ),
        None => (
            "The selected SSH key file".to_owned(),
            "ssh-add <the key file>".to_owned(),
        ),
    };
    format!(
        "{named} is protected by a passphrase, and gwz cannot ask for a passphrase, so it cannot use the file. Either \
         load the key into an SSH agent with {add} (on Windows the agent is the OpenSSH Authentication Agent service, \
         which must be running, or Pageant) and do not select a key file, so that gwz uses the agent: drop --identity, \
         --remote-identity and the remote's gwzSshIdentity setting. Or remove the passphrase from a copy of the key \
         with ssh-keygen -p -f <copy of the key>, which asks for the old passphrase and then for a new one (press \
         Enter at the new one), and select the copy. On Windows gwz reads a key file only if it is an RSA key in PEM \
         form, so for an RSA key use ssh-keygen -p -m PEM -f <copy of the key>; an ECDSA or Ed25519 key can only come \
         from an agent there."
    )
}

cfg_if::cfg_if! {
    if #[cfg(any(windows, test))] {
        /// Whether libssh2's WinCNG backend reads a private key held in memory: the traditional PEM form of an RSA
        /// key. Given any other form it does not return (found on dabeest, step 1.4), so a key is checked before the
        /// call. (A key with a passphrase never gets this far: the key snapshot admits unencrypted keys only.)
        ///
        /// The library's PEM reader (`_libssh2_pem_parse_memory`) searches for a line equal to the header, then for
        /// one equal to the footer, and repeats the search over empty lines past the end of the text until it finds
        /// one; it also drops the last character of a line of 127 or more. So the text is admitted only when its
        /// first non-blank line is exactly the RSA header, the first armor line after that is exactly the matching
        /// footer (the block is one RSA block, and the key that the container check admitted), and every line is
        /// shorter than 127 characters. Text after the footer is never read.
        pub(crate) fn cng_reads(text: &str) -> bool {
            const BEGIN: &str = "-----BEGIN RSA PRIVATE KEY-----";
            const END: &str = "-----END RSA PRIVATE KEY-----";
            const LONGEST_LINE: usize = 126;
            // Carriage return and line feed each end a line, as the library's reader takes them.
            let mut lines = text.split(['\r', '\n']);
            if lines.clone().any(|line| line.len() > LONGEST_LINE) {
                return false;
            }
            if lines.find(|line| !line.trim().is_empty()) != Some(BEGIN) {
                return false;
            }
            for line in lines {
                if line.starts_with("-----BEGIN ") {
                    return false;
                }
                if line.starts_with("-----END ") {
                    return line == END;
                }
            }
            false
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
