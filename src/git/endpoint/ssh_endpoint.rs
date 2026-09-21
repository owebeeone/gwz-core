//! Per-remote binding to one shared endpoint. Credential selection is endpoint
//! policy and must revalidate explicit authority before every allocation.
use super::{
    ssh_channel::GitService, ssh_destination::Destination, ssh_remote::OpenStream,
    ssh_worker::Endpoint, stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::Opened,
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
        }
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
        }
    }
}
impl OpenStream for Route {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream> {
        let destination = Destination::parse(url)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SSH endpoint requires an SSH destination",
            )
        })?;
        let (stream, facts) = match &self.authority {
            Authority::Selected(path) => self.endpoint.open_selected(
                destination.key,
                path.clone(),
                service,
                &destination.path,
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
