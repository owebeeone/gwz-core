//! Endpoint-local `gh auth git-credential get` lookup for HTTPS.
//!
//! The helper is intentionally invoked directly.  Its environment, pipes,
//! output and child lifetime are all bounded by this module; no credential
//! bytes leave the endpoint adapter.

use super::https_destination::Destination;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    path::PathBuf,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, Command},
    time::{Instant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 16 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_millis(500);
/// Live helper processes one host admits, retained unreaped children included.
const HELPER_SLOTS: usize = 8;

/// One host context's HTTPS helper slots, shared by every endpoint its driver
/// opens (GwzCoreSessionDesign §5.6): the driver creates them once and hands
/// them to each endpoint's `AuthOwner`. A standalone endpoint is its own host.
#[derive(Clone)]
pub(crate) struct HelperSlots(Arc<Semaphore>);

impl HelperSlots {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Semaphore::new(HELPER_SLOTS)))
    }
}

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) executable: PathBuf,
    pub(crate) environment: Vec<(OsString, OsString)>,
}

struct PendingChild {
    child: Child,
    helper_slot: OwnedSemaphorePermit,
}

struct OrphanChild {
    owner_id: u64,
    pending: PendingChild,
}

fn orphan_registry() -> &'static Mutex<Vec<OrphanChild>> {
    static ORPHANS: OnceLock<Mutex<Vec<OrphanChild>>> = OnceLock::new();
    ORPHANS.get_or_init(|| Mutex::new(Vec::new()))
}

struct AuthOwnerInner {
    id: u64,
    cancelled: CancellationToken,
    active: Arc<AtomicUsize>,
    reaping: AtomicUsize,
    pending: Mutex<Vec<PendingChild>>,
    helper_slots: HelperSlots,
}

impl Drop for AuthOwnerInner {
    fn drop(&mut self) {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        if pending.is_empty() {
            return;
        }
        orphan_registry()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(pending.into_iter().map(|pending| OrphanChild {
                owner_id: self.id,
                pending,
            }));
    }
}

/// Endpoint-scoped ownership for credential helper processes.
///
/// Helper admission comes from the host's `HelperSlots`, so another host's
/// live or retained helpers never exhaust it. Retained children and their
/// admission permits live in this owner only. A
/// dropped owner transfers them to an owner-ID-tagged fallback queue, so
/// `kill_on_drop` is not used to release a still-reserved permit and cleanup
/// cannot be accidentally attributed to another endpoint.
#[derive(Clone)]
pub(crate) struct AuthOwner {
    inner: Arc<AuthOwnerInner>,
}

impl AuthOwner {
    pub(crate) fn new(helper_slots: HelperSlots) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            inner: Arc::new(AuthOwnerInner {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                cancelled: CancellationToken::new(),
                active: Arc::new(AtomicUsize::new(0)),
                reaping: AtomicUsize::new(0),
                pending: Mutex::new(Vec::new()),
                helper_slots,
            }),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.inner.id
    }

    /// Cancel all owned active helper lookups. Retained children remain in the
    /// owner registry until `reap_pending` joins them or the owner is dropped.
    pub(crate) fn cancel(&self) {
        self.inner.cancelled.cancel();
    }

    pub(crate) fn active_count(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    pub(crate) fn pending_cleanup_count(&self) -> usize {
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.len() + self.inner.reaping.load(Ordering::Acquire)
    }

    /// Reaping owns a guarded batch, including while this future is cancelled.
    /// Concurrent transfers into the owner remain visible in the returned count.
    pub(crate) async fn reap_pending(&self, deadline: Instant) -> usize {
        let children = {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let children = std::mem::take(&mut *pending);
            self.inner
                .reaping
                .fetch_add(children.len(), Ordering::AcqRel);
            children
        };
        let mut batch = ReapBatch {
            owner: self.clone(),
            children,
        };
        let mut index = 0;
        while index < batch.children.len() && Instant::now() < deadline {
            let result =
                tokio::time::timeout_at(deadline, batch.children[index].child.wait()).await;
            if matches!(result, Ok(Ok(_))) {
                batch.children.swap_remove(index);
                self.inner.reaping.fetch_sub(1, Ordering::AcqRel);
            } else {
                index += 1;
            }
        }
        drop(batch);
        self.pending_cleanup_count()
    }

    fn reap_ready(&self) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain_mut(|pending| !matches!(pending.child.try_wait(), Ok(Some(_))));
    }

    fn retain_pending(&self, pending: PendingChild) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(pending);
    }
}

