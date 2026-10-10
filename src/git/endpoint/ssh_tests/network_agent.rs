//! Network setup through the loopback `sshd` with the ambient agent: the rows of `network` that need
//! the Unix agent socket (`agent_auth`, `agent_socket`, `agent_fixture`). Step 3.4 gives them a Windows twin.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::network::{finish, key};
        use crate::git::endpoint::{
            agent_auth, agent_job::Job, agent_socket, ssh_channel, ssh_key_snapshot, ssh_network, ssh_setup,
            ssh_worker,
        };
        use super::agent_fixture as support;
        use gwz_transport::{
            pool::{Config, Identity, Key},
            protocol::{AuthMethod, Facts},
        };
        use std::{
            fs,
            io::{self, Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        #[test]
        fn fresh_network_setup_authenticates_and_reuses_in_the_shared_pool() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let known = fixture.ssh.known_hosts.clone();
            let path = fixture.path.clone();
            let config = Config {
                total: 1,
                per_host: 1,
                per_user_host: 1,
                ..Config::default()
            };
            let deadlines = super::attachment::deadlines(&config, 1000);
            let endpoint = ssh_worker::Endpoint::with_registry(
                config,
                ssh_key_snapshot::Registry::new(),
                move |origin, _| {
                    ssh_setup::SetupConnector::isolated(
                        origin,
                        Duration::from_secs(1),
                        move |key: &Key, identity: &Identity, _| -> io::Result<ssh_setup::Setup> {
                            assert_eq!(*identity, Identity::Ambient);
                            let key = key.clone();
                            let known = known.clone();
                            let path = path.clone();
                            Ok(Box::new(move |control| {
                                let (connection, host) = ssh_network::establish(&key, &known, &control)?;
                                let connection = agent_auth::authenticate_reporting(
                                    connection,
                                    key.username.as_deref().unwrap(),
                                    &host,
                                    control.clone(),
                                    || agent_socket::connect(&path, control),
                                    || {},
                                    || {},
                                )?;
                                ssh_setup::Authenticated::new(
                                    connection,
                                    Identity::Ambient,
                                    Facts {
                                        method: AuthMethod::SshAgent,
                                        authenticated: Some(true),
                                        credential_offered: true,
                                        ..Facts::default()
                                    },
                                )
                            }))
                        },
                    )
                },
                1000,
            )
            .unwrap();
            for reused in [false, true] {
                let (mut stream, opened) = super::attachment::open(
                    &endpoint,
                    key(&fixture.ssh),
                    None,
                    ssh_channel::GitService::UploadPack,
                    fixture.ssh.repository.to_str().unwrap(),
                    deadlines.clone(),
                )
                .unwrap();
                assert_eq!(opened.reused, reused);
                assert_eq!(opened.facts.credential_offered, !reused);
                stream.write_all(b"0000").unwrap();
                stream.end_write().unwrap();
                stream.read_to_end(&mut Vec::new()).unwrap();
                stream.close().unwrap();
            }
            endpoint.shutdown();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !endpoint.shutdown_status().cleanup_complete {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(endpoint.shutdown_status().failure, None);
            fixture.assert_requests("ssh-ed25519", 1);
        }
        #[test]
        fn terminal_authentication_failure_never_tries_another_address() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            fs::write(fixture.ssh.temp.path().join("authorized_keys"), b"").unwrap();
            let second = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            second.set_nonblocking(true).unwrap();
            let other = second.local_addr().unwrap();
            let first = (
                "127.0.0.1".parse::<std::net::IpAddr>().unwrap(),
                fixture.ssh.port,
            )
                .into();
            let key = key(&fixture.ssh);
            let known = fixture.ssh.known_hosts.clone();
            let text = fs::read_to_string(&known).unwrap();
            let path = fixture.path.clone();
            let mut job = Job::start_isolated(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |c| {
                    let (connection, host) = ssh_network::establish_with(
                        &key,
                        &known,
                        &c,
                        move |_, _| Ok(text),
                        move |_, _, _| Ok(vec![first, other]),
                    )?;
                    agent_auth::authenticate_reporting(
                        connection,
                        key.username.as_deref().unwrap(),
                        &host,
                        c.clone(),
                        || agent_socket::connect(&path, c),
                        || {},
                        || {},
                    )
                },
            )
            .unwrap();
            assert!(finish(&mut job).is_err());
            assert!(matches!(second.accept(),Err(e)if e.kind()==io::ErrorKind::WouldBlock));
            fixture.assert_requests("ssh-ed25519", 0);
        }
    }
}
