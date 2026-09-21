//! Bounded worker termination with retained physical-pool cleanup.
use super::{
    agent_job::Cleanup,
    ssh_admission::Admissions,
    ssh_pool::{Connector, PoolHost},
};
use gwz_transport::pool::Pool;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    task::{Context, Waker},
    time::Instant,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShutdownStatus {
    pub cleanup_complete: bool,
    pub pending_connections: usize,
    pub pending_admissions: usize,
    pub failure: Option<io::ErrorKind>,
}
pub(crate) type Status = Arc<Mutex<ShutdownStatus>>;
#[derive(Clone)]
pub(crate) struct ShutdownWatch(pub(crate) Status);
impl ShutdownWatch {
    pub(crate) fn status(&self) -> ShutdownStatus {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}
pub(crate) fn fail(status: &Status, error: io::ErrorKind) {
    status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .failure
        .get_or_insert(error);
}
pub(crate) fn sample<C: Connector>(status: &Status, host: &PoolHost<C>, admissions: &Admissions) {
    status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending_connections = host.physical_count();
    status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending_admissions = admissions.len();
}
pub(crate) fn manage<C>(
    mut host: PoolHost<C>,
    mut admissions: Admissions,
    pool: Pool,
    status: Status,
    retention: Cleanup,
    origin: Instant,
    run: impl FnOnce(&mut PoolHost<C>, &mut Admissions),
) where
    C: Connector + Send + 'static,
    C::Resource: Send + 'static,
{
    if catch_unwind(AssertUnwindSafe(|| run(&mut host, &mut admissions))).is_err() {
        fail(&status, io::ErrorKind::Other);
    }
    pool.shutdown();
    let mut cx = Context::from_waker(Waker::noop());
    let _ = admissions.poll(&mut cx, origin.elapsed().as_millis() as u64, true);
    sample(&status, &host, &admissions);
    if host.shutdown_complete() && admissions.is_empty() {
        drop(host);
        status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cleanup_complete = true;
        return;
    }
    fail(&status, io::ErrorKind::TimedOut);
    retention.retain(move || {
        let mut cx = Context::from_waker(Waker::noop());
        if host
            .step(&mut cx, origin.elapsed().as_millis() as u64)
            .is_err()
        {
            fail(&status, io::ErrorKind::Other);
        }
        let _ = admissions.poll(&mut cx, origin.elapsed().as_millis() as u64, true);
        if let Some(error) = admissions.take_failure() {
            fail(&status, error);
        }
        if let Some(error) = host.take_disposal_error() {
            fail(&status, error.kind());
        }
        sample(&status, &host, &admissions);
        if host.shutdown_complete() && admissions.is_empty() {
            status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cleanup_complete = true;
            true
        } else {
            false
        }
    });
}
