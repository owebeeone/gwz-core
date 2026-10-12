//! A host key the endpoint does not trust, and its plain words (design `GwzTransportWindowsParityDesign.md` §14, item
//! 13, U8; the user guide's "Home").
//!
//! Trust is checked before any authentication, and the check has two failing outcomes that libssh2 tells apart
//! (decision D12): the host is not in `known_hosts` (or the file is missing), or the file lists it with a different
//! key. The wire's failure vocabulary is closed and carries no text, so both travel as `Trust`; the endpoint records
//! which it found in the runtime's shared [`super::ssh_handoff::Handoff`], and the host words it from that record and
//! from what it knows of the endpoint's home ([`HostKeyRefused::words`]): the file gwz read, which variable named its
//! home, and what to run. An endpoint in another process leaves no record, and the host then words the failure
//! without knowing which ([`HostKeyRefused::reason`]): it names both, and puts the fingerprint check before the
//! commands that add a key.
use super::ssh_home::KnownHostsFile;
use gwz_transport::protocol::{ErrorCode, Failure};
use std::io;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Whether this build runs where `cmd` and PowerShell set an environment variable and quote a path differently,
        /// so a message that names a command shows both.
        pub(crate) const WINDOWS_SHELLS: bool = true;
    } else {
        /// Whether this build runs where `cmd` and PowerShell set an environment variable and quote a path
        /// differently, so a message that names a command shows both.
        pub(crate) const WINDOWS_SHELLS: bool = false;
    }
}

/// What the trust check found when it refused a host key, as the source of the error it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKeyRefused {
    /// `known_hosts` does not list the host, or the file does not exist.
    NotListed,
    /// `known_hosts` lists the host with a different key; `line` is the first such line (from 1) when it was found.
    Changed { line: Option<usize> },
}

impl std::fmt::Display for HostKeyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("host key not trusted")
    }
}

impl std::error::Error for HostKeyRefused {}

impl HostKeyRefused {
    /// The setup's error: kind `PermissionDenied`, as the trust check has always returned, with the finding as its
    /// source.
    pub(crate) fn into_error(self) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, self)
    }

    /// Whether `error` was made by [`Self::into_error`].
    pub(crate) fn of_error(error: &io::Error) -> bool {
        Self::found_in(error).is_some()
    }

    /// What the trust check found, if `error` was made by [`Self::into_error`].
    pub(crate) fn found_in(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }

    /// The failure a setup reports: `Trust`, which no retry cures. What was found does not travel in it.
    pub(crate) fn failure() -> Failure {
        Failure {
            code: ErrorCode::Trust,
            ..Failure::default()
        }
    }

    /// Whether an endpoint's SSH open failure is a refused host key. HTTPS failures also carry `Trust`, and never reach
    /// an SSH open.
    pub(crate) fn of_failure(failure: &Failure) -> bool {
        failure.code == ErrorCode::Trust
    }

    /// The message when the finding is not known: it names all of it. `file` is what the endpoint read and where its
    /// home came from; `windows_shells` shows the commands of both Windows shells, and asks only for RSA keys, the
    /// only host keys libssh2 on Windows verifies.
    pub(crate) fn reason(
        host: &str,
        port: u16,
        file: Option<&KnownHostsFile>,
        windows_shells: bool,
    ) -> String {
        let cause = |file: &str| {
            format!(
                "gwz cannot trust the host key of {host}: {file} does not list it, does not exist or cannot be read, \
                 or lists a different key for it."
            )
        };
        let Some(file) = file else {
            return format!(
                "{} Check the server's key fingerprint with its administrator or provider, add the key to your \
                 known_hosts file (ssh-keyscan lists it), and retry.",
                cause("your known_hosts file")
            );
        };
        let path = file.path.display();
        let entry = if port == 22 {
            host.to_owned()
        } else {
            format!("\"[{host}]:{port}\"")
        };
        format!(
            "{} gwz read {path}; its home came from {}. If {host} is new to you, first check its key fingerprint \
             with its administrator or provider, then add the key. {}  Then compare the fingerprint: \
             ssh-keygen -lf \"{path}\". If the key has changed for a reason you have confirmed, remove the old entry \
             first: ssh-keygen -R {entry} -f \"{path}\". Then retry.",
            cause(&path.to_string()),
            file.source.variables(),
            add_key_commands(host, port, &path.to_string(), windows_shells),
        )
    }

    /// The message for what the trust check found. A host that is not listed keeps the guidance: the file read, the
    /// variable its home came from, the fingerprint check and the commands that add the key. A changed key says
    /// plainly that it differs, that this may be an attack or a re-keyed server, to verify the fingerprint out of
    /// band, and to remove the old line only after verifying; it prints no command that adds a key or removes an
    /// entry.
    pub(crate) fn words(
        self,
        host: &str,
        port: u16,
        file: Option<&KnownHostsFile>,
        windows_shells: bool,
    ) -> String {
        match self {
            Self::NotListed => {
                let Some(file) = file else {
                    return format!(
                        "gwz cannot trust the host key of {host}: your known_hosts file does not list it. Check the \
                         server's key fingerprint with its administrator or provider, add the key to your known_hosts \
                         file (ssh-keyscan lists it), and retry."
                    );
                };
                let path = file.path.display().to_string();
                format!(
                    "gwz cannot trust the host key of {host}: {path} does not list it, or does not exist or cannot be \
                     read. gwz read {path}; its home came from {}. If {host} is new to you, first check its key \
                     fingerprint with its administrator or provider, then add the key. {}  Then compare the \
                     fingerprint: ssh-keygen -lf \"{path}\". Then retry.",
                    file.source.variables(),
                    add_key_commands(host, port, &path, windows_shells),
                )
            }
            Self::Changed { line } => {
                let recorded = match (file, line) {
                    (Some(file), Some(line)) => format!("{} at line {line}", file.path.display()),
                    (Some(file), None) => file.path.display().to_string(),
                    (None, _) => "your known_hosts file".to_owned(),
                };
                let read = file.map_or_else(String::new, |file| {
                    format!(
                        " gwz read {}; its home came from {}.",
                        file.path.display(),
                        file.source.variables()
                    )
                });
                let compare = file.map_or_else(String::new, |file| {
                    format!(
                        " ssh-keygen -lf \"{}\" shows the fingerprints on record.",
                        file.path.display()
                    )
                });
                format!(
                    "gwz refuses to connect to {host}: the host key it presented differs from the one recorded in \
                     {recorded}.{read} This may be an attack, with someone between you and the server, or the server \
                     may have been re-keyed. Verify the server's key fingerprint out of band, with its administrator \
                     or provider, before you trust it.{compare} Remove the old line only after you have verified \
                     the new key, then retry."
                )
            }
        }
    }
}

/// The commands that add `host`'s key to the file at `path`, for the shells of this build.
fn add_key_commands(host: &str, port: u16, path: &str, windows_shells: bool) -> String {
    let scan_port = if port == 22 {
        String::new()
    } else {
        format!("-p {port} ")
    };
    let scan = if windows_shells {
        format!("ssh-keyscan -t rsa {scan_port}{host}")
    } else {
        format!("ssh-keyscan {scan_port}{host}")
    };
    if windows_shells {
        format!(
            "In PowerShell: {scan} | Add-Content -Encoding ascii \"{path}\"  In Git Bash: {scan} >> \"{path}\""
        )
    } else {
        format!("Run: {scan} >> \"{path}\"")
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