/// Unreaped children always keep their permits and their original owner.
struct ReapBatch {
    owner: AuthOwner,
    children: Vec<PendingChild>,
}
impl Drop for ReapBatch {
    fn drop(&mut self) {
        let count = self.children.len();
        let mut pending = self
            .owner
            .inner
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for mut child in self.children.drain(..) {
            let _ = child.child.start_kill();
            pending.push(child);
        }
        self.owner.inner.reaping.fetch_sub(count, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthError {
    MissingExecutable,
    SpawnFailed,
    Io,
    MissingCredential,
    MalformedOutput,
    OutputTooLarge,
    HelperRejected,
    Timeout,
    Cancelled,
    CleanupPending,
    Capacity,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingExecutable => "HTTPS credential helper executable is missing",
            Self::SpawnFailed => "HTTPS credential helper could not be started",
            Self::Io => "HTTPS credential helper I/O failed",
            Self::MissingCredential => "HTTPS credential helper returned no usable credential",
            Self::MalformedOutput => "HTTPS credential helper returned malformed output",
            Self::OutputTooLarge => "HTTPS credential helper output exceeded its limit",
            Self::HelperRejected => "HTTPS credential helper rejected the request",
            Self::Timeout => "HTTPS credential helper timed out",
            Self::Cancelled => "HTTPS credential helper was cancelled",
            Self::CleanupPending => "HTTPS credential helper cleanup remains pending",
            Self::Capacity => "HTTPS credential helper capacity is exhausted",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AuthError {}

impl AuthError {
    pub(crate) fn code(self) -> gwz_transport::protocol::ErrorCode {
        use gwz_transport::protocol::ErrorCode;
        match self {
            Self::MissingExecutable
            | Self::SpawnFailed
            | Self::MissingCredential
            | Self::MalformedOutput
            | Self::OutputTooLarge
            | Self::HelperRejected => ErrorCode::Authentication,
            Self::Timeout => ErrorCode::Timeout,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::Io | Self::CleanupPending => ErrorCode::Io,
            Self::Capacity => ErrorCode::Capacity,
        }
    }
}

pub(crate) struct Secret {
    username: Vec<u8>,
    password: Vec<u8>,
}

impl Secret {
    pub(crate) fn header(&self) -> String {
        let mut credential = Vec::with_capacity(self.username.len() + self.password.len() + 1);
        credential.extend_from_slice(&self.username);
        credential.push(b':');
        credential.extend_from_slice(&self.password);
        let encoded = STANDARD.encode(credential.as_slice());
        credential.fill(0);
        format!("Basic {encoded}")
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(REDACTED)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.username.fill(0);
        self.password.fill(0);
    }
}

pub(crate) async fn lookup(
    config: &Config,
    destination: &Destination,
    deadline: Instant,
    cancelled: &CancellationToken,
) -> Result<Secret, AuthError> {
    // Compatibility wrapper for callers that have not yet adopted endpoint
    // ownership. The temporary owner, its own host, ensures a retained child
    // cannot enter a process-wide cleanup pool; endpoint callers should use
    // `lookup_owned`.
    let owner = AuthOwner::new(HelperSlots::new());
    lookup_owned(&owner, config, destination, deadline, cancelled).await
}

pub(crate) async fn lookup_owned(
    owner: &AuthOwner,
    config: &Config,
    destination: &Destination,
    deadline: Instant,
    cancelled: &CancellationToken,
) -> Result<Secret, AuthError> {
    if config.executable.as_os_str().is_empty() {
        return Err(AuthError::MissingExecutable);
    }
    if cancelled.is_cancelled() || owner.inner.cancelled.is_cancelled() {
        return Err(AuthError::Cancelled);
    }
    if deadline <= Instant::now() {
        return Err(AuthError::Timeout);
    }
    owner.reap_ready();
    let helper_slot = owner
        .inner
        .helper_slots
        .0
        .clone()
        .try_acquire_owned()
        .map_err(|_| AuthError::Capacity)?;

    let mut command = Command::new(&config.executable);
    // The helper needs no directory, and must not inherit the host's.
    command
        .current_dir("/")
        .args([
            OsStr::new("auth"),
            OsStr::new("git-credential"),
            OsStr::new("get"),
        ])
        .env_clear()
        .envs(config.environment.iter().map(|(key, value)| (key, value)))
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AuthError::MissingExecutable
        } else {
            AuthError::SpawnFailed
        }
    })?;
    owner.inner.active.fetch_add(1, Ordering::AcqRel);
    let _active = ActiveGuard {
        active: owner.inner.active.clone(),
    };
    let mut job = HelperJob::new(child, helper_slot, owner.clone());
    let mut stdin = job.child_mut().stdin.take().ok_or(AuthError::SpawnFailed)?;
    let stdout = job
        .child_mut()
        .stdout
        .take()
        .ok_or(AuthError::SpawnFailed)?;
    let stderr = job
        .child_mut()
        .stderr
        .take()
        .ok_or(AuthError::SpawnFailed)?;
    let request = format!(
        "protocol=https\nhost={}\npath={}\n\n",
        destination.authority(),
        destination.url.path()
    )
    .into_bytes();
    let overflow = CancellationToken::new();
    let write = async move { write_request(stdin, &request).await };
    let read_stdout = read_limited(stdout, overflow.clone());
    let read_stderr = read_limited(stderr, overflow.clone());
    let mut work = Box::pin(async {
        let (write_result, stdout_result, stderr_result) =
            tokio::join!(write, read_stdout, read_stderr);
        let status = job.child_mut().wait().await.map_err(|_| AuthError::Io)?;
        write_result?;
        stderr_result?;
        let mut stdout = stdout_result?;
        let result = if status.success() {
            parse_secret(&stdout)
        } else {
            Err(AuthError::HelperRejected)
        };
        stdout.fill(0);
        result
    });
    let mut timer = Box::pin(sleep_until(deadline));
    tokio::select! {
        result = &mut work => {
            let result = result;
            drop(work);
            job.complete_if_exited();
            result
        },
        _ = cancelled.cancelled() => {
            drop(work);
            match job.terminate().await {
                Ok(()) => Err(AuthError::Cancelled),
                Err(error) => Err(error),
            }
        }
        _ = owner.inner.cancelled.cancelled() => {
            drop(work);
            match job.terminate().await {
                Ok(()) => Err(AuthError::Cancelled),
                Err(error) => Err(error),
            }
        }
        _ = &mut timer => {
            drop(work);
            match job.terminate().await {
                Ok(()) => Err(AuthError::Timeout),
                Err(error) => Err(error),
            }
        }
        _ = overflow.cancelled() => {
            drop(work);
            match job.terminate().await {
                Ok(()) => Err(AuthError::OutputTooLarge),
                Err(error) => Err(error),
            }
        }
    }
}

/// Writes the lookup request to the helper's input and closes it.
///
/// A helper may exit without reading its input. Git ignores SIGPIPE while it
/// writes to a credential helper, so a broken pipe here is not a failure: the
/// helper's exit status and output decide the lookup. Any other write error is.
async fn write_request<W>(mut input: W, request: &[u8]) -> Result<(), AuthError>
where
    W: AsyncWrite + Unpin,
{
    let written = match input.write_all(request).await {
        Ok(()) => input.shutdown().await,
        Err(error) => Err(error),
    };
    match written {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => Err(AuthError::Io),
        _ => Ok(()),
    }
}

async fn read_limited<R>(reader: R, overflow: CancellationToken) -> Result<Vec<u8>, AuthError>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut reader = reader.take((OUTPUT_LIMIT + 1) as u64);
    reader
        .read_to_end(&mut output)
        .await
        .map_err(|_| AuthError::Io)?;
    if output.len() > OUTPUT_LIMIT {
        overflow.cancel();
        return Err(AuthError::OutputTooLarge);
    }
    Ok(output)
}

