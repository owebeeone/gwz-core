//! Endpoint-local configured `git credential fill` lookup for HTTPS.
//!
//! The helper is intentionally invoked directly.  Its environment, pipes,
//! output and child lifetime are all bounded by this module; no credential
//! bytes leave the endpoint adapter.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, Command},
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;
cfg_if::cfg_if! { if #[cfg(unix)] {
    use super::https_destination::Destination;
} }
const OUTPUT_LIMIT: usize = 16 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_millis(500);
/// How often a helper's owner looks again at a process tree that has not yet emptied.
const DRAIN_POLL: Duration = Duration::from_millis(2);
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
use secret::{SecretBuffer, parse_secret};
/// Real host helper admission ledger; Windows qualification never acquires it.
#[derive(Clone)]
pub(crate) struct HelperSlots(pub(super) Arc<Semaphore>);
impl HelperSlots {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Semaphore::new(HELPER_SLOTS)))
    }
}
// The helper process owner, which both platforms compile (step 4.2): the process tree, the owner that ends it, the
// pipes and the runner that drives one helper child.
mod owner;
mod pipes;
mod process_tree;
mod runner;
pub(crate) use owner::AuthOwner;
use owner::HelperJob;
use process_tree::ProcessTree;
// The rest of the helper stack is Unix-only until step 4.3 (discovery, the configuration view, file reads) and step
// 4.4 (admission) bring it to Windows.
cfg_if::cfg_if! { if #[cfg(unix)] {
    mod executable;
    mod lookup;
    mod view;
    mod file_worker;
    pub(crate) use lookup::{lookup_until, lookup_setup, LookupAdmission};
    use owner::ActiveGuard;
} }
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod helper_fixture;
    }
}
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        use lookup::{lookup_owned, lookup_with_budget};
        use owner::PendingChild;
        use pipes::write_request;
        mod test_support;
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                mod runner_tests;
            }
        }
    }
}
