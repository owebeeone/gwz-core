//! Candidate binding is isolated until source/platform qualification permits
//! activation. Default builds retain the native transport and dependency graph.
use super::{
    backend::Git2Backend, transport_observations::TransportAttempt,
    transport_support::identity::SelectedIdentity,
};

cfg_if::cfg_if! {
    if #[cfg(all(unix, gwz_transport_candidate))] {
        use crate::git::endpoint::{ssh_destination::Destination, ssh_endpoint::Route, ssh_local, ssh_remote::RemoteTransport, ssh_worker::Endpoint};
        use std::{io, sync::{Arc, OnceLock}};

        #[derive(Clone, Default)]
        pub(crate) struct Runtime(Arc<OnceLock<Result<Endpoint, io::ErrorKind>>>);
        impl Runtime {
            fn endpoint(&self) -> Result<Endpoint, git2::Error> {
                self.0.get_or_init(|| {
                    let home = std::env::var_os("HOME").ok_or(io::ErrorKind::NotFound)?;
                    let known = std::path::PathBuf::from(home).join(".ssh/known_hosts");
                    let agent = std::env::var_os("SSH_AUTH_SOCK").filter(|p| !p.is_empty()).map(Into::into);
                    let timeout = super::transport_support::server_timeout_ms();
                    let config = gwz_transport::pool::Config { connect_timeout_ms: timeout, ..Default::default() };
                    ssh_local::connect(config, known, agent, timeout).map_err(|e| e.kind())
                }).clone().map_err(|_| git2::Error::new(git2::ErrorCode::GenericError, git2::ErrorClass::Net, "SSH endpoint unavailable"))
            }
            cfg_if::cfg_if! {
                if #[cfg(test)] {
                    pub(crate) fn from_endpoint(endpoint: Endpoint) -> Self {
                        Self(Arc::new(OnceLock::from(Ok(endpoint))))
                    }
                }
            }
        }
        pub(crate) fn configure(
            backend: &Git2Backend, url: &str, identity: Option<&SelectedIdentity>,
            attempt: Option<&TransportAttempt>, callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {
            if matches!(Destination::parse(url), Ok(None)) { return; }
            let runtime = backend.ssh.clone();
            let selected = identity.map(|i| i.path.clone());
            let attempt = attempt.cloned();
            callbacks.smart_transport(false, move |_| {
                let attempt = attempt.clone();
                let route = Route::reporting(runtime.endpoint()?, selected.clone(), Arc::new(move |facts| {
                    if let Some(attempt) = &attempt { attempt.facts(facts); }
                }));
                Ok(RemoteTransport::new(Arc::new(route)))
            });
        }
    } else {
        #[derive(Clone, Default)]
        pub(crate) struct Runtime;
        pub(crate) fn configure(
            _backend: &Git2Backend, _url: &str, _identity: Option<&SelectedIdentity>,
            _attempt: Option<&TransportAttempt>, _callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {}
    }
}
