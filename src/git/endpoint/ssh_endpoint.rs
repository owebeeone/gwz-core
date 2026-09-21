//! Per-remote binding to one shared endpoint. Credential selection is endpoint
//! policy and must revalidate explicit authority before every allocation.
use super::{
    ssh_channel::GitService, ssh_destination::Destination, ssh_remote::OpenStream,
    ssh_worker::Endpoint, stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{Facts, Opened},
};
use std::{io, path::PathBuf, sync::Arc};

pub(crate) trait IdentityResolver: Send + Sync + 'static {
    /// Return an eligible identity only after validating its current authority.
    /// An explicit proof must agree with the Connector's authentication result;
    /// private bytes and paths never belong in that proof or diagnostics.
    fn resolve(&self, key: &Key) -> io::Result<Identity>;
}
pub(crate) struct Route {
    endpoint: Endpoint,
    authority: Authority,
    observe: Arc<dyn Fn(&Opened) + Send + Sync>,
    report: Option<Arc<dyn Fn(&Facts) + Send + Sync>>,
}
enum Authority {
    Resolved(Arc<dyn IdentityResolver>),
    Ambient,
    Selected(PathBuf),
}
impl Route {
    /// Selection is immutable per operation. File admission belongs to the
    /// supervised worker and runs before every checkout, including reuse.
    pub(crate) fn local(
        endpoint: Endpoint,
        selected: Option<PathBuf>,
        observe: Arc<dyn Fn(&Opened) + Send + Sync>,
    ) -> Self {
        Self {
            endpoint,
            authority: selected.map_or(Authority::Ambient, Authority::Selected),
            observe,
            report: None,
        }
    }
    pub(crate) fn reporting(
        endpoint: Endpoint,
        selected: Option<PathBuf>,
        report: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> Self {
        let mut route = Self::local(endpoint, selected, Arc::new(|_| {}));
        route.report = Some(report);
        route
    }
    pub(crate) fn new(endpoint: Endpoint, identities: Arc<dyn IdentityResolver>) -> Self {
        Self::observed(endpoint, identities, Arc::new(|_| {}))
    }
    pub(crate) fn observed(
        endpoint: Endpoint,
        identities: Arc<dyn IdentityResolver>,
        observe: Arc<dyn Fn(&Opened) + Send + Sync>,
    ) -> Self {
        Self {
            endpoint,
            authority: Authority::Resolved(identities),
            observe,
            report: None,
        }
    }
}
impl OpenStream for Route {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream> {
        let progress = super::ssh_pool::Progress::default();
        let result = self.open_inner(url, service, progress.clone());
        let facts = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(report) = &self.report {
            report(&facts);
        }
        let exhausted = result
            .as_ref()
            .err()
            .and_then(|error| error.get_ref())
            .and_then(|cause| cause.downcast_ref::<gwz_transport::pool::Error>())
            .is_some_and(|cause| {
                matches!(
                    cause,
                    gwz_transport::pool::Error::ConnectFailed {
                        code: gwz_transport::protocol::ErrorCode::Authentication,
                        ..
                    }
                )
            });
        if exhausted && facts.authenticated == Some(false) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                super::ssh_remote::AuthenticationRejected,
            ));
        }
        result
    }
}
impl Route {
    fn open_inner(
        &self,
        url: &str,
        service: GitService,
        progress: super::ssh_pool::Progress,
    ) -> io::Result<BlockingStream> {
        let destination = Destination::parse(url)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SSH endpoint requires an SSH destination",
            )
        })?;
        let (stream, facts) = match &self.authority {
            Authority::Selected(path) => self.endpoint.open_reported(
                destination.key,
                Some(path.clone()),
                service,
                &destination.path,
                progress,
            )?,
            Authority::Ambient => self.endpoint.open_reported(
                destination.key,
                None,
                service,
                &destination.path,
                progress,
            )?,
            authority => {
                let identity = match authority {
                    Authority::Resolved(resolver) => resolver.resolve(&destination.key)?,
                    _ => Identity::Ambient,
                };
                self.endpoint.open_observed(
                    destination.key,
                    identity,
                    service,
                    &destination.path,
                )?
            }
        };
        (self.observe)(&facts);
        Ok(stream)
    }
}
