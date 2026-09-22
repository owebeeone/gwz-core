//! HTTPS physical owner using the existing generic pool, driven independently of Git.
use super::{
    https_connection::{self, Config, Connection, HttpConnector, HttpResource},
    shared_reservation::{Authority, ReservedConnector},
    ssh_pool::{PoolHost, Resource},
};
use gwz_transport::{
    pool::{self, Identity, Key, Lease, Owner, Request},
    protocol::{Disposition, ErrorCode, Failure},
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
    pub(crate) fn new(config: pool::Config, tls: Config) -> Result<Self, Failure> {
        let authority = Authority::new(config.total, config.per_host);
        Self::with_authority(config, tls, authority)
    }
    pub(crate) fn with_authority(
        config: pool::Config,
        tls: Config,
        authority: Authority,
    ) -> Result<Self, Failure> {
        tls.validate()?;
        let epoch = Instant::now();
        let connector = HttpConnector {
            config: tls,
            epoch,
            setup_slots: Arc::new(Semaphore::new(8)),
        };
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
    pub(crate) async fn checkout(
        &self,
        key: Key,
        owner: Owner,
        until: u64,
        connect_ms: u64,
        cancel: &CancellationToken,
    ) -> Result<HttpLease, Failure> {
        let mut request = Request::new(key, Identity::Https, owner);
        request.connect_timeout_ms = Some(connect_ms);
        let checkout = self
            .pool
            .checkout_until(request, Some(until))
            .map_err(pool_failure)?;
        let lease = tokio::select! {result=checkout=>result.map_err(pool_failure)?,_=cancel.cancelled()=>return Err(https_connection::failure(ErrorCode::Cancelled))};
        let (connection, reusable, resource_cancel, disposed, connect_elapsed, reused) = {
            let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
            let reused = host.allocation_reused(&lease).map_err(pool_failure)?;
            let resource: &mut HttpResource =
                host.resource(&lease).map_err(pool_failure)?.inner_mut();
            if !resource.reusable() {
                return Err(https_connection::failure(ErrorCode::Io));
            }
            resource.reusable.store(false, Ordering::Release);
            (
                resource
                    .connection
                    .clone()
                    .ok_or_else(|| https_connection::failure(ErrorCode::Io))?,
                resource.reusable.clone(),
                resource.cancel.clone(),
                resource.disposed.clone(),
                if reused {
                    Duration::ZERO
                } else {
                    resource.connect_elapsed
                },
                reused,
            )
        };
        let id = format!("https-{:?}", lease.connection().map_err(pool_failure)?);
        Ok(HttpLease {
            lease: Some(lease),
            connection: Some(connection),
            host: self.host.clone(),
            reusable,
            cancel: resource_cancel,
            disposed,
            connect_elapsed,
            id,
            reused,
        })
    }
}
pub(crate) struct HttpLease {
    lease: Option<Lease>,
    pub(crate) connection: Option<Arc<AsyncMutex<Connection>>>,
    host: Arc<Mutex<Host>>,
    reusable: Arc<AtomicBool>,
    pub(crate) cancel: CancellationToken,
    pub(crate) disposed: Arc<AtomicBool>,
    pub(crate) connect_elapsed: Duration,
    pub(crate) id: String,
    pub(crate) reused: bool,
}
impl HttpLease {
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
    https_connection::failure(match error {
        Error::Capacity => ErrorCode::Capacity,
        Error::AllocationTimeout | Error::ConnectTimeout | Error::InteractionTimeout => {
            ErrorCode::Timeout
        }
        Error::Cancelled => ErrorCode::Cancelled,
        Error::ConnectFailed { code, .. } => code,
        _ => ErrorCode::Io,
    })
}