struct ActiveGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

struct HelperJob {
    child: Option<Child>,
    helper_slot: Option<OwnedSemaphorePermit>,
    owner: AuthOwner,
}

impl HelperJob {
    fn new(child: Child, helper_slot: OwnedSemaphorePermit, owner: AuthOwner) -> Self {
        Self {
            child: Some(child),
            helper_slot: Some(helper_slot),
            owner,
        }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("active helper child")
    }

    fn complete(&mut self) {
        drop(self.child.take());
        drop(self.helper_slot.take());
    }

    fn complete_if_exited(&mut self) {
        if self
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
        {
            self.complete();
        }
    }

    async fn terminate(&mut self) -> Result<(), AuthError> {
        let result = {
            let child = self.child_mut();
            let _ = child.start_kill();
            timeout(CLEANUP_GRACE, child.wait()).await
        };
        match result {
            Ok(Ok(_)) => {
                self.complete();
                Ok(())
            }
            Ok(Err(_)) | Err(_) => Err(AuthError::CleanupPending),
        }
    }
}

impl Drop for HelperJob {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let Some(helper_slot) = self.helper_slot.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            drop(helper_slot);
            return;
        }
        let _ = child.start_kill();
        self.owner
            .retain_pending(PendingChild { child, helper_slot });
    }
}

