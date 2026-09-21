//! Stateful per-remote Git adapter. Endpoint placement, URL/identity resolution
//! and the independent message/SSH worker belong to the injected endpoint.
use super::{ssh_channel::GitService, stream_io::BlockingStream};
use git2::{
    Error, ErrorClass, ErrorCode, RemoteCallbacks,
    transport::{Service, SmartSubtransport, SmartSubtransportStream},
};
use std::{
    io,
    sync::{Arc, Mutex},
};

pub(crate) trait OpenStream: Send + Sync + 'static {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream>;
}

/// An explicit native authentication refusal, distinct from local permissions.
#[derive(Debug)]
pub(crate) struct AuthenticationRejected;
impl std::fmt::Display for AuthenticationRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SSH authentication rejected")
    }
}
impl std::error::Error for AuthenticationRejected {}

pub(crate) struct RemoteTransport {
    endpoint: Arc<dyn OpenStream>,
    active: Mutex<Option<BlockingStream>>,
}

impl RemoteTransport {
    pub(crate) fn new(endpoint: Arc<dyn OpenStream>) -> Self {
        Self {
            endpoint,
            active: Mutex::new(None),
        }
    }
}

/// Each factory gets owned per-remote state; the endpoint/pool may be shared
/// across operations. Stateful mode keeps discovery and negotiation on one
/// stream through git2-rs's UploadPackLs/UploadPack and ReceivePack equivalents.
pub(crate) fn callbacks(endpoint: Arc<dyn OpenStream>) -> RemoteCallbacks<'static> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.smart_transport(false, move |_remote| {
        Ok(RemoteTransport::new(endpoint.clone()))
    });
    callbacks
}

fn network_error(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::GenericError,
        ErrorClass::Net,
        error.to_string().replace('\0', "\\0"),
    )
}

impl SmartSubtransport for RemoteTransport {
    fn action(
        &self,
        url: &str,
        service: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, Error> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| network_error("remote stream lock poisoned"))?;
        if active.is_some() {
            return Err(network_error(
                "remote stream must close before another service opens",
            ));
        }
        let service = match service {
            Service::UploadPackLs | Service::UploadPack => GitService::UploadPack,
            Service::ReceivePackLs | Service::ReceivePack => GitService::ReceivePack,
        };
        let stream = self.endpoint.open(url, service).map_err(|error| {
            if error
                .get_ref()
                .is_some_and(|cause| cause.is::<AuthenticationRejected>())
            {
                Error::new(ErrorCode::Auth, ErrorClass::Ssh, error.to_string())
            } else {
                network_error(error)
            }
        })?;
        *active = Some(stream.clone());
        Ok(Box::new(stream))
    }

    fn close(&self) -> Result<(), Error> {
        let stream = self
            .active
            .lock()
            .map_err(|_| network_error("remote stream lock poisoned"))?
            .take();
        if let Some(stream) = stream {
            // close orders EndWrite, drains/discards responses, then waits for
            // actual endpoint cleanup. Only the endpoint can return a pool lease.
            stream.close().map_err(network_error)?;
        }
        Ok(())
    }
}

impl Drop for RemoteTransport {
    fn drop(&mut self) {
        let active = self
            .active
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(stream) = active.take() {
            // Destructors never block the Git thread or claim successful cleanup.
            stream.cancel();
        }
    }
}
