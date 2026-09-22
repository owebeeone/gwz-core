//! Candidate binding is isolated until source/platform qualification permits
//! activation. Default builds retain the native transport and dependency graph.
use super::{
    backend::Git2Backend, transport_observations::TransportAttempt,
    transport_support::identity::SelectedIdentity,
};

cfg_if::cfg_if! {
    if #[cfg(all(unix, gwz_transport_candidate))] {
        use crate::git::endpoint::{
            https_remote, https_remote::OpenRpc, ssh_channel::GitService as SshGitService,
            ssh_destination::Destination, ssh_endpoint::Route, ssh_local,
            ssh_remote::OpenStream, ssh_remote::RemoteTransport, ssh_worker::Endpoint,
        };
        use gwz_transport::protocol::{AuthPolicy, Facts, GitService, Opened};
        use std::{
            io,
            sync::{Arc, Mutex},
        };

        #[derive(Clone)]
        pub(crate) struct Runtime(Arc<RuntimeState>);
        struct RuntimeState {
            endpoint: Mutex<Option<Endpoint>>,
            factory: Box<dyn Fn() -> io::Result<Endpoint> + Send + Sync>,
            host_context: Mutex<Option<crate::transport_host::RequestContext>>,
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
                    host_context: Mutex::new(None),
                }))
            }
            pub(crate) fn with_host_context(&self, context: crate::transport_host::RequestContext) -> Self {
                *self.0.host_context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context);
                self.clone()
            }
            pub(crate) fn host_context(&self) -> Option<crate::transport_host::RequestContext> {
                self.0.host_context.lock().unwrap_or_else(|e| e.into_inner()).clone()
            }
            pub(crate) fn is_cli_context(&self) -> bool {
                self.host_context().is_some_and(|context| context.is_cli())
            }
            pub(crate) fn validate_scope(
                &self,
                meta: &crate::RequestMeta,
                operation_id: &str,
            ) -> crate::model::ModelResult<()> {
                if let Some(context) = self.host_context() {
                    context.validate(meta, operation_id)
                } else {
                    Ok(())
                }
            }
            pub(crate) fn check_identity(&self, raw: &str) -> crate::model::ModelResult<()> {
                if let Some(context) = self.host_context() {
                    context.check_identity(raw)
                } else {
                    Ok(())
                }
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
        struct HostRoute {
            context: crate::transport_host::RequestContext,
            selected: Option<String>,
            report: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
            facts: Arc<dyn Fn(&Facts) + Send + Sync>,
        }
        struct HostHttpsRoute {
            context: crate::transport_host::RequestContext,
            policy: Option<AuthPolicy>,
            report: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
            facts: Arc<dyn Fn(&Facts) + Send + Sync>,
            active: Mutex<Option<crate::git::endpoint::stream_io::BlockingStream>>,
            first_failure: Arc<Mutex<Option<crate::transport_host::HttpsAttemptReceipt>>>,
        }
        impl OpenRpc for HostHttpsRoute {
            fn open(
                &self,
                url: &str,
                service: GitService,
            ) -> io::Result<crate::git::endpoint::stream_io::BlockingStream> {
                self.context.open_https_recording(
                    url,
                    service,
                    self.policy,
                    self.report.clone(),
                    self.facts.clone(),
                    self.first_failure.clone(),
                )
                .map(|stream| {
                    *self.active.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(stream.clone());
                    stream
                })
            }

            fn cancel(&self) {
                if let Some(stream) = self
                    .active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    stream.cancel();
                }
            }
        }
        pub(super) fn is_https_remote(url: &str) -> bool {
            url.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        }
        pub(super) fn https_policy_for(
            policy: super::CredentialHelperPolicy,
        ) -> Option<AuthPolicy> {
            match policy {
                super::CredentialHelperPolicy::AllowConfigured => None,
                super::CredentialHelperPolicy::Disabled => Some(AuthPolicy::Anonymous),
            }
        }
        impl OpenStream for HostRoute {
            fn open(&self, url: &str, service: SshGitService) -> io::Result<super::super::endpoint::stream_io::BlockingStream> {
                self.context.open(
                    url,
                    service,
                    self.selected.clone(),
                    self.report.clone(),
                    self.facts.clone(),
                )
            }
        }
        pub(crate) fn configure(
            backend: &Git2Backend,
            url: &str,
            identity: Option<&SelectedIdentity>,
            attempt: Option<&TransportAttempt>,
            callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {
            let runtime = backend.ssh.clone();
            if is_https_remote(url) {
                let Some(context) = runtime.host_context() else {
                    return;
                };
                let policy = https_policy_for(backend.credential_helpers);
                let attempt = attempt.cloned();
                let facts_attempt = attempt.clone();
                let opened_attempt = attempt.clone();
                let route = HostHttpsRoute {
                    context,
                    policy,
                    report: Arc::new(move |stream_id, opened| {
                        if let Some(attempt) = &opened_attempt {
                            attempt.opened(stream_id, opened);
                            attempt.facts(&opened.facts);
                        }
                    }),
                    facts: Arc::new(move |facts| {
                        if let Some(attempt) = &facts_attempt {
                            attempt.facts(facts);
                        }
                    }),
                    active: Mutex::new(None),
                    first_failure: Arc::new(Mutex::new(None)),
                };
                https_remote::install(callbacks, Arc::new(route));
                return;
            }
            if matches!(Destination::parse(url), Ok(None)) {
                return;
            }
            let selected = identity.map(|i| i.path.clone());
            let attempt = attempt.cloned();
            callbacks.smart_transport(false, move |_| {
                let attempt = attempt.clone();
                if let Some(context) = runtime.host_context() {
                    let selected = selected.as_ref().map(|path| path.to_string_lossy().into_owned());
                    let facts_attempt = attempt.clone();
                    let opened_attempt = attempt.clone();
                    let route = HostRoute {
                        context,
                        selected,
                        report: Arc::new(move |stream_id, opened| {
                            if let Some(attempt) = &opened_attempt {
                                attempt.opened(stream_id, opened);
                                attempt.facts(&opened.facts);
                            }
                        }),
                        facts: Arc::new(move |facts| {
                            if let Some(attempt) = &facts_attempt {
                                attempt.facts(facts);
                            }
                        }),
                    };
                    return Ok(RemoteTransport::new(Arc::new(route)));
                }
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
            (error.class() == git2::ErrorClass::Net
                && error.message() == crate::git::endpoint::stream_io::REPOSITORY_REFUSED)
                || (error.class() == git2::ErrorClass::Http
                    && error.message() == crate::git::endpoint::https_remote::REPOSITORY_REFUSED)
        }
    } else {
        #[derive(Clone, Default)]
        pub(crate) struct Runtime;
        impl Runtime {
            pub(crate) fn is_cli_context(&self) -> bool {
                false
            }
            pub(crate) fn validate_scope(
                &self,
                _meta: &crate::RequestMeta,
                _operation_id: &str,
            ) -> crate::model::ModelResult<()> {
                Ok(())
            }
            pub(crate) fn check_identity(&self, _raw: &str) -> crate::model::ModelResult<()> {
                Ok(())
            }
        }
        pub(super) fn repository_refused(_error: &git2::Error) -> bool { false }
        pub(crate) fn configure(
            _backend: &Git2Backend, _url: &str, _identity: Option<&SelectedIdentity>,
            _attempt: Option<&TransportAttempt>, _callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {}
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(test, unix, gwz_transport_candidate))] {
        #[path = "https_transport_binding_tests.rs"]
        mod https_transport_binding_tests;
    }
}
