//! Bounded regular-file reads retain their lookup admissions through completion.
use super::*;
use std::{
    io::Read,
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::Path,
};
use tokio::task::JoinHandle;

pub(super) struct PendingWorker {
    handle: JoinHandle<Result<SecretBuffer, AuthError>>,
    _permits: Arc<owner::AdmissionPermits>,
}
struct Worker<'a> {
    pending: Option<PendingWorker>,
    owner: &'a AuthOwner,
}
impl Drop for Worker<'_> {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.owner
                .inner
                .workers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(pending);
        }
    }
}
pub(super) fn reap_ready(owner: &AuthOwner) {
    // Finished blocking workers own no open file. Dropping their completed
    // join handle disposes the zeroizing result without blocking this thread.
    owner
        .inner
        .workers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|pending| !pending.handle.is_finished());
}

pub(super) async fn read(
    runner: &runner::Runner<'_>,
    path: SecretBuffer,
) -> Result<SecretBuffer, AuthError> {
    runner.check()?;
    let cancelled = runner.cancelled.clone();
    let owner_cancel = runner.owner.inner.cancelled.clone();
    let deadline = runner.deadline;
    let permits = runner.permits.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let _permits = permits;
        let check = || {
            if cancelled.is_cancelled() || owner_cancel.is_cancelled() {
                return Err(AuthError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(AuthError::Timeout);
            }
            Ok(())
        };
        check()?;
        let path = Path::new(OsStr::from_bytes(&path.0));
        if !path.is_absolute() {
            return Err(AuthError::ConfigurationRefused);
        }
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(SecretBuffer(Vec::new())),
            Err(_) => return Err(AuthError::ConfigurationRefused),
        };
        if !file
            .metadata()
            .map_err(|_| AuthError::ConfigurationRefused)?
            .is_file()
        {
            return Err(AuthError::ConfigurationRefused);
        }
        let mut bytes = SecretBuffer(vec![0; super::view::SOURCE_LIMIT + 1]);
        let mut used = 0;
        loop {
            check()?;
            let n = file
                .read(&mut bytes.0[used..])
                .map_err(|_| AuthError::ConfigurationRefused)?;
            check()?;
            if n == 0 {
                bytes.0.truncate(used);
                return Ok(bytes);
            }
            used += n;
            if used > super::view::SOURCE_LIMIT {
                return Err(AuthError::ConfigurationRefused);
            }
        }
    });
    let mut worker = Worker {
        pending: Some(PendingWorker {
            handle,
            _permits: runner.permits.clone(),
        }),
        owner: runner.owner,
    };
    let result = tokio::select! {
        result = &mut worker.pending.as_mut().unwrap().handle => Some(result),
        _ = runner.cancelled.cancelled() => None,
        _ = runner.owner.inner.cancelled.cancelled() => None,
        _ = sleep_until(runner.deadline) => None,
    };
    if let Some(result) = result {
        worker.pending = None;
        return result.map_err(|_| AuthError::ConfigurationRefused)?;
    }
    // A filesystem operation may be uninterruptible. It owns both permits and
    // all buffers until finished, even if this future is dropped during grace.
    if timeout(CLEANUP_GRACE, &mut worker.pending.as_mut().unwrap().handle)
        .await
        .is_ok()
    {
        worker.pending = None;
        runner.check()?;
    }
    Err(AuthError::CleanupPending)
}
