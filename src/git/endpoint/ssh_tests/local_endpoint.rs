//! The endpoint production builds, `ssh_local::connect_with_authority`, with
//! native trust and both authentication modes, through the attachment path.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::agent_fixture as support;
        use super::attachment;
        use crate::git::endpoint::{
            agent_auth, agent_socket,
            shared_reservation::Authority,
            ssh_channel::GitService,
            ssh_fixture as common,
            ssh_key_snapshot::Registry,
            ssh_local, ssh_network, ssh_pool, ssh_setup,
            ssh_worker::{Endpoint, EndpointOpenFailure},
            stream_io::BlockingStream,
        };
        use gwz_transport::{
            pool::{Config, Key},
            protocol::{AuthMethod, Deadlines, ErrorCode, Facts, Opened},
        };
        use std::{
            fs,
            io::{self, Read, Write},
            path::PathBuf,
            sync::Arc,
            time::{Duration, Instant},
        };
        fn config() -> Config {
            Config {
                total: 2,
                per_host: 2,
                per_user_host: 1,
                cleanup_timeout_ms: 100,
                ..Config::default()
            }
        }
        /// The endpoint as the transport host builds it, over `config`.
        fn connect(config: Config, known_hosts: PathBuf, agent: Option<PathBuf>, io_timeout_ms: u64) -> Endpoint {
            let authority = Authority::new(config.total, config.per_host);
            ssh_local::connect_with_authority(config, known_hosts, agent, io_timeout_ms, authority, Default::default()).unwrap()
        }
        fn endpoint(f: &common::SshdFixture, agent: Option<PathBuf>) -> Endpoint {
            connect(config(), f.known_hosts.clone(), agent, 3000)
        }
        fn deadlines() -> Deadlines {
            attachment::deadlines(&config(), 3000)
        }
        fn key(f: &common::SshdFixture) -> Key {
            Key::ssh(&f.user, "127.0.0.1", f.port)
        }
        /// One upload-pack exchange to completion: its receipt, or why it failed.
        fn exchange(e: &Endpoint, f: &common::SshdFixture, selected: Option<PathBuf>) -> io::Result<Opened> {
            let (stream, opened) = attachment::open(
                e,
                key(f),
                selected,
                GitService::UploadPack,
                f.repository.to_str().unwrap(),
                deadlines(),
            )?;
            drain(stream)?;
            Ok(opened)
        }
        fn drain(mut stream: BlockingStream) -> io::Result<()> {
            stream.write_all(b"0000")?;
            stream.end_write()?;
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            assert!(!bytes.is_empty());
            stream.close()?;
            Ok(())
        }
        /// The facts a failed open carried across the bridge.
        fn failure(error: &io::Error) -> gwz_transport::protocol::Failure {
            error
                .get_ref()
                .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
                .expect("the bridge keeps the typed setup failure")
                .failure
                .clone()
        }
        fn facts(error: &io::Error) -> Facts {
            failure(error).facts.expect("a setup failure carries its facts")
        }
        #[test]
        fn configured_stall_reaches_native_handshake_through_the_endpoint() {
            use gwz_transport::protocol::SetupFailureCause;
            use std::net::TcpListener;
            use std::sync::mpsc;

            let temp = tempfile::tempdir().unwrap();
            let known_hosts = temp.path().join("known_hosts");
            fs::write(&known_hosts, "").unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (accepted_tx, accepted_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let server = std::thread::spawn(move || {
                listener.set_nonblocking(true).unwrap();
                let until = Instant::now() + Duration::from_secs(3);
                loop {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            accepted_tx.send(()).unwrap();
                            let _ = release_rx.recv_timeout(Duration::from_secs(3));
                            drop(socket);
                            return;
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < until, "setup never connected");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("listener failed: {error}"),
                    }
                }
            });
            let aggregate = Duration::from_secs(2);
            let config = Config {
                connect_timeout_ms: aggregate.as_millis() as u64,
                cleanup_timeout_ms: 200,
                ..Config::default()
            };
            let endpoint = connect(config.clone(), known_hosts, None, 100);
            let started = Instant::now();
            let error = attachment::finish(
                &attachment::start(
                    &endpoint,
                    Key::ssh("git", "127.0.0.1", port),
                    None,
                    GitService::UploadPack,
                    "repo",
                    attachment::deadlines(&config, 100),
                )
                .unwrap(),
            )
            .err()
            .expect("stalled native handshake must fail");
            accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(
                failure(&error).setup_cause,
                Some(SetupFailureCause::Stall),
                "aggregate expiry would mean the configured stall never reached setup"
            );
            assert!(started.elapsed() < aggregate);
            release_tx.send(()).unwrap();
            server.join().unwrap();
            finish(&endpoint);
        }
        fn finish(e: &Endpoint) {
            e.shutdown();
            let until = Instant::now() + Duration::from_secs(5);
            while !e.shutdown_status().cleanup_complete {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(e.shutdown_status().failure, None);
        }
        #[test]
        fn invalid_pool_configuration_is_refused_by_the_constructor() {
            let temp = tempfile::tempdir().unwrap();
            let config = Config {
                total: 0,
                ..Config::default()
            };
            assert!(
                ssh_local::connect_with_authority(
                    config,
                    temp.path().join("known_hosts"),
                    None,
                    1000,
                    Authority::new(1, 1),
                    Default::default(),
                )
                .is_err()
            );
        }
        #[test]
        fn ambient_reuse_and_selected_refusal_do_not_cross_authority() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let receipts = [exchange(&e, &f.ssh, None).unwrap(), exchange(&e, &f.ssh, None).unwrap()];
            // This key is valid but unauthorized by the fixture; the ambient session
            // already in the capacity-one pool must never satisfy it or retry agent auth.
            assert!(exchange(&e, &f.ssh, Some(f.ssh.temp.path().join("client_ed25519"))).is_err());
            assert_eq!(receipts[0].facts.method, AuthMethod::SshAgent);
            assert!(!receipts[0].reused);
            assert!(receipts[1].reused);
            assert_eq!(receipts[0].connection_id, receipts[1].connection_id);
            assert!(!receipts[1].facts.credential_offered);
            f.assert_requests("ssh-ed25519", 1);
            finish(&e);
        }
        #[test]
        fn selected_reuse_reloads_path_and_never_contacts_available_agent() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let selected_path = f.ssh.temp.path().join("auth_key");
            exchange(&e, &f.ssh, Some(selected_path.clone())).unwrap();
            let reused = exchange(&e, &f.ssh, Some(selected_path.clone())).unwrap();
            assert!(reused.reused);
            fs::remove_file(&selected_path).unwrap();
            assert!(exchange(&e, &f.ssh, Some(selected_path)).is_err());
            f.no_requests();
            finish(&e);
        }
        #[test]
        fn ambient_cannot_reuse_selected_connection_without_agent() {
            let f = common::SshdFixture::new();
            let e = endpoint(&f, None);
            exchange(&e, &f, Some(f.temp.path().join("client_ed25519"))).unwrap();
            assert!(exchange(&e, &f, None).is_err());
            finish(&e);
        }
        #[test]
        fn untrusted_host_refuses_before_agent_io() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            fs::write(&f.ssh.known_hosts, "").unwrap();
            let error = exchange(&e, &f.ssh, None).unwrap_err();
            assert!(!facts(&error).credential_offered);
            f.no_requests();
            finish(&e);
        }
        #[test]
        fn agent_reporting_survives_a_failed_signature_and_does_not_claim_rejection() {
            let f = support::Fixture::new("ssh-ed25519", false);
            f.fault.store(1, std::sync::atomic::Ordering::SeqCst);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let facts = facts(&exchange(&e, &f.ssh, None).unwrap_err());
            assert_eq!(facts.method, AuthMethod::SshAgent);
            assert!(facts.credential_offered);
            assert_eq!(
                facts.authenticated, None,
                "a local signing error is not a server rejection"
            );
            finish(&e);
        }
        #[test]
        fn concurrent_authentication_receipts_belong_to_the_initiating_request() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let config = Config {
                total: 2,
                per_host: 2,
                per_user_host: 2,
                ..Config::default()
            };
            let e = connect(config.clone(), f.ssh.known_hosts.clone(), Some(f.path.clone()), 3000);
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let threads: Vec<_> = ["auth_key", "client_ed25519"]
                .into_iter()
                .map(|selected| {
                    let e = e.clone();
                    let selected = f.ssh.temp.path().join(selected);
                    let key = key(&f.ssh);
                    let repository = f.ssh.repository.to_str().unwrap().to_owned();
                    let deadlines = attachment::deadlines(&config, 3000);
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        attachment::open(&e, key, Some(selected), GitService::UploadPack, &repository, deadlines)
                            .and_then(|(stream, opened)| drain(stream).map(|()| opened))
                    })
                })
                .collect();
            let mut results = threads.into_iter().map(|thread| thread.join().unwrap());
            let good = results.next().unwrap().unwrap().facts;
            let bad = facts(&results.next().unwrap().unwrap_err());
            assert!(good.credential_offered && bad.credential_offered);
            assert_eq!(good.authenticated, Some(true));
            assert_eq!(bad.authenticated, Some(false));
            f.no_requests();
            finish(&e);
        }
        #[test]
        fn rejection_then_deadline_remains_timeout_at_the_endpoint_boundary() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let known = f.ssh.known_hosts.clone();
            let socket = f.path.clone();
            let config = Config {
                connect_timeout_ms: 800,
                cleanup_timeout_ms: 200,
                ..Default::default()
            };
            let e = Endpoint::with_registry(
                config.clone(),
                Registry::new(),
                move |origin, _| {
                    ssh_setup::SetupConnector::reported(
                        origin,
                        Duration::from_millis(200),
                        move |key, _, opening: ssh_pool::Opening| {
                            let progress = opening.progress;
                            let key = key.clone();
                            let known = known.clone();
                            let socket = socket.clone();
                            Ok(Box::new(move |control| {
                                let (connection, trusted) = ssh_network::establish(&key, &known, &control)?;
                                let connection = agent_auth::authenticate_reporting(
                                    connection,
                                    key.username.as_deref().unwrap(),
                                    &trusted,
                                    control.clone(),
                                    || agent_socket::connect(&socket, control.clone()),
                                    || {
                                        let mut facts = progress.lock().unwrap();
                                        facts.method = AuthMethod::SshAgent;
                                        facts.credential_offered = true;
                                        facts.authenticated = None;
                                    },
                                    || {
                                        progress.lock().unwrap().authenticated = Some(false);
                                        while control.check().is_ok() {
                                            std::thread::sleep(Duration::from_millis(1));
                                        }
                                    },
                                )?;
                                ssh_setup::Authenticated::new(
                                    connection,
                                    gwz_transport::pool::Identity::Ambient,
                                    Facts {
                                        authenticated: Some(true),
                                        ..Default::default()
                                    },
                                )
                            }))
                        },
                    )
                },
                3000,
            )
            .unwrap();
            let error = attachment::open(
                &e,
                key(&f.ssh),
                None,
                GitService::UploadPack,
                f.ssh.repository.to_str().unwrap(),
                attachment::deadlines(&config, 3000),
            )
            .err()
            .unwrap();
            let failure = failure(&error);
            assert_eq!(failure.code, ErrorCode::Timeout, "{error:?}");
            let facts = failure.facts.unwrap();
            assert!(facts.credential_offered);
            assert_eq!(facts.authenticated, Some(false));
            assert!(
                f.signing.try_recv().is_err(),
                "second, valid key was never offered"
            );
            finish(&e);
        }

        #[test]
        fn exhausted_agent_refusals_remain_authentication_rejection() {
            let f = support::Fixture::new("ssh-ed25519", false);
            fs::copy(
                f.ssh.temp.path().join("client_ed25519.pub"),
                f.ssh.temp.path().join("authorized_keys"),
            )
            .unwrap();
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let failure = failure(&exchange(&e, &f.ssh, None).unwrap_err());
            assert_eq!(failure.code, ErrorCode::Authentication);
            assert_eq!(failure.facts.unwrap().authenticated, Some(false));
            finish(&e);
        }
    }
}
