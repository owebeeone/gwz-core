use super::*;

#[test]
fn ssh_network_timeout_text_cannot_infer_a_helper_outcome() {
    for message in [
        "No credential helper answered within 0.5 seconds, a fixed bound",
        "SSH authentication needs `git` on PATH:",
    ] {
        let error = git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Net,
            message,
        );
        assert!(!credential_helper_timeout(&error));
        assert!(!credential_helper_unavailable(&error));
        assert_eq!(
            crate::git::git_error(error).code,
            crate::model::ErrorCode::GitCommandFailed
        );
    }
}

#[test]
fn candidate_binding_recognizes_only_https_as_the_host_http_route() {
    assert!(is_https_remote("https://example.invalid/owner/repo"));
    assert!(is_https_remote("HTTPS://example.invalid/owner/repo"));
    assert!(!is_https_remote("ssh://git@example.invalid/owner/repo"));
    assert!(!is_https_remote("git@example.invalid:owner/repo"));
    assert!(!is_https_remote("file:///tmp/repo"));
    assert!(
        is_https_remote("https://"),
        "invalid HTTPS must reach host validation, never native fallback"
    );
}

#[test]
fn candidate_binding_maps_helper_availability_to_https_policy() {
    // Windows asks for its own policies where the others leave the choice to the helper (`None`).
    let (allowed, disabled) = if cfg!(windows) {
        (
            Some(gwz_transport::protocol::AuthPolicy::WindowsConfigured),
            Some(gwz_transport::protocol::AuthPolicy::WindowsDefault),
        )
    } else {
        (None, Some(gwz_transport::protocol::AuthPolicy::Anonymous))
    };
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::AllowConfigured),
        allowed
    );
    assert_eq!(
        https_policy_for(crate::git::gitbackend::CredentialHelperPolicy::Disabled),
        disabled
    );
}

mod http_remote {
    //! Decision D2 (TR1.8 section 7): a plain `http://` remote on Windows goes by the native route, and the user is
    //! warned at route selection, before any request, on every operation. The platform is a test input here
    //! (`with_windows_policy_for_test`), as it is for the HTTPS policy mapping, so the rows run on every platform.
    use super::*;
    use crate::git::endpoint::https_auth::HelperSlots;
    use crate::transport_host::{
        HttpsEndpointConfig, NoteSink, SshEndpointConfig, TransportRequest, TransportRuntime,
    };
    use std::sync::{Arc, Mutex};

    const T2: &str = "warning: http://example.test/owner/repo.git is an http:// remote. GWZ carries it on the native transport: a Basic password is sent unencrypted and a Windows sign-in has no channel binding. Use an https:// address if the server offers one.";

    /// A runtime whose notes land in `lines`.
    struct Rig {
        runtime: TransportRuntime,
        lines: Arc<Mutex<Vec<String>>>,
        _home: tempfile::TempDir,
    }
    impl Rig {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let runtime = TransportRuntime::with_https(
                SshEndpointConfig::fixture(home.path().to_path_buf(), None),
                HttpsEndpointConfig {
                    tls: Default::default(),
                    auth: None,
                },
                HelperSlots::new(),
            )
            .unwrap();
            let lines = Arc::new(Mutex::new(Vec::<String>::new()));
            let kept = lines.clone();
            runtime.set_notes(&NoteSink::new(move |line| {
                kept.lock().unwrap().push(line.to_owned());
            }));
            Self {
                runtime,
                lines,
                _home: home,
            }
        }
        /// One operation's request and its backend, on the platform under test.
        async fn operation(&self, id: &str, windows: bool) -> (TransportRequest, Git2Backend) {
            let request = self
                .runtime
                .request(
                    crate::RequestMeta {
                        schema_version: "gwz.protocol/v0".into(),
                        request_id: id.into(),
                        ..Default::default()
                    },
                    "operation".into(),
                )
                .await
                .unwrap();
            let mut backend = request.backend().clone();
            if windows {
                backend.ssh = backend.ssh.with_windows_policy_for_test();
            }
            (request, backend)
        }
        fn lines(&self) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }
    }

    fn run(work: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(work);
    }

    fn configure_for(backend: &Git2Backend, url: &str) {
        let mut callbacks = git2::RemoteCallbacks::new();
        configure(backend, url, None, None, &mut callbacks);
    }

    #[test]
    fn an_http_remote_on_windows_is_warned_of_at_route_selection_t2() {
        run(async {
            let rig = Rig::new();
            let (_request, backend) = rig.operation("http-warns", true).await;
            configure_for(&backend, "http://example.test/owner/repo.git");
            assert_eq!(rig.lines(), [T2]);
            rig.runtime.shutdown().await;
        });
    }

    #[test]
    fn the_warning_is_given_once_per_remote_per_operation_and_again_in_the_next_operation() {
        run(async {
            let rig = Rig::new();
            let (_first, backend) = rig.operation("http-first", true).await;
            configure_for(&backend, "http://example.test/owner/repo.git");
            configure_for(&backend, "HTTP://Example.test/owner/repo.git");
            configure_for(&backend, "http://example.test/owner/other.git");
            assert_eq!(rig.lines().len(), 2, "{:?}", rig.lines());
            let (_second, backend) = rig.operation("http-second", true).await;
            configure_for(&backend, "http://example.test/owner/repo.git");
            assert_eq!(rig.lines().len(), 3, "{:?}", rig.lines());
            rig.runtime.shutdown().await;
        });
    }

    #[test]
    fn the_warning_never_repeats_a_url_user_or_password() {
        run(async {
            let rig = Rig::new();
            let (_request, backend) = rig.operation("http-secret", true).await;
            configure_for(
                &backend,
                "http://fixture-user:fixture-secret@example.test/owner/repo.git",
            );
            assert_eq!(rig.lines(), [T2]);
            rig.runtime.shutdown().await;
        });
    }

    #[test]
    fn nothing_else_warns() {
        run(async {
            let rig = Rig::new();
            let (_request, backend) = rig.operation("not-http", true).await;
            for url in [
                "https://example.test/owner/repo.git",
                "ssh://git@example.test/owner/repo.git",
                "git@example.test:owner/repo.git",
                "file:///tmp/repo",
            ] {
                configure_for(&backend, url);
            }
            assert!(rig.lines().is_empty(), "{:?}", rig.lines());
            rig.runtime.shutdown().await;
        });
    }

    #[test]
    fn off_windows_an_http_remote_is_as_it_was_and_warns_of_nothing() {
        run(async {
            let rig = Rig::new();
            let (_request, backend) = rig.operation("elsewhere", false).await;
            configure_for(&backend, "http://example.test/owner/repo.git");
            // Only the platform itself can make a runtime act as Windows when the test does not say so.
            let expected = usize::from(cfg!(windows));
            assert_eq!(rig.lines().len(), expected, "{:?}", rig.lines());
            rig.runtime.shutdown().await;
        });
    }
}
