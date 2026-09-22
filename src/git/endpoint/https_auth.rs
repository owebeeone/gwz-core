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
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    time::{Instant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 16 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_millis(500);

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
    if config.executable.as_os_str().is_empty() {
        return Err(AuthError::MissingExecutable);
    }
    if cancelled.is_cancelled() {
        return Err(AuthError::Cancelled);
    }
    if deadline <= Instant::now() {
        return Err(AuthError::Timeout);
    }
    reap_ready();
    let helper_slot = helper_slots()
        .try_acquire_owned()
        .map_err(|_| AuthError::Capacity)?;

    let mut command = Command::new(&config.executable);
    command
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
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AuthError::MissingExecutable
        } else {
            AuthError::SpawnFailed
        }
    })?;
    let mut stdin = child.stdin.take().ok_or(AuthError::SpawnFailed)?;
    let stdout = child.stdout.take().ok_or(AuthError::SpawnFailed)?;
    let stderr = child.stderr.take().ok_or(AuthError::SpawnFailed)?;
    let request = format!(
        "protocol=https\nhost={}\npath={}\n\n",
        destination.authority(),
        destination.url.path()
    )
    .into_bytes();
    let overflow = CancellationToken::new();
    let write = async move {
        stdin.write_all(&request).await.map_err(|_| AuthError::Io)?;
        stdin.shutdown().await.map_err(|_| AuthError::Io)
    };
    let read_stdout = read_limited(stdout, overflow.clone());
    let read_stderr = read_limited(stderr, overflow.clone());
    let mut work = Box::pin(async {
        let (write_result, stdout_result, stderr_result) =
            tokio::join!(write, read_stdout, read_stderr);
        let status = child.wait().await.map_err(|_| AuthError::Io)?;
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
        result = &mut work => result,
        _ = cancelled.cancelled() => {
            drop(work);
            match terminate(child, helper_slot).await {
                Ok(()) => Err(AuthError::Cancelled),
                Err(error) => Err(error),
            }
        }
        _ = &mut timer => {
            drop(work);
            match terminate(child, helper_slot).await {
                Ok(()) => Err(AuthError::Timeout),
                Err(error) => Err(error),
            }
        }
        _ = overflow.cancelled() => {
            drop(work);
            match terminate(child, helper_slot).await {
                Ok(()) => Err(AuthError::OutputTooLarge),
                Err(error) => Err(error),
            }
        }
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

async fn terminate(child: Child, helper_slot: OwnedSemaphorePermit) -> Result<(), AuthError> {
    let mut child = child;
    let _ = child.start_kill();
    match timeout(CLEANUP_GRACE, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) | Err(_) => {
            pending_registry()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((child, helper_slot));
            Err(AuthError::CleanupPending)
        }
    }
}

fn pending_registry() -> &'static Mutex<Vec<(Child, OwnedSemaphorePermit)>> {
    static PENDING: OnceLock<Mutex<Vec<(Child, OwnedSemaphorePermit)>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

fn helper_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(8))).clone()
}

fn reap_ready() {
    pending_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain_mut(|(child, _)| !matches!(child.try_wait(), Ok(Some(_))));
}
/// Retained children keep endpoint admission reserved until they are joined.
pub(crate) fn pending_cleanup_count() -> usize {
    pending_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .len()
}

/// Join retained children by the endpoint cleanup deadline. Children that do
/// not finish remain owned by the registry and are counted on the next pass.
pub(crate) async fn reap_pending(deadline: Instant) -> usize {
    let children = std::mem::take(
        &mut *pending_registry()
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    );
    let mut pending = Vec::new();
    for (mut child, helper_slot) in children {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            pending.push((child, helper_slot));
            continue;
        }
        match timeout(remaining, child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => pending.push((child, helper_slot)),
        }
    }
    let count = pending.len();
    pending_registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .extend(pending);
    count
}

fn parse_secret(output: &[u8]) -> Result<Secret, AuthError> {
    if !output.ends_with(b"\n\n") {
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
            use std::{
                fs,
                os::unix::fs::PermissionsExt,
            };
            use tempfile::tempdir;

            fn helper(script: &str) -> (tempfile::TempDir, Config) {
                let directory = tempdir().unwrap();
                let path = directory.path().join("gh-helper");
                fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
                let mut permissions = fs::metadata(&path).unwrap().permissions();
                permissions.set_mode(0o700);
                fs::set_permissions(&path, permissions).unwrap();
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
            fn malformed_or_ambiguous_helper_output_is_rejected() {
                for output in [
                    b"username=alice\npassword=secret\n".as_slice(),
                    b"username=alice\nusername=bob\npassword=secret\n\n",
                    b"username=al:ice\npassword=secret\n\n",
                    b"username=alice\npassword=\n\n",
                    b"username=alice\npassword=secret\n\nextra=x\n",
                ] {
                    assert!(matches!(parse_secret(output), Err(AuthError::MalformedOutput) | Err(AuthError::MissingCredential)));
                }
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
            async fn cancellation_kills_and_reaps_hanging_helper() {
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
