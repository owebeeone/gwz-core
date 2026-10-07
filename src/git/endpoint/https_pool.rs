//! HTTPS physical owner using the existing generic pool, driven independently of Git.
use super::{
    https_connection::{self, Config, Connection, HttpConnector, HttpResource},
    setup_retry::{self, Phase},
    shared_reservation::{Authority, ReservedConnector},
    ssh_pool::{PoolHost, Resource},
};
use gwz_transport::{
    pool::{self, Identity, Key, Lease, Owner, Request},
    protocol::{Disposition, ErrorCode, Failure, SetupFailureCause},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Waker},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
type Host = PoolHost<ReservedConnector<HttpConnector>>;
#[derive(Clone)]
pub(crate) struct HttpsPool {
    pub(crate) pool: pool::Pool,
    host: Arc<Mutex<Host>>,
    epoch: Instant,
}
pub(crate) struct RunningPool {
    pub(crate) client: HttpsPool,
    supervisor: Option<JoinHandle<()>>,
    failed_shutdown: bool,
}
impl RunningPool {
    pub(crate) fn with_authority(
        config: pool::Config,
        tls: Config,
        authority: Authority,
    ) -> Result<Self, Failure> {
        tls.validate()?;
        let supervisor = authority.supervisor().clone();
        Self::with_connector(config, authority, move |epoch| HttpConnector {
            config: tls,
            epoch,
            setup_slots: Arc::new(Semaphore::new(8)),
            supervisor,
        })
    }
    /// The pool over the connector `connector` makes, given the pool's epoch,
    /// which the connector's deadlines are measured from.
    pub(crate) fn with_connector(
        config: pool::Config,
        authority: Authority,
        connector: impl FnOnce(Instant) -> HttpConnector,
    ) -> Result<Self, Failure> {
        let epoch = Instant::now();
        let connector = connector(epoch);
        let (pool, host) = PoolHost::new(config, ReservedConnector::new(connector, authority), 0)
            .map_err(|_| https_connection::failure(ErrorCode::InvalidRequest))?;
        let client = HttpsPool {
            pool,
            host: Arc::new(Mutex::new(host)),
            epoch,
        };
        let owner = client.clone();
        let supervisor = tokio::spawn(async move {
            loop {
                {
                    let mut host = owner.host.lock().unwrap_or_else(|e| e.into_inner());
                    let mut cx = Context::from_waker(Waker::noop());
                    if host.step(&mut cx, owner.now()).is_err() {
                        owner.pool.shutdown();
                    }
                    if host.shutdown_complete() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
        Ok(Self {
            client,
            supervisor: Some(supervisor),
            failed_shutdown: false,
        })
    }
    pub(crate) async fn shutdown(&mut self, timeout: Duration) -> usize {
        self.client.pool.shutdown();
        if let Some(task) = self.supervisor.as_mut() {
            match tokio::time::timeout(timeout, task).await {
                Ok(Ok(())) => {
                    self.supervisor = None;
                }
                Ok(Err(_)) => {
                    self.supervisor = None;
                    self.failed_shutdown = true;
                }
                Err(_) => {}
            }
        }
        self.client.pending().max(usize::from(self.failed_shutdown))
    }
}
impl Drop for RunningPool {
    fn drop(&mut self) {
        self.client.pool.shutdown();
    } // supervisor retains physical owners until retirement
}
impl HttpsPool {
    pub(crate) fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    pub(crate) fn pending(&self) -> usize {
        self.host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .physical_count()
    }
    cfg_if::cfg_if! {
        if #[cfg(test)] {
    /// Leases a connection for `key`. A failure carries its phase: only the
    /// pool's own connect failures end a setup (`setup_retry::phase_of`).
    pub(crate) async fn checkout(
        &self,
        key: Key,
        owner: Owner,
        allocation_ms: u64,
        connect_ms: u64,
        cancel: &CancellationToken,
    ) -> Result<HttpLease, (Failure, Phase)> {
        self.checkout_scoped(key, owner, allocation_ms, connect_ms, cancel, None, false)
            .await
    }
        }
    }
    /// Leases a connection for `key`, a new one when `fresh`. A reused
    /// connection found dead before any byte is released, and the checkout is
    /// made once more, fresh (dev-docs/GwzTransportIdleLossDesign.md §6.1).
    #[allow(clippy::too_many_arguments)] // One pool request's fields, and its cancellation.
    pub(crate) async fn checkout_scoped(
        &self,
        key: Key,
        owner: Owner,
        allocation_ms: u64,
        connect_ms: u64,
        cancel: &CancellationToken,
        scope: Option<&str>,
        fresh: bool,
    ) -> Result<HttpLease, (Failure, Phase)> {
        let started = Instant::now();
        let identity = scope.map_or(Identity::Https, |scope| Identity::HttpsScoped(scope.into()));
        let mut request = Request::new(key, identity, owner);
        request.connect_timeout_ms = Some(connect_ms);
        request.allocation_timeout_ms = Some(allocation_ms);
        request.fresh = fresh;
        let lease = self.lease(request.clone(), cancel).await?;
        match self.adopt(lease, started)? {
            Adopted::Lease(lease) => Ok(lease),
            Adopted::Dead => {
                // A fresh request never gets a reused connection, so this
                // second lease cannot be Dead.
                request.fresh = true;
                let remaining = allocation_ms.saturating_sub(started.elapsed().as_millis() as u64);
                request.allocation_timeout_ms = Some(remaining.max(1));
                let lease = self.lease(request, cancel).await?;
                match self.adopt(lease, started)? {
                    Adopted::Lease(lease) => Ok(lease),
                    Adopted::Dead => Err((https_connection::failure(ErrorCode::Io), Phase::Other)),
                }
            }
        }
    }
    async fn lease(
        &self,
        request: Request,
        cancel: &CancellationToken,
    ) -> Result<Lease, (Failure, Phase)> {
        let other = |failure| (failure, Phase::Other);
        let checkout = {
            let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
            let now = self.now();
            let mut cx = Context::from_waker(Waker::noop());
            host.step(&mut cx, now)
                .map_err(pool_failure)
                .map_err(other)?;
            let checkout = self
                .pool
                .checkout(request)
                .map_err(pool_failure)
                .map_err(other)?;
            // Both admission and immediate dispatch see one current clock.
            // A stale supervisor tick must not expire a fresh short allowance.
            host.step(&mut cx, now)
                .map_err(pool_failure)
                .map_err(other)?;
            checkout
        };
        tokio::select! {
            result = checkout => result.map_err(|error| {
                let phase = setup_retry::phase_of(&error);
                (pool_failure(error), phase)
            }),
            _ = cancel.cancelled() => Err(other(https_connection::failure(ErrorCode::Cancelled))),
        }
    }
    /// Takes `lease`'s connection for one exchange, unless it was reused and
    /// is already dead: then it is released and the caller asks again, fresh.
    pub(crate) fn adopt(
        &self,
        lease: Lease,
        started: Instant,
    ) -> Result<Adopted, (Failure, Phase)> {
        let other = |failure| (failure, Phase::Other);
        let (
            connection,
            reusable,
            resource_cancel,
            disposed,
            connect_elapsed,
            allocation_elapsed,
            reused,
        ) = {
            let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(reused) = host.lost(&lease) {
                host.release(lease, Disposition::Discarded)
                    .map_err(pool_failure)
                    .map_err(other)?;
                return if reused {
                    Ok(Adopted::Dead)
                } else {
                    Err(other(https_connection::failure(ErrorCode::Io)))
                };
            }
            let reused = host
                .allocation_reused(&lease)
                .map_err(pool_failure)
                .map_err(other)?;
            let resource: &mut HttpResource = host
                .resource(&lease)
                .map_err(pool_failure)
                .map_err(other)?
                .inner_mut()
                .ok_or_else(|| other(https_connection::failure(ErrorCode::Io)))?;
            if !resource.reusable() {
                host.release(lease, Disposition::Discarded)
                    .map_err(pool_failure)
                    .map_err(other)?;
                return if reused {
                    Ok(Adopted::Dead)
                } else {
                    Err(other(https_connection::failure(ErrorCode::Io)))
                };
            }
            resource.reusable.store(false, Ordering::Release);
            (
                resource
                    .connection
                    .clone()
                    .ok_or_else(|| other(https_connection::failure(ErrorCode::Io)))?,
                resource.reusable.clone(),
                resource.cancel.clone(),
                resource.disposed.clone(),
                if reused {
                    Duration::ZERO
                } else {
                    resource.connect_elapsed
                },
                Duration::from_millis(if reused {
                    started.elapsed().as_millis()
                } else {
                    resource
                        .connect_started
                        .saturating_duration_since(started)
                        .as_millis()
                } as u64),
                reused,
            )
        };
        let id = format!(
            "https-{:?}",
            lease.connection().map_err(pool_failure).map_err(other)?
        );
        Ok(Adopted::Lease(HttpLease {
            lease: Some(lease),
            connection: Some(connection),
            host: self.host.clone(),
            reusable,
            cancel: resource_cancel,
            disposed,
            connect_elapsed,
            allocation_elapsed,
            id,
            reused,
        }))
    }
}
/// What a lease turned out to be (`HttpsPool::adopt`).
pub(crate) enum Adopted {
    Lease(HttpLease),
    /// A reused connection found dead before any byte; already released.
    Dead,
}
pub(crate) struct HttpLease {
    lease: Option<Lease>,
    pub(crate) connection: Option<Arc<AsyncMutex<Connection>>>,
    host: Arc<Mutex<Host>>,
    reusable: Arc<AtomicBool>,
    pub(crate) cancel: CancellationToken,
    pub(crate) disposed: Arc<AtomicBool>,
    pub(crate) connect_elapsed: Duration,
    pub(crate) allocation_elapsed: Duration,
    pub(crate) id: String,
    pub(crate) reused: bool,
}
impl HttpLease {
    pub(crate) fn scope(&self, scope: &str) -> Result<(), Failure> {
        self.lease
            .as_ref()
            .ok_or_else(|| https_connection::failure(ErrorCode::Protocol))?
            .scope_https(scope)
            .map_err(pool_failure)
    }
    pub(crate) fn finish(mut self, disposition: Disposition) -> Result<(), Failure> {
        self.connection = None;
        self.reusable
            .store(disposition == Disposition::Reusable, Ordering::Release);
        let lease = self
            .lease
            .take()
            .ok_or_else(|| https_connection::failure(ErrorCode::Protocol))?;
        self.host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(lease, disposition)
            .map_err(pool_failure)
    }
}
fn pool_failure(error: pool::Error) -> Failure {
    use pool::Error;
    let (code, setup_cause) = match error {
        // The pool's own limit, or a wait on a local budget that outlasted the
        // open's allocation: this host's failure, with no cause of a server's.
        Error::Capacity | Error::LocalWaitExpired => (ErrorCode::Capacity, None),
        Error::AllocationTimeout => (ErrorCode::Timeout, Some(SetupFailureCause::Allocation)),
        Error::ConnectTimeout => (ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)),
        Error::InteractionTimeout => (ErrorCode::Timeout, Some(SetupFailureCause::Interaction)),
        Error::Cancelled | Error::Shutdown => (ErrorCode::Cancelled, None),
        Error::ConnectFailed {
            code, setup_cause, ..
        } => (code, setup_cause),
        _ => (ErrorCode::Io, None),
    };
    Failure {
        setup_cause,
        ..https_connection::failure(code)
    }
}
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        mod idle_budget_tests;
        mod idle_tests;
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod setup_cause_tests {
            use super::*;

            #[test]
            fn pool_failure_keeps_timeout_and_connector_origins() {
                for (error, cause) in [
                    (pool::Error::AllocationTimeout, SetupFailureCause::Allocation),
                    (pool::Error::ConnectTimeout, SetupFailureCause::Aggregate),
                    (pool::Error::InteractionTimeout, SetupFailureCause::Interaction),
                ] {
                    let failure = pool_failure(error);
                    assert_eq!(failure.code, ErrorCode::Timeout);
                    assert_eq!(failure.setup_cause, Some(cause));
                }
                let failure = pool_failure(pool::Error::ConnectFailed {
                    code: ErrorCode::Unavailable,
                    effect: gwz_transport::protocol::Effect::None,
                    setup_cause: Some(SetupFailureCause::ConnectionRefused),
                });
                assert_eq!(failure.code, ErrorCode::Unavailable);
                assert_eq!(failure.setup_cause, Some(SetupFailureCause::ConnectionRefused));
            }
        }
    }
}
