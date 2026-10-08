//! Candidate binding is isolated until source/platform qualification permits
//! activation. Default builds retain the native transport and dependency graph.
use super::{
    backend::Git2Backend, transport_observations::TransportAttempt,
    transport_support::identity::SelectedIdentity,
};

cfg_if::cfg_if! {
    if #[cfg(any(all(unix, gwz_transport_candidate), all(windows, gwz_transport_candidate, gwz_windows_https_qualification)))] {
        use crate::git::endpoint::{
            https_remote, https_remote::OpenRpc, ssh_channel::GitService as SshGitService,
            ssh_destination::Destination, ssh_remote::OpenStream, ssh_remote::RemoteTransport,
        };
        use crate::transport_host::RequestContext;
        use gwz_transport::protocol::{AuthPolicy, Facts, GitService, Opened};
        use std::{
            io,
            sync::{Arc, Mutex},
        };

        // The host context of the operation a backend serves, if any. Only a
        // host context reaches the transport (amendment 2's TR2.11): a backend
        // without one, `Git2Backend::new()`'s, takes libgit2's native route
        // for SSH and HTTPS alike, and constructs no transport endpoint.
        cfg_if::cfg_if! {
            if #[cfg(test)] {
                #[derive(Clone, Default)] pub(crate) struct Runtime(Option<RequestContext>, bool);
            } else {
                #[derive(Clone, Default)] pub(crate) struct Runtime(Option<RequestContext>);
            }
        }

        impl Runtime {
            pub(crate) fn with_host_context(&self, context: RequestContext) -> Self {
                cfg_if::cfg_if! { if #[cfg(test)] { Self(Some(context), self.1) } else { Self(Some(context)) } }
            }
            cfg_if::cfg_if! { if #[cfg(test)] {
                pub(crate) fn with_windows_policy_for_test(mut self) -> Self { self.1 = true; self }
                pub(crate) fn https_policy_for_test(&self, policy: super::CredentialHelperPolicy) -> Option<AuthPolicy> { self.https_policy(policy) }
            } }
            fn https_policy(&self, policy: super::CredentialHelperPolicy) -> Option<AuthPolicy> {
                cfg_if::cfg_if! { if #[cfg(test)] {
                    https_policy_for_platform(policy, cfg!(windows) || self.1)
                } else { https_policy_for(policy) } }
            }
            pub(crate) fn host_context(&self) -> Option<RequestContext> {
                self.0.clone()
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
        }
        struct HostRoute {
            attempt: Option<TransportAttempt>,
            context: RequestContext,
            selected: Option<String>,
            helpers_allowed: bool,
            report: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
            facts: Arc<dyn Fn(&Facts) + Send + Sync>,
        }
        struct HostHttpsRoute {
            attempt: Option<TransportAttempt>,
            context: RequestContext,
            policy: Option<AuthPolicy>,
            report: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
            facts: Arc<dyn Fn(&Facts) + Send + Sync>,
            active: Mutex<Option<crate::git::endpoint::stream_io::BlockingStream>>,
            first_failure: Arc<Mutex<Option<crate::transport_host::HttpsAttemptReceipt>>>,
        }
        impl OpenRpc for HostHttpsRoute {
            fn report_failure(&self, failure: gwz_transport::protocol::Failure, service: GitService) {
                let message = crate::transport_host::HttpsOpenFailure {
                    failure, anonymous: None, attempts: None, service: Some(service),
                    helpers_disabled: self.policy == Some(AuthPolicy::Anonymous), cli_hint: self.context.is_cli(),
                };
                if let Some(error) = message.model_error()
                    && let Some(attempt) = &self.attempt {
                    attempt.failed(error);
                }
            }
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
                .inspect_err(|error| {
                    if let Some(failed) = error.get_ref().and_then(|e| e.downcast_ref::<crate::transport_host::HttpsOpenFailure>()) {
                        if let Some(error) = failed.model_error() {
                            if let Some(attempt) = &self.attempt { attempt.failed(error); }
                        }
                    }
                })
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
            https_policy_for_platform(policy, cfg!(windows))
        }
        fn https_policy_for_platform(policy: super::CredentialHelperPolicy, windows: bool) -> Option<AuthPolicy> {
            if windows {
                return Some(match policy {
                    super::CredentialHelperPolicy::AllowConfigured => AuthPolicy::WindowsConfigured,
                    super::CredentialHelperPolicy::Disabled => AuthPolicy::WindowsDefault,
                });
            }
            match policy {
                super::CredentialHelperPolicy::AllowConfigured => None,
                super::CredentialHelperPolicy::Disabled => Some(AuthPolicy::Anonymous),
            }
        }
        cfg_if::cfg_if! { if #[cfg(test)] {
            mod native_policy_tests {
                use super::*;
                #[test]
                fn coarse_windows_helper_policy_preserves_explicit_anonymous_shape() {
                    use super::super::super::CredentialHelperPolicy::{AllowConfigured, Disabled};
                    assert_eq!(https_policy_for_platform(AllowConfigured, true), Some(AuthPolicy::WindowsConfigured));
                    assert_eq!(https_policy_for_platform(Disabled, true), Some(AuthPolicy::WindowsDefault));
                    assert_eq!(https_policy_for_platform(AllowConfigured, false), None);
                    assert_eq!(https_policy_for_platform(Disabled, false), Some(AuthPolicy::Anonymous));
                }
            }
        } }
        impl OpenStream for HostRoute {
            fn open(&self, url: &str, service: SshGitService) -> io::Result<super::super::endpoint::stream_io::BlockingStream> {
                self.context.open_with_helpers(
                    url,
                    service,
                    self.selected.clone(),
                    self.helpers_allowed,
                    self.report.clone(),
                    self.facts.clone(),
                ).inspect_err(|error| {
                    if let Some(failed) = error.get_ref().and_then(|e| e.downcast_ref::<crate::transport_host::SshOpenFailure>())
                        && let Some(error) = failed.model_error(self.context.is_cli())
                        && let Some(attempt) = &self.attempt {
                        attempt.failed(error);
                    }
                })
            }
        }
        pub(crate) fn configure(
            backend: &Git2Backend,
            url: &str,
            identity: Option<&SelectedIdentity>,
            attempt: Option<&TransportAttempt>,
            callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {
            // The route is chosen here, before any connection opens. Without a
            // host context nothing is installed, so libgit2 serves the remote
            // with its own transports, as 1.0.17 does (TR2.11).
            let Some(context) = backend.ssh.host_context() else {
                return;
            };
            if is_https_remote(url) {
                let policy = backend.ssh.https_policy(backend.credential_helpers);
                let attempt = attempt.cloned();
                let facts_attempt = attempt.clone();
                let opened_attempt = attempt.clone();
                let route = HostHttpsRoute {
                    attempt,
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
            cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
                callbacks.smart_transport(false, |_| -> Result<RemoteTransport, git2::Error> {
                    Err(git2::Error::new(git2::ErrorCode::Invalid, git2::ErrorClass::Net, "Windows HTTPS qualification supports only HTTPS remotes"))
                });
                return;
            } }
            if matches!(Destination::parse(url), Ok(None)) {
                return;
            }
            let selected = identity.map(|i| i.path.to_string_lossy().into_owned());
            let helpers_allowed = backend.credential_helpers == super::CredentialHelperPolicy::AllowConfigured;
            let attempt = attempt.cloned();
            callbacks.smart_transport(false, move |_| {
                let facts_attempt = attempt.clone();
                let opened_attempt = attempt.clone();
                let route = HostRoute {
                    attempt: attempt.clone(),
                    context: context.clone(),
                    selected: selected.clone(),
                    helpers_allowed,
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
                Ok(RemoteTransport::new(Arc::new(route)))
            });
        }
        pub(super) fn repository_refused(error: &git2::Error) -> bool {
            (error.class() == git2::ErrorClass::Net
                && error.message() == crate::git::endpoint::stream_io::REPOSITORY_REFUSED)
                || (error.class() == git2::ErrorClass::Http
                    && error.message() == crate::git::endpoint::https_remote::REPOSITORY_REFUSED)
        }
        pub(super) fn credential_helper_unavailable(error: &git2::Error) -> bool {
            error.code() == git2::ErrorCode::NotFound && error.class() == git2::ErrorClass::Http
                && error.message().starts_with("HTTPS authentication needs `git` on PATH:")
        }
        pub(super) fn credential_helper_timeout(error: &git2::Error) -> bool {
            error.code() == git2::ErrorCode::Timeout
                && error.class() == git2::ErrorClass::Http
                && crate::transport_host::HttpsOpenFailure::is_helper_timeout_message(error.message())
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
        pub(super) fn credential_helper_unavailable(_error: &git2::Error) -> bool { false }
        pub(super) fn credential_helper_timeout(_error: &git2::Error) -> bool { false }
        pub(crate) fn configure(
            _backend: &Git2Backend, _url: &str, _identity: Option<&SelectedIdentity>,
            _attempt: Option<&TransportAttempt>, _callbacks: &mut git2::RemoteCallbacks<'_>,
        ) {}
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(test, any(all(unix, gwz_transport_candidate), all(windows, gwz_transport_candidate, gwz_windows_https_qualification))))] {
        #[path = "https_transport_binding_tests.rs"]
        mod https_transport_binding_tests;
    }
}