/// Compatibility shim for callers that have not yet supplied an
/// `AuthOwner`. New endpoint code must call `AuthOwner::reap_pending` so
/// cleanup remains endpoint-scoped.
static ORPHAN_REAPING: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn pending_cleanup_count() -> usize {
    let pending = orphan_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    pending.len() + ORPHAN_REAPING.load(Ordering::Acquire)
}
struct OrphanReapBatch {
    children: Vec<OrphanChild>,
}
impl Drop for OrphanReapBatch {
    fn drop(&mut self) {
        let count = self.children.len();
        let mut pending = orphan_registry().lock().unwrap_or_else(|e| e.into_inner());
        for mut child in self.children.drain(..) {
            let _ = child.pending.child.start_kill();
            pending.push(child);
        }
        ORPHAN_REAPING.fetch_sub(count, Ordering::AcqRel);
    }
}
async fn reap_orphans(deadline: Instant) -> usize {
    let children = {
        let mut pending = orphan_registry().lock().unwrap_or_else(|e| e.into_inner());
        let children = std::mem::take(&mut *pending);
        ORPHAN_REAPING.fetch_add(children.len(), Ordering::AcqRel);
        children
    };
    let mut batch = OrphanReapBatch { children };
    let mut index = 0;
    while index < batch.children.len() && Instant::now() < deadline {
        let result =
            tokio::time::timeout_at(deadline, batch.children[index].pending.child.wait()).await;
        if matches!(result, Ok(Ok(_))) {
            batch.children.swap_remove(index);
            ORPHAN_REAPING.fetch_sub(1, Ordering::AcqRel);
        } else {
            index += 1;
        }
    }
    drop(batch);
    pending_cleanup_count()
}

/// Compatibility shim for the old process-wide cleanup call. Owned lookups
/// never place children in that pool, so there is no cross-endpoint work to
/// perform here.
pub(crate) async fn reap_pending(_deadline: Instant) -> usize {
    reap_orphans(_deadline).await
}

