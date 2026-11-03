//! Endpoint-local configured `git credential fill` lookup for HTTPS.
//!
//! The helper is intentionally invoked directly.  Its environment, pipes,
//! output and child lifetime are all bounded by this module; no credential
//! bytes leave the endpoint adapter.

use super::{https_destination::Destination, https_worker::native::NativeCaller};
use crate::git::spawn_gate::{SpawnGate, SpawnHold};
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
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;
const OUTPUT_LIMIT: usize = 16 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_millis(500);
/// How often a helper's owner looks again at a process tree that has not yet emptied.
const DRAIN_POLL: Duration = Duration::from_millis(2);
/// How often a launch that waits for the host's spawn gate looks again. A hold lasts one spawn, so a wait is short.
const GATE_POLL: Duration = Duration::from_millis(1);
/// Live helper processes one host admits, retained unreaped children included.
const HELPER_SLOTS: usize = 8;

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) executable: PathBuf,
    pub(crate) environment: Vec<(OsString, OsString)>,
}

/// One operation's authentication view (plan step 1.5; contract section 7, Safety P2-4): the credential helper's
/// executable and the environment it runs with, so the account tokens (`GH_TOKEN`) and the configuration
/// (`GIT_CONFIG_GLOBAL`) the helper reads. It is the provider of the operation's [`gwz_connection::AuthContext`], which
/// lives in that operation's binding and reaches an adapter only with the connects and exchanges started for that
/// operation: it is never part of an endpoint profile or an endpoint instance, so operations with equal profiles share
/// connections' instances but never one another's helper. It has no `Debug`.
pub(crate) struct OperationAuth {
    helper: Config,
    /// The caller captured on the operation's original entry (R3, Code P2-2; the HTTPS model draft r5 section 3.2):
    /// the logon session this operation's challenges are answered by, and no other operation's. Retained native
    /// cleanup clones it and never holds the context.
    native: Option<NativeCaller>,
    /// The operation's credential record (HTTPS model draft r5 section 3(b)): it lives as long as this provider.
    needs: needs::Needs,
}
/// What the scope's digest is of, so that no other digest in gwz can equal it.
const SCOPE_DOMAIN: &[u8] = b"gwz-operation-auth-scope/1";
impl OperationAuth {
    /// The context of an operation whose helper is `helper` and whose logon-session caller is `native`, when it has
    /// one. Its scope is the SHA-256 of the helper's executable, of its whole environment, sorted, and of the caller's
    /// identity: two operations share a connection only when their helpers would run alike for the same caller, and
    /// any changed variable (a token, a configuration file, `PATH`) or another caller gives a scope of its own. The
    /// caller enters as its `CallerIdentity` (R3b-win, draft r5 section 3.2 item 3): a digest of non-secret identifiers,
    /// or a value unique to its capture when they could not be read. An operation with no caller folds a tag of its own
    /// (it has no native route, and its helper runs as the process). The digest stays in memory beside the values it is
    /// of and is never logged or sent. The provider retains the operation's credentials, so the context is
    /// single-admission (HTTPS model draft r5 3(b), A8): one admission of it, or of any clone, registers, and every other
    /// `admit` is refused `InvalidInput`; a served bind builds a fresh one from its own capture.
    pub(crate) fn context(
        helper: Config,
        native: Option<NativeCaller>,
    ) -> gwz_connection::AuthContext {
        Self::with_needs(helper, native, needs::Needs::new())
    }
    /// `context`, over the record `needs`.
    fn with_needs(
        helper: Config,
        native: Option<NativeCaller>,
        needs: needs::Needs,
    ) -> gwz_connection::AuthContext {
        use sha2::{Digest, Sha256};
        let mut entries: Vec<_> = helper.environment.iter().collect();
        entries.sort();
        let mut digest = Sha256::new();
        let mut field = |bytes: &[u8]| {
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        };
        field(SCOPE_DOMAIN);
        field(helper.executable.as_os_str().as_encoded_bytes());
        field(&(entries.len() as u64).to_le_bytes());
        let mut charge = helper.executable.as_os_str().len()
            + 64
            + needs::CHARGE
            + native
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<NativeCaller>());
        for (name, value) in entries {
            field(name.as_encoded_bytes());
            field(value.as_encoded_bytes());
            charge += name.len() + value.len() + 2 * std::mem::size_of::<OsString>();
        }
        match &native {
            Some(caller) => {
                field(b"caller");
                field(caller.identity().digest());
            }
            None => {
                field(b"no caller");
            }
        }
        let scope = gwz_connection::AuthScope::new(digest.finalize().into());
        gwz_connection::AuthContext::single_admission(
            charge,
            scope,
            Arc::new(Self {
                helper,
                native,
                needs,
            }),
        )
    }
    /// The view `context` holds, when it is one of these.
    pub(crate) fn of(context: &gwz_connection::AuthContext) -> Option<&Self> {
        context.provider().downcast_ref::<Self>()
    }
    /// The helper this operation's credential lookups run.
    pub(crate) fn helper(&self) -> &Config {
        &self.helper
    }
    /// The logon-session caller this operation captured on its original entry, if its entry gave one.
    pub(crate) fn native(&self) -> Option<&NativeCaller> {
        self.native.as_ref()
    }
    /// This operation's credential record.
    pub(crate) fn needs(&self) -> &needs::Needs {
        &self.needs
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// One bound of the operation's record (HTTPS model draft r5 3(b)) that a host row brings within reach.
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum Bound {
            /// Entries of the key table.
            Keys,
            /// Distinct credentials retained.
            Credentials,
            /// All retained credential bytes.
            Bytes,
            /// Rejected credentials' fingerprints.
            Fingerprints,
        }
        impl OperationAuth {
            /// `context`, but its record's randomness failed at creation: it is `Exhausted` from the start (draft r5 X1).
            pub(crate) fn context_without_randomness(
                helper: Config,
                native: Option<NativeCaller>,
            ) -> gwz_connection::AuthContext {
                Self::with_needs(helper, native, needs::Needs::keyed(None))
            }
            /// The small test bounds of draft r5 sub-step H1d: fills the record, at an origin of its own, until `room` of
            /// `bound` is left, so that a host row reaches the bound with a few real exchanges. The fillers are what the
            /// record keeps: `Rejected` keys with no credential (`Keys`); distinct short credentials (`Credentials`);
            /// credentials of the largest size and a remainder (`Bytes`); distinct rejected credentials (`Fingerprints`).
            pub(crate) fn crowd(&self, bound: Bound, room: usize) {
                let origin = gwz_transport::pool::Key::https("filler.invalid", 443);
                let key = |index: usize| {
                    let url = format!("https://filler.invalid/filler-{index}");
                    let destination = Destination::parse(&url).expect("a filler URL");
                    let service = gwz_transport::protocol::GitService::UploadPackAdvertisement;
                    needs::NeedKey::new(&origin, &destination, service)
                };
                // A credential of `size` bytes, username and password, distinct for each `index`.
                let credential = |index: usize, size: usize| {
                    let username = format!("f{index}");
                    assert!(size > username.len(), "a filler credential of {size} bytes");
                    let mut owned = Vec::with_capacity(username.len() + 1);
                    owned.extend_from_slice(username.as_bytes());
                    Secret::new(owned, vec![b'p'; size - username.len()])
                };
                match bound {
                    Bound::Keys => {
                        for index in 0..needs::KEYS - room {
                            let _ = self.needs.reject(&key(index), None);
                        }
                    }
                    Bound::Credentials => {
                        for index in 0..needs::CREDENTIALS - room {
                            let filler = credential(index, 8);
                            self.needs.record(&key(index), filler).expect("a filler credential");
                        }
                    }
                    Bound::Bytes => {
                        let mut left = needs::RETAINED_BYTES - room;
                        let mut index = 0;
                        while left > 0 {
                            let size = left.min(needs::CREDENTIAL_BYTES);
                            let filler = credential(index, size);
                            self.needs.record(&key(index), filler).expect("a filler credential");
                            left -= size;
                            index += 1;
                        }
                    }
                    Bound::Fingerprints => {
                        for index in 0..needs::FINGERPRINTS - room {
                            let _ = self.needs.reject(&key(index), Some(&credential(index, 8)));
                        }
                    }
                }
            }
            /// Poisons the record's lock, as a panic under it would (draft r5 X1).
            pub(crate) fn poison(&self) {
                self.needs.poison();
            }
        }
    }
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
    /// `git credential fill` exited with failure. Only the runner sees this: the lookup says which of
    /// [`Self::NoHelper`] and [`Self::HelperDeclined`] it was, from the configuration view.
    HelperRejected,
    /// `git credential fill` failed and the configuration view holds no credential helper for the URL.
    NoHelper,
    /// `git credential fill` failed and the view holds a helper for the URL: it gave no password (cancelled,
    /// nothing stored for the server, or it failed; Git's prompts are off, so the three end alike).
    HelperDeclined,
    ConfigurationRefused,
    Timeout,
    /// The lookup's clock ran out while it waited for the host's spawn gate (`git/spawn_gate.rs`): another spawn of
    /// the host, such as a credential helper waiting for a sign-in, held it, and this lookup's helper was never started.
    GateTimeout,
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
            Self::NoHelper => "no credential helper is configured for the HTTPS remote",
            Self::HelperDeclined => "the configured HTTPS credential helper gave no password",
            Self::Timeout => "HTTPS credential helper timed out",
            Self::GateTimeout => {
                "HTTPS credential helper was not started: another credential helper of this host held the spawn gate"
            }
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
            | Self::NoHelper
            | Self::HelperDeclined
            | Self::ConfigurationRefused => ErrorCode::Authentication,
            Self::Timeout | Self::GateTimeout | Self::AllocationTimeout => ErrorCode::Timeout,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::Io | Self::CleanupPending => ErrorCode::Io,
        }
    }
}

