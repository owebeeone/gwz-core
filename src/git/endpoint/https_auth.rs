//! Endpoint-local configured `git credential fill` lookup for HTTPS.
//!
//! The helper is intentionally invoked directly.  Its environment, pipes,
//! output and child lifetime are all bounded by this module; no credential
//! bytes leave the endpoint adapter.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{ffi::OsString, fmt, io, path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
cfg_if::cfg_if! { if #[cfg(unix)] {
    use super::https_destination::Destination;
    use std::{ffi::OsStr, sync::{atomic::{AtomicUsize, Ordering}, Mutex}, time::Duration};
    use tokio::sync::OwnedSemaphorePermit;
    use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, process::{Child, Command}, time::{Instant, sleep_until, timeout}};
    use tokio_util::sync::CancellationToken;
    const OUTPUT_LIMIT: usize = 16 * 1024;
    const CLEANUP_GRACE: Duration = Duration::from_millis(500);
} }
/// Live helper processes one host admits, retained unreaped children included.
const HELPER_SLOTS: usize = 8;

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) executable: PathBuf,
    pub(crate) environment: Vec<(OsString, OsString)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthError {
    MissingExecutable,
    SpawnFailed,
    Io,
    Pipe(io::ErrorKind),
    ControlCharacter,
    UsernameColon,
    NotUtf8,
    MissingNewline,
    MissingCredential,
    MalformedOutput,
    OutputTooLarge,
    HelperRejected,
    ConfigurationRefused,
    Timeout,
    AllocationTimeout,
    Cancelled,
    CleanupPending,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingExecutable => "HTTPS credential helper executable is missing",
            Self::SpawnFailed => "HTTPS credential helper could not be started",
            Self::Io => "HTTPS credential helper process wait failed",
            Self::Pipe(_) => "HTTPS credential helper pipe failed",
            Self::ControlCharacter => {
                "HTTPS credential helper credential holds a control character"
            }
            Self::UsernameColon => "HTTPS credential helper username holds a colon",
            Self::NotUtf8 => "HTTPS credential helper output is not UTF-8",
            Self::MissingNewline => "HTTPS credential helper output has no final newline",
            Self::MissingCredential => "HTTPS credential helper returned no usable credential",
            Self::MalformedOutput => "HTTPS credential helper returned malformed output",
            Self::OutputTooLarge => "HTTPS credential helper output exceeded its limit",
            Self::ConfigurationRefused => "HTTPS credential configuration was refused",
            Self::HelperRejected => "HTTPS credential helper rejected the request",
            Self::Timeout => "HTTPS credential helper timed out",
            Self::AllocationTimeout => "HTTPS credential helper admission timed out",
            Self::Cancelled => "HTTPS credential helper was cancelled",
            Self::CleanupPending => "HTTPS credential helper cleanup remains pending",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AuthError {}

impl AuthError {
    pub(crate) fn code(self) -> gwz_transport::protocol::ErrorCode {
        use gwz_transport::protocol::ErrorCode;
        match self {
            Self::MissingExecutable | Self::SpawnFailed => ErrorCode::Unavailable,
            Self::Pipe(_)
            | Self::ControlCharacter
            | Self::UsernameColon
            | Self::NotUtf8
            | Self::MissingNewline
            | Self::MissingCredential
            | Self::MalformedOutput
            | Self::OutputTooLarge
            | Self::HelperRejected
            | Self::ConfigurationRefused => ErrorCode::Authentication,
            Self::Timeout | Self::AllocationTimeout => ErrorCode::Timeout,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::Io | Self::CleanupPending => ErrorCode::Io,
        }
    }
}

mod secret;
pub(crate) use secret::{Secret, SecretHeader};
/// Real host helper admission ledger; Windows qualification never acquires it.
#[derive(Clone)]
pub(crate) struct HelperSlots(pub(super) Arc<Semaphore>);
impl HelperSlots {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Semaphore::new(HELPER_SLOTS)))
    }
}
cfg_if::cfg_if! { if #[cfg(unix)] {
    mod executable;
    mod owner;
    mod lookup;
    mod runner;
    mod view;
    mod file_worker;
    pub(crate) use owner::AuthOwner;
    pub(crate) use lookup::{lookup_until, lookup_setup, LookupAdmission};
    use owner::{ActiveGuard, HelperJob};
    use secret::{parse_secret, SecretBuffer};
} }
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        use lookup::{write_request, lookup_owned, lookup_with_budget};
        use owner::PendingChild;
        mod test_support;
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                mod runner_tests;
            }
        }
    }
}
