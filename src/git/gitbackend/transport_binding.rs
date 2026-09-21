//! Candidate binding is isolated until source/platform qualification permits
//! activation. Default builds retain the native transport and dependency graph.
use super::{
    backend::Git2Backend, transport_observations::TransportAttempt,
    transport_support::identity::SelectedIdentity,
};

cfg_if::cfg_if! {
    if #[cfg(all(unix, gwz_transport_candidate))] {
        use crate::git::endpoint::{
            ssh_destination::Destination, ssh_endpoint::Route, ssh_local, ssh_remote::RemoteTransport,
            ssh_worker::Endpoint,
        };
        use std::{
            io,
            sync::{Arc, Mutex},
        };

        #[derive(Clone)]
        pub(crate) struct Runtime(Arc<RuntimeState>);
        struct RuntimeState {
            endpoint: Mutex<Option<Endpoint>>,
            factory: Box<dyn Fn() -> io::Result<Endpoint> + Send + Sync>,
        }
        impl Default for Runtime {
            fn default() -> Self {
                Self::with_factory(|| {
                    let home = std::env::var_os("HOME").ok_or(io::ErrorKind::NotFound)?;
                    let known = std::path::PathBuf::from(home).join(".ssh/known_hosts");
                    let agent = std::env::var_os("SSH_AUTH_SOCK")
                        .filter(|p| !p.is_empty())
                        .map(Into::into);
                    let timeout = super::transport_support::server_timeout_ms();
                    let config = gwz_transport::pool::Config {
                        connect_timeout_ms: timeout,
                        ..Default::default()
                    };
                    ssh_local::connect(config, known, agent, timeout)
                })
            }
        }
        impl Runtime {
            pub(super) fn with_factory(
                factory: impl Fn() -> io::Result<Endpoint> + Send + Sync + 'static,
            ) -> Self {
                Self(Arc::new(RuntimeState {
                    endpoint: Mutex::new(None),
                    factory: Box::new(factory),
                }))
            }
            pub(super) fn endpoint(&self) -> io::Result<Endpoint> {
                // Construction reserves ownership but performs no network/trust I/O.
                // Serialize it, publishing only success; transient failures may retry.
                let mut endpoint = self.0.endpoint.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(endpoint) = &*endpoint {
                    return Ok(endpoint.clone());
                }
                let created = (self.0.factory)()?;
                *endpoint = Some(created.clone());
                Ok(created)
            }
            cfg_if::cfg_if! {
                if #[cfg(test)] {
                    pub(crate) fn from_endpoint(endpoint: Endpoint) -> Self {
                        Self::with_factory(move || Ok(endpoint.clone()))
                    }
                }
            }
        }
        pub(crate) fn configure(
            backend: &Git2Backend,
            url: &str,
            identity: Option<&SelectedIdentity>,
            attempt: Option<&TransportAttempt>,
            callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {
            if matches!(Destination::parse(url), Ok(None)) {
                return;
            }
            let runtime = backend.ssh.clone();
            let selected = identity.map(|i| i.path.clone());
            let attempt = attempt.cloned();
            callbacks.smart_transport(false, move |_| {
                let attempt = attempt.clone();
                let route = Route::reporting(
                    runtime.endpoint().map_err(|e| {
                        git2::Error::new(
                            git2::ErrorCode::GenericError,
                            git2::ErrorClass::Net,
                            format!("SSH endpoint unavailable: {:?}", e.kind()),
                        )
                    })?,
                    selected.clone(),
                    Arc::new(move |facts| {
                        if let Some(attempt) = &attempt {
                            attempt.facts(facts);
                        }
                    }),
                );
                Ok(RemoteTransport::new(Arc::new(route)))
            });
        }
        pub(super) fn repository_refused(error: &git2::Error) -> bool {
            error.class() == git2::ErrorClass::Net
                && error.message() == crate::git::endpoint::stream_io::REPOSITORY_REFUSED
        }
    } else {
        #[derive(Clone, Default)]
        pub(crate) struct Runtime;
        pub(super) fn repository_refused(_error: &git2::Error) -> bool { false }
        pub(crate) fn configure(
            _backend: &Git2Backend, _url: &str, _identity: Option<&SelectedIdentity>,
            _attempt: Option<&TransportAttempt>, _callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {}
    }
}