// The operation's credential record (HTTPS model draft r5, sub-step H1a), which the HTTPS adapter's exchanges drive.
pub(crate) mod needs;
mod secret;
pub(crate) use secret::{Secret, SecretHeader};
use secret::{SecretBuffer, parse_secret};
/// Real host helper admission ledger, and the host's spawn gate (`git/spawn_gate.rs`): the helpers' launches hold
/// the gate for their inheritance windows, and the host's other spawns, which share it, wait for those windows.
#[derive(Clone)]
pub(crate) struct HelperSlots(pub(super) Arc<Semaphore>, SpawnGate, Arc<AtomicUsize>);
impl HelperSlots {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            /// Slots on a gate of their own, for a test; the product builds them on the composition's gate.
            pub(crate) fn new() -> Self {
                Self::with_gate(SpawnGate::new())
            }
        }
    }
    /// Slots whose launches use `gate`, which the host's other spawns hold too.
    pub(crate) fn with_gate(gate: SpawnGate) -> Self {
        Self(
            Arc::new(Semaphore::new(HELPER_SLOTS)),
            gate,
            Arc::new(AtomicUsize::new(HELPER_SLOTS)),
        )
    }
    /// Holds the ledger to `slots` live helpers: the cleanup domain's helper budget (contract section 10), which the
    /// connection host's factory applies as it makes its first adapter, before any helper of the host has started.
    /// The difference from the slots now set is added or taken from the free ones, so applying the same number again
    /// changes nothing; a helper already holding a slot keeps it, and a shrink can only take the free ones.
    pub(crate) fn fit_to(&self, slots: usize) {
        let before = self.2.swap(slots, Ordering::AcqRel);
        if slots > before {
            self.0.add_permits(slots - before);
        } else if slots < before {
            self.0.forget_permits(before - slots);
        }
    }
    /// The host's spawn gate: a handle to the one the launches hold.
    pub(crate) fn gate(&self) -> &SpawnGate {
        &self.1
    }
}
// The helper process owner, which both platforms compile (step 4.2): the process tree, the owner that ends it, the
// pipes and the runner that drives one helper child.
mod owner;
mod pipes;
mod process_tree;
mod runner;
mod wiped;
pub(crate) use owner::AuthOwner;
use owner::HelperJob;
use process_tree::{HelperChild, ProcessTree};
// Executable discovery, the configuration view, the file reads behind it and the lookup that drives them (step
// 4.3) compile on both platforms. HTTPS's lookup is `lookup_until` (step 4.4); SSH's password route (step 4.5)
// uses `lookup_setup`.
mod executable;
mod file_worker;
mod lookup;
mod view;
pub(crate) use lookup::{LookupAdmission, lookup_setup, lookup_until};
use owner::ActiveGuard;
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod helper_fixture;
        use lookup::{lookup_owned, lookup_with_budget};
        use owner::PendingChild;
        use pipes::write_request;
        mod test_support;
        mod runner_tests;
        mod needs_tests;
    }
}
