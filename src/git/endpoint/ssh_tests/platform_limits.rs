//! TD5 through the whole open: where libssh2 on Windows (WinCNG) cannot do what the open needs, the open ends with the
//! limit, not a transport error to retry, and its facts tell the host which limit (`ssh_limits`). The words the host
//! gives it are `transport_host::session::driver`'s.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        use super::{attachment, key_material as keys};
        use crate::git::endpoint::{
            setup_retry::{Phase, Verdict, classify},
            shared_reservation::Authority,
            ssh_channel::GitService,
            ssh_fixture::SshdFixture,
            ssh_limits::SshLimit,
            ssh_local,
            ssh_worker::{Endpoint, EndpointOpenFailure},
        };
        use gwz_transport::{
            pool::{Config, Key},
            protocol::ErrorCode,
        };
        use std::{
            fs, io,
            path::PathBuf,
            thread,
            time::{Duration, Instant},
        };

        /// The endpoint for `fixture`'s server, with the fixture's trust file and no agent.
        fn endpoint(fixture: &SshdFixture, config: &Config) -> Endpoint {
            let authority = Authority::new(config.total, config.per_host);
            ssh_local::connect_with_authority(
                config.clone(),
                fixture.known_hosts.clone(),
                None,
                3_000,
                authority,
                Default::default(),
            )
            .unwrap()
        }

        /// Opens a stream, ambient or with the identity file `selected`, and returns the failure the open ended with.
        fn failed_open(
            endpoint: &Endpoint,
            fixture: &SshdFixture,
            config: &Config,
            selected: Option<PathBuf>,
        ) -> EndpointOpenFailure {
            let path = fixture.repository.to_str().unwrap().to_owned();
            let error = attachment::open(
                endpoint,
                Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
                selected,
                GitService::UploadPack,
                &path,
                attachment::deadlines(config, 3_000),
            )
            .err()
            .expect("the open meets a limit");
            error
                .get_ref()
                .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
                .map(|failure| EndpointOpenFailure {
                    failure: failure.failure.clone(),
                    phase: failure.phase,
                })
                .unwrap_or_else(|| panic!("the bridge keeps the typed setup failure: {error}"))
        }

        fn finish(endpoint: &Endpoint) {
            endpoint.shutdown();
            let until = Instant::now() + Duration::from_secs(5);
            while !endpoint.shutdown_status().cleanup_complete {
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(2));
            }
        }

        fn config() -> Config {
            Config { total: 1, per_host: 1, per_user_host: 1, ..Config::default() }
        }

        #[test]
        fn a_known_hosts_with_no_rsa_key_ends_the_open_with_the_host_key_limit_before_any_credential() {
            let fixture = SshdFixture::new();
            // The only entry for the server is an Ed25519 key, which this libssh2 cannot verify.
            fs::write(
                &fixture.known_hosts,
                format!(
                    "[127.0.0.1]:{} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n",
                    fixture.port
                ),
            )
            .unwrap();
            let config = config();
            let endpoint = endpoint(&fixture, &config);
            let failed = failed_open(&endpoint, &fixture, &config, None);
            assert_eq!(failed.failure.code, ErrorCode::UnsupportedOperation, "{:?}", failed.failure);
            assert_eq!(failed.phase, Phase::Setup);
            // The limit is final: the retry machine closes the key, it does not try again.
            assert_eq!(classify(&failed.failure, failed.phase), Verdict::Close);
            let facts = failed.failure.facts.as_ref().expect("a setup failure carries its facts");
            assert!(!facts.credential_offered, "{facts:?}");
            assert_eq!(SshLimit::of_failure(&failed.failure), Some(SshLimit::HostKeys));
            assert_eq!(fixture.authenticated_sessions, 0);
            finish(&endpoint);
        }

        #[test]
        fn a_key_file_that_is_not_rsa_pem_ends_the_open_with_the_key_file_limit() {
            let fixture = SshdFixture::new();
            let dir = tempfile::tempdir().unwrap();
            // The form `ssh-keygen` writes by default for every type, Ed25519 here: not read by WinCNG.
            let key = keys::keygen(dir.path(), "ed25519", "ed25519", None);
            let config = config();
            let endpoint = endpoint(&fixture, &config);
            let failed = failed_open(&endpoint, &fixture, &config, Some(key));
            assert_eq!(failed.failure.code, ErrorCode::UnsupportedOperation, "{:?}", failed.failure);
            assert_eq!(classify(&failed.failure, failed.phase), Verdict::Close);
            let facts = failed.failure.facts.as_ref().expect("a setup failure carries its facts");
            assert_eq!(SshLimit::of_failure(&failed.failure), Some(SshLimit::KeyFile), "{facts:?}");
            assert_eq!(fixture.authenticated_sessions, 0);
            finish(&endpoint);
        }

        #[test]
        fn an_rsa_pem_key_file_opens_where_the_limit_would_not_apply() {
            let fixture = SshdFixture::new();
            let config = config();
            let endpoint = endpoint(&fixture, &config);
            // The fixture's own client key is RSA in PEM form on Windows.
            let key = fixture.temp.path().join("client_ed25519");
            let path = fixture.repository.to_str().unwrap().to_owned();
            let result: io::Result<_> = attachment::open(
                &endpoint,
                Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
                Some(key),
                GitService::UploadPack,
                &path,
                attachment::deadlines(&config, 3_000),
            );
            assert!(result.is_ok(), "{:?}", result.err());
            finish(&endpoint);
        }
    }
}
