//! A host key the endpoint does not trust, and its plain words (design `GwzTransportWindowsParityDesign.md` §14, item
//! 13, U8; the user guide's "Home").
//!
//! Trust is checked before any authentication, and the check has three failing outcomes that libssh2 reports alike:
//! the host is not in `known_hosts`, the file is missing or cannot be read, or the file lists a different key. The
//! wire's failure vocabulary is closed and carries no text, so they travel as `Trust`, and the host words them from
//! what it knows of the endpoint's home ([`HostKeyRefused::reason`]): the file gwz read, which variable named its home,
//! and what to run. It cannot tell the three apart, so it names all three, and puts the fingerprint check before the
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

/// The cause of a refused host key, as the source of the error the trust check returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostKeyRefused;

impl std::fmt::Display for HostKeyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("host key not trusted")
    }
}

impl std::error::Error for HostKeyRefused {}

impl HostKeyRefused {
    /// The setup's error: kind `PermissionDenied`, as the trust check has always returned, with the cause as its source.
    pub(crate) fn into_error(self) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, self)
    }

    /// Whether `error` was made by [`Self::into_error`].
    pub(crate) fn of_error(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|cause| cause.is::<Self>())
    }

    /// The failure a setup reports: `Trust`, which no retry cures.
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

    /// The message. `file` is what the endpoint read and where its home came from; `windows_shells` shows the commands
    /// of both Windows shells, and asks only for RSA keys, the only host keys libssh2 on Windows verifies.
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
        let commands = if windows_shells {
            format!(
                "In PowerShell: {scan} | Add-Content -Encoding ascii \"{path}\"  In Git Bash: {scan} >> \"{path}\""
            )
        } else {
            format!("Run: {scan} >> \"{path}\"")
        };
        let entry = if port == 22 {
            host.to_owned()
        } else {
            format!("\"[{host}]:{port}\"")
        };
        format!(
            "{} gwz read {path}; its home came from {}. If {host} is new to you, first check its key fingerprint \
             with its administrator or provider, then add the key. {commands}  Then compare the fingerprint: \
             ssh-keygen -lf \"{path}\". If the key has changed for a reason you have confirmed, remove the old entry \
             first: ssh-keygen -R {entry} -f \"{path}\". Then retry.",
            cause(&path.to_string()),
            file.source.variables()
        )
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