fn parse_secret(output: &[u8]) -> Result<Secret, AuthError> {
    // `gh auth git-credential get` ends on the last attribute line. A blank
    // line is optional; attributes after one are still rejected below.
    if !output.ends_with(b"\n") {
        return Err(AuthError::MalformedOutput);
    }
    let text = std::str::from_utf8(output).map_err(|_| AuthError::MalformedOutput)?;
    let mut username = None;
    let mut password = None;
    let mut ended = false;
    for line in text.split('\n') {
        if line.is_empty() {
            ended = true;
            continue;
        }
        if ended {
            return Err(AuthError::MalformedOutput);
        }
        let (key, value) = line.split_once('=').ok_or(AuthError::MalformedOutput)?;
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(AuthError::MalformedOutput);
        }
        if value.chars().any(char::is_control) {
            return Err(AuthError::MalformedOutput);
        }
        match key {
            "username" if username.is_none() => username = Some(value.as_bytes().to_vec()),
            "password" if password.is_none() => password = Some(value.as_bytes().to_vec()),
            "username" | "password" => return Err(AuthError::MalformedOutput),
            _ => {}
        }
    }
    let username = username
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::MissingCredential)?;
    let password = password
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::MissingCredential)?;
    if username.contains(&b':') {
        return Err(AuthError::MalformedOutput);
    }
    Ok(Secret { username, password })
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use crate::git::endpoint::helper_script::write_helper_script;
            use std::fs;
            use tempfile::tempdir;

            fn helper(script: &str) -> (tempfile::TempDir, Config) {
                let directory = tempdir().unwrap();
                let path = directory.path().join("gh-helper");
                write_helper_script(&path, &format!("{script}\n"));
                (
                    directory,
                    Config {
                        executable: path,
                        environment: Vec::new(),
                    },
                )
            }

            #[test]
            fn secret_is_redacted_and_builds_basic_header() {
                let secret = Secret {
                    username: b"alice".to_vec(),
                    password: b"secret".to_vec(),
                };
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                assert_eq!(format!("{secret:?}"), "Secret(REDACTED)");
            }

            #[test]
            fn gh_attribute_line_without_blank_line_is_a_credential() {
                let secret = parse_secret(b"protocol=https\nhost=github.com\nusername=alice\npassword=secret\n").unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
            }

            #[test]
            fn malformed_or_ambiguous_helper_output_is_rejected() {
                for output in [
                    b"username=alice\npassword=secret".as_slice(),
                    b"username=alice\nusername=bob\npassword=secret\n\n",
                    b"username=al:ice\npassword=secret\n\n",
                    b"username=alice\npassword=\n\n",
                    b"username=alice\npassword=secret\n\nextra=x\n",
                ] {
                    assert!(matches!(parse_secret(output), Err(AuthError::MalformedOutput) | Err(AuthError::MissingCredential)));
                }
            }

            #[tokio::test]
            async fn lookup_accepts_gh_output_without_a_blank_line() {
                let (directory, config) = helper("printf 'protocol=https\\nhost=github.com\\nusername=alice\\npassword=secret\\n'");
                let destination = Destination::parse("https://github.com/owner/repo.git").unwrap();
                let secret = lookup(
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(5),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                drop(directory);
            }

            #[tokio::test]
            async fn lookup_uses_bounded_direct_helper_and_returns_secret() {
                let (directory, config) = helper("printf 'username=alice\\npassword=secret\\n\\n'");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let secret = lookup(
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(5),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
                assert_eq!(secret.header(), "Basic YWxpY2U6c2VjcmV0");
                drop(directory);
            }

            #[tokio::test]
            async fn a_helper_that_exits_without_reading_the_request_does_not_fail_the_write() {
                // Git ignores SIGPIPE while it writes to a credential helper, so a
                // helper may answer without reading its input. The helper's exit
                // status and output decide the lookup, not the closed input.
                let (input, helper) = tokio::io::duplex(64);
                drop(helper);
                assert!(matches!(write_request(input, b"protocol=https\n\n").await, Ok(())));
            }

            #[tokio::test]
            async fn any_other_request_write_failure_is_an_io_failure() {
                let input = FailingInput(io::ErrorKind::PermissionDenied);
                assert!(matches!(write_request(input, b"protocol=https\n\n").await, Err(AuthError::Io)));
            }

            /// Input whose every write fails with one error kind.
            struct FailingInput(io::ErrorKind);

            impl AsyncWrite for FailingInput {
                fn poll_write(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                    _: &[u8],
                ) -> std::task::Poll<io::Result<usize>> {
                    std::task::Poll::Ready(Err(self.0.into()))
                }

                fn poll_flush(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }

                fn poll_shutdown(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
            }

            #[tokio::test]
            async fn pre_cancelled_lookup_does_not_start_helper() {
                let (directory, config) = helper("sleep 5");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let cancelled = CancellationToken::new();
                cancelled.cancel();
                let result = lookup(
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(5),
                    &cancelled,
                )
                .await;
                assert!(matches!(result, Err(AuthError::Cancelled) | Err(AuthError::CleanupPending)));
                drop(directory);
            }

            #[tokio::test]
            async fn aborting_reap_preserves_child_and_permit_ownership() {
                let owner = AuthOwner::new(HelperSlots::new());
                let slots = owner.inner.helper_slots.0.clone();
                let before = slots.available_permits();
                let child = Command::new("/bin/sleep").arg("5").kill_on_drop(true).spawn().unwrap();
                owner.retain_pending(PendingChild { child, helper_slot: slots.clone().try_acquire_owned().unwrap() });
                let reaper = owner.clone();
                let task = tokio::spawn(async move { reaper.reap_pending(Instant::now()+Duration::from_secs(5)).await });
                while !owner.inner.pending.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
                task.abort(); let _ = task.await;
                let retained = owner.pending_cleanup_count();
                let permits = slots.available_permits();
                let _ = owner.reap_pending(Instant::now()+Duration::from_secs(1)).await;
                assert_eq!(retained, 1, "aborting reap lost its owned child");
                assert_eq!(permits, before-1, "permit released before actual reap");
                assert_eq!(owner.pending_cleanup_count(), 0);
            }

            #[tokio::test]
            async fn reap_reports_children_arriving_while_it_waits() {
                let owner = AuthOwner::new(HelperSlots::new());
                let child = Command::new("/bin/sleep").arg("0.1").kill_on_drop(true).spawn().unwrap();
                owner.retain_pending(PendingChild { child, helper_slot: owner.inner.helper_slots.0.clone().try_acquire_owned().unwrap() });
                let reaper = owner.clone();
                let task = tokio::spawn(async move { reaper.reap_pending(Instant::now()+Duration::from_secs(1)).await });
                while !owner.inner.pending.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
                let child = Command::new("/bin/sleep").arg("5").kill_on_drop(true).spawn().unwrap();
                owner.retain_pending(PendingChild { child, helper_slot: owner.inner.helper_slots.0.clone().try_acquire_owned().unwrap() });
                let reported = task.await.unwrap();
                let actual = owner.pending_cleanup_count();
                for pending in owner.inner.pending.lock().unwrap().iter_mut() { let _ = pending.child.start_kill(); }
                let _ = owner.reap_pending(Instant::now()+Duration::from_secs(1)).await;
                assert_eq!(reported, actual, "reap omitted a child transferred during its wait");
                assert_eq!(reported, 1);
            }

            #[tokio::test]
            async fn owner_cancellation_after_start_reclaims_helper_admission() {
                let host = HelperSlots::new();
                let owner = AuthOwner::new(host.clone());
                let (directory, tasks) = occupy_eight_slots(&owner).await;
                assert_eq!(owner.active_count(), 8);
                owner.cancel();
                for task in tasks {
                    let result = task.await.unwrap();
                    assert!(
                        matches!(result, Err(AuthError::Cancelled) | Err(AuthError::CleanupPending)),
                        "unexpected helper result: {result:?}"
                    );
                }
                assert_eq!(owner.active_count(), 0);
                let retained = owner
                    .reap_pending(Instant::now() + Duration::from_secs(2))
                    .await;
                assert_eq!(retained, 0);
                assert_eq!(owner.pending_cleanup_count(), 0);
                // The host's eight slots are back: another of its owners is admitted.
                assert_eq!(
                    quick_lookup(&AuthOwner::new(host)).await,
                    Ok("Basic YWxpY2U6c2VjcmV0".to_owned())
                );
                drop(directory);
            }

            /// Starts eight helpers on `owner` that stay live until cancelled.
            async fn occupy_eight_slots(
                owner: &AuthOwner,
            ) -> (tempfile::TempDir, Vec<tokio::task::JoinHandle<Result<Secret, AuthError>>>) {
                let directory = tempdir().unwrap();
                let started = directory.path().join("started");
                let executable = directory.path().join("busy-helper");
                write_helper_script(
                    &executable,
                    "printf '%s\\n' \"$$\" >> \"$GWZ_HELPER_STARTED\"\nexec /bin/sleep 5\n",
                );
                let config = Config {
                    executable,
                    environment: vec![("GWZ_HELPER_STARTED".into(), started.as_os_str().into())],
                };
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let mut tasks = Vec::new();
                for _ in 0..8 {
                    let owner = owner.clone();
                    let config = config.clone();
                    let destination = destination.clone();
                    tasks.push(tokio::spawn(async move {
                        lookup_owned(
                            &owner,
                            &config,
                            &destination,
                            Instant::now() + Duration::from_secs(5),
                            &CancellationToken::new(),
                        )
                        .await
                    }));
                }
                let barrier = Instant::now() + Duration::from_secs(2);
                while fs::read_to_string(&started).map_or(0, |started| started.lines().count()) < 8 {
                    assert!(Instant::now() < barrier, "helper start barrier timed out");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                (directory, tasks)
            }

            async fn release(owner: &AuthOwner, tasks: Vec<tokio::task::JoinHandle<Result<Secret, AuthError>>>) {
                owner.cancel();
                for task in tasks {
                    let _ = task.await.unwrap();
                }
                assert_eq!(owner.reap_pending(Instant::now() + Duration::from_secs(2)).await, 0);
            }

            async fn quick_lookup(owner: &AuthOwner) -> Result<String, AuthError> {
                let (directory, config) = helper("printf 'username=alice\\npassword=secret\\n\\n'");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let result = lookup_owned(
                    owner,
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(2),
                    &CancellationToken::new(),
                )
                .await
                .map(|secret| secret.header());
                drop(directory);
                result
            }

            #[tokio::test]
            async fn one_hosts_live_helpers_never_exhaust_another_hosts_admission() {
                let busy = AuthOwner::new(HelperSlots::new());
                let (directory, tasks) = occupy_eight_slots(&busy).await;
                let other = quick_lookup(&AuthOwner::new(HelperSlots::new())).await;
                release(&busy, tasks).await;
                assert_eq!(other, Ok("Basic YWxpY2U6c2VjcmV0".to_owned()));
                drop(directory);
            }

            #[tokio::test]
            async fn endpoints_of_one_host_share_its_eight_helper_slots() {
                let host = HelperSlots::new();
                let busy = AuthOwner::new(host.clone());
                let sibling = AuthOwner::new(host);
                let (directory, tasks) = occupy_eight_slots(&busy).await;
                let ninth = quick_lookup(&sibling).await;
                release(&busy, tasks).await;
                let after = quick_lookup(&sibling).await;
                assert_eq!(ninth, Err(AuthError::Capacity), "a ninth live helper on one host");
                assert_eq!(after, Ok("Basic YWxpY2U6c2VjcmV0".to_owned()));
                drop(directory);
            }

            #[tokio::test]
            async fn abort_after_helper_start_retains_permit_until_owner_reap() {
                let directory = tempdir().unwrap();
                let started = directory.path().join("started");
                let executable = directory.path().join("abort-helper");
                write_helper_script(
                    &executable,
                    "printf started > \"$GWZ_HELPER_STARTED\"\nexec /bin/sleep 5\n",
                );
                let config = Config {
                    executable,
                    environment: vec![(
                        "GWZ_HELPER_STARTED".into(),
                        started.as_os_str().into(),
                    )],
                };
                let owner = AuthOwner::new(HelperSlots::new());
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let available_before = owner.inner.helper_slots.0.available_permits();
                assert!(available_before > 0, "helper admission unexpectedly exhausted");
                let task_owner = owner.clone();
                let task_config = config.clone();
                let task_destination = destination.clone();
                let task = tokio::spawn(async move {
                    lookup_owned(
                        &task_owner,
                        &task_config,
                        &task_destination,
                        Instant::now() + Duration::from_secs(5),
                        &CancellationToken::new(),
                    )
                    .await
                });
                let barrier = Instant::now() + Duration::from_secs(2);
                while !started.exists() {
                    assert!(Instant::now() < barrier, "helper start barrier timed out");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert_eq!(owner.active_count(), 0);
                assert_eq!(owner.pending_cleanup_count(), 1);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before - 1);

                assert_eq!(
                    owner.reap_pending(Instant::now()).await,
                    1,
                    "zero-time reap must retain a live aborted child"
                );
                assert_eq!(owner.pending_cleanup_count(), 1);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before - 1);
                assert_eq!(owner.reap_pending(Instant::now() + Duration::from_secs(2)).await, 0);
                assert_eq!(owner.pending_cleanup_count(), 0);
                assert_eq!(owner.inner.helper_slots.0.available_permits(), available_before);
            }

            #[tokio::test]
            async fn oversized_output_is_stopped_at_sixteen_kibibytes() {
                let (directory, config) = helper("head -c 17000 /dev/zero");
                let destination = Destination::parse("https://example.com/owner/repo").unwrap();
                let result = lookup(
                    &config,
                    &destination,
                    Instant::now() + Duration::from_secs(5),
                    &CancellationToken::new(),
                )
                .await;
                assert!(matches!(result, Err(AuthError::OutputTooLarge) | Err(AuthError::CleanupPending)));
                drop(directory);
            }
        }
    }
}
