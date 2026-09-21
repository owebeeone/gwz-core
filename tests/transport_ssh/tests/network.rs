#![allow(dead_code, unused_imports)]
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod common;
        use common::{SshConnection, ssh_channel, ssh_connection};
        #[path = "../../../src/git/endpoint/agent_auth.rs"]
        mod agent_auth;
        #[path = "../../../src/git/endpoint/agent_client.rs"]
        mod agent_client;
        #[path = "../../../src/git/endpoint/agent_job.rs"]
        mod agent_job;
        #[path = "../../../src/git/endpoint/agent_socket.rs"]
        mod agent_socket;
        #[path = "../../../src/git/endpoint/ssh_network.rs"]
        mod ssh_network;
        #[path = "../../../src/git/endpoint/ssh_pool.rs"]
        mod ssh_pool;
        #[path = "../../../src/git/endpoint/ssh_pump.rs"]
        mod ssh_pump;
        #[path = "../../../src/git/endpoint/ssh_setup.rs"]
        mod ssh_setup;
        #[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
        mod ssh_shutdown;
        #[path = "../../../src/git/endpoint/ssh_worker.rs"]
        mod ssh_worker;
        #[path = "../../../src/git/endpoint/stream_io.rs"]
        mod stream_io;
        #[path = "../support/agent_auth.rs"]
        mod support;
        use agent_job::Job;
        use gwz_transport::{
            pool::{Config, Identity, Key},
            protocol::{AuthMethod, Facts},
        };
        use std::{
            fs,
            io::{self, Read, Write},
            net::{TcpListener, TcpStream},
            path::{Path, PathBuf},
            process::{Command, Stdio},
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };
        fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Poll::Ready(r) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return r;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn establish(key: Key, path: PathBuf) -> io::Result<(SshConnection, Vec<u8>)> {
            finish(
                &mut Job::start(
                    Some(Instant::now() + Duration::from_secs(3)),
                    Duration::from_secs(1),
                    move |c| ssh_network::establish(&key, &path, &c),
                )
                .unwrap(),
            )
        }
        fn key(f: &common::SshdFixture) -> Key {
            Key::ssh(&f.user, "127.0.0.1", f.port)
        }
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
            let endpoint = ssh_worker::Endpoint::with_connector(
                config,
                move |origin| {
                    ssh_setup::SetupConnector::new(
                        origin,
                        Duration::from_secs(1),
                        move |key: &Key, identity: &Identity| -> io::Result<ssh_setup::Setup> {
                            assert_eq!(*identity, Identity::Ambient);
                            let key = key.clone();
                            let known = known.clone();
                            let path = path.clone();
                            Ok(Box::new(move |control| {
                                let (connection, host) = ssh_network::establish(&key, &known, &control)?;
                                let connection = agent_auth::authenticate(
                                    connection,
                                    key.username.as_deref().unwrap(),
                                    &host,
                                    control.clone(),
                                    || agent_socket::connect(&path, control),
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
                let (mut stream, opened) = endpoint
                    .open_observed(
                        key(&fixture.ssh),
                        Identity::Ambient,
                        ssh_channel::GitService::UploadPack,
                        fixture.ssh.repository.to_str().unwrap(),
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
        fn missing_unknown_and_mismatched_trust_never_reach_authentication() {
            let f = common::SshdFixture::new();
            let foreign = f.temp.path().join("foreign");
            common::run_keygen(&foreign);
            let original = fs::read_to_string(&f.known_hosts).unwrap();
            let samples = [
                None,
                Some(String::new()),
                Some(original.replace("127.0.0.1", "unknown.invalid")),
                Some(format!(
                    "[127.0.0.1]:{} {}",
                    f.port,
                    fs::read_to_string(foreign.with_extension("pub")).unwrap()
                )),
            ];
            for (i, text) in samples.into_iter().enumerate() {
                let path = f.temp.path().join(format!("trust-{i}"));
                if let Some(text) = text {
                    fs::write(&path, text).unwrap();
                }
                let reached = Arc::new(AtomicBool::new(false));
                let observed = reached.clone();
                let key = key(&f);
                let result = finish(
                    &mut Job::start(
                        Some(Instant::now() + Duration::from_secs(2)),
                        Duration::from_secs(1),
                        move |c| {
                            let pair = ssh_network::establish(&key, &path, &c)?;
                            observed.store(true, Ordering::SeqCst);
                            Ok(pair)
                        },
                    )
                    .unwrap(),
                );
                assert!(result.is_err());
                assert!(!reached.load(Ordering::SeqCst));
            }
        }
        #[test]
        fn hashed_host_and_nondefault_port_use_the_logical_destination() {
            let f = common::SshdFixture::new();
            common::run(
                Command::new("ssh-keygen")
                    .args(["-H", "-f"])
                    .arg(&f.known_hosts),
            );
            let (mut connection, approved) = establish(key(&f), f.known_hosts.clone()).unwrap();
            assert_eq!(connection.session().host_key().unwrap().0, approved);
            assert!(!connection.session().authenticated());
            let mut other = key(&f);
            other.host = "localhost".into();
            assert!(establish(other, f.known_hosts.clone()).is_err());
        }
        #[test]
        fn malformed_oversized_and_nonregular_trust_are_refused() {
            let f = common::SshdFixture::new();
            let cases = [
                b"127.0.0.1 ssh-ed25519 invalid!\n".to_vec(),
                vec![b'x'; 16 * 1024 + 1],
                vec![b'\n'; 4 * 1024 * 1024 + 1],
            ];
            for (i, bytes) in cases.into_iter().enumerate() {
                let path = f.temp.path().join(format!("bad-{i}"));
                fs::write(&path, bytes).unwrap();
                assert!(establish(key(&f), path).is_err());
            }
            assert!(establish(key(&f), f.temp.path().to_owned()).is_err());
            let fifo = f.temp.path().join("fifo");
            common::run(Command::new("mkfifo").arg(&fifo));
            let start = Instant::now();
            assert!(establish(key(&f), fifo).is_err());
            assert!(start.elapsed() < Duration::from_secs(1));
        }
        #[test]
        fn unavailable_peer_fails_without_authentication() {
            let f = common::SshdFixture::new();
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let result = establish(Key::ssh(&f.user, "127.0.0.1", port), f.known_hosts.clone());
            assert!(result.is_err());
        }
        #[test]
        fn stalled_handshake_is_cancelled_or_times_out_and_closes_the_socket() {
            for cancel in [false, true] {
                let f = common::SshdFixture::new();
                let known = f.known_hosts.clone();
                let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                let port = listener.local_addr().unwrap().port();
                let deadline = if cancel {
                    None
                } else {
                    Some(Instant::now() + Duration::from_millis(200))
                };
                let key = Key::ssh("git", "127.0.0.1", port);
                let mut job = Job::start(deadline, Duration::from_secs(1), move |c| {
                    ssh_network::establish(&key, &known, &c)
                })
                .unwrap();
                listener.set_nonblocking(true).unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                let mut peer = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < until);
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                if cancel {
                    job.cancel();
                }
                let error = finish(&mut job).err().unwrap();
                assert_eq!(
                    error.kind(),
                    if cancel {
                        io::ErrorKind::ConnectionAborted
                    } else {
                        io::ErrorKind::TimedOut
                    }
                );
                peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                let mut received = Vec::new();
                match peer.read_to_end(&mut received) {
                    Ok(_) => {}
                    Err(e) => assert_eq!(e.kind(), io::ErrorKind::ConnectionReset),
                };
            }
        }
        #[test]
        fn trusted_rsa_host_is_negotiated_when_peer_also_offers_ed25519() {
            let mut f = common::SshdFixture::new();
            let rsa = f.temp.path().join("host_rsa");
            common::run(
                Command::new("ssh-keygen")
                    .args(["-q", "-t", "rsa", "-b", "2048", "-N", "", "-f"])
                    .arg(&rsa),
            );
            let config = f.temp.path().join("sshd_config");
            let mut contents = fs::read_to_string(&config).unwrap();
            contents.push_str(&format!("HostKey {}\n", rsa.display()));
            fs::write(&config, contents).unwrap();
            f.child.kill().unwrap();
            f.child.wait().unwrap();
            f.child = Command::new("/usr/sbin/sshd")
                .args(["-D", "-e", "-f"])
                .arg(&config)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while TcpStream::connect(("127.0.0.1", f.port)).is_err() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            fs::write(
                &f.known_hosts,
                format!(
                    "[127.0.0.1]:{} {}",
                    f.port,
                    fs::read_to_string(rsa.with_extension("pub")).unwrap()
                ),
            )
            .unwrap();
            let (mut connection, _) = establish(key(&f), f.known_hosts.clone()).unwrap();
            assert!(matches!(
                connection.session().host_key().unwrap().1,
                ssh2::HostKeyType::Rsa
            ));
            drop(connection);
            let accepted_prefix = fs::read_to_string(&f.known_hosts)
                .unwrap()
                .replace("ssh-rsa", "ssh-rs");
            fs::write(&f.known_hosts, accepted_prefix).unwrap();
            let (mut connection, _) = establish(key(&f), f.known_hosts.clone()).unwrap();
            assert!(matches!(
                connection.session().host_key().unwrap().1,
                ssh2::HostKeyType::Rsa
            ));
        }
        fn padded_trust(line: &str, bytes: usize) -> String {
            let mut text = line.to_owned();
            while text.len() + 4091 <= bytes {
                text.push('#');
                text.push_str(&" ".repeat(4089));
                text.push('\n');
            }
            let left = bytes - text.len();
            if left > 0 {
                text.push('#');
                text.push_str(&" ".repeat(left - 1));
            }
            assert_eq!(text.len(), bytes);
            text
        }
        #[test]
        fn trust_byte_and_line_admission_boundaries_are_explicit() {
            let f = common::SshdFixture::new();
            let line = fs::read_to_string(&f.known_hosts).unwrap();
            for bytes in [4 * 1024 * 1024 - 1, 4 * 1024 * 1024, 4 * 1024 * 1024 + 1] {
                fs::write(&f.known_hosts, padded_trust(&line, bytes)).unwrap();
                let result = establish(key(&f), f.known_hosts.clone());
                if bytes <= 4 * 1024 * 1024 {
                    assert!(result.is_ok(), "{bytes}: {:?}", result.err());
                } else {
                    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
                }
            }
            for bytes in [16 * 1024 - 1, 16 * 1024, 16 * 1024 + 1] {
                let mut text = line.trim_end().to_owned();
                text.push_str(&" ".repeat(bytes - text.len()));
                text.push('\n');
                fs::write(&f.known_hosts, text).unwrap();
                let result = establish(key(&f), f.known_hosts.clone());
                if bytes <= 16 * 1024 {
                    assert!(result.is_ok(), "{bytes}: {:?}", result.err());
                } else {
                    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
                }
            }
        }
        #[test]
        fn native_trust_accepts_large_file_that_bounded_endpoint_explicitly_refuses() {
            let mut f = common::SshdFixture::new();
            let mut connection = f.session();
            let host = connection.session().host_key().unwrap().0.to_vec();
            let line = fs::read_to_string(&f.known_hosts).unwrap();
            fs::write(&f.known_hosts, padded_trust(&line, 5 * 1024 * 1024)).unwrap();
            let mut native = connection.session().known_hosts().unwrap();
            native
                .read_file(&f.known_hosts, ssh2::KnownHostFileKind::OpenSSH)
                .unwrap();
            assert!(matches!(
                native.check_port("127.0.0.1", f.port, &host),
                ssh2::CheckResult::Match
            ));
            drop(native);
            drop(connection);
            assert_eq!(
                establish(key(&f), f.known_hosts.clone())
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        #[test]
        fn complete_line_parsing_has_the_documented_native_differential() {
            let mut f = common::SshdFixture::new();
            let mut connection = f.session();
            let host = connection.session().host_key().unwrap().0.to_vec();
            let original = fs::read_to_string(&f.known_hosts)
                .unwrap()
                .trim_end()
                .to_owned();
            for host_list in [false, true] {
                for bytes in [4090, 4091, 4092] {
                    let extra = bytes - original.len();
                    let line = if host_list {
                        format!(
                            "{}{}{}",
                            if extra % 2 == 1 { "x" } else { "" },
                            "x,".repeat(extra / 2),
                            original
                        )
                    } else {
                        format!("{original}{}", "z".repeat(extra))
                    };
                    assert_eq!(line.len(), bytes);
                    fs::write(&f.known_hosts, line + "\n").unwrap();
                    let mut native = connection.session().known_hosts().unwrap();
                    let baseline = native
                        .read_file(&f.known_hosts, ssh2::KnownHostFileKind::OpenSSH)
                        .is_ok();
                    assert_eq!(
                        baseline,
                        bytes <= 4091,
                        "host_list={host_list}, bytes={bytes}"
                    );
                    if baseline {
                        assert!(matches!(
                            native.check_port("127.0.0.1", f.port, &host),
                            ssh2::CheckResult::Match
                        ));
                    }
                    drop(native);
                    assert!(establish(key(&f), f.known_hosts.clone()).is_ok());
                }
            }
            fs::write(&f.known_hosts, format!("\x0b\n{original}\n")).unwrap();
            let mut native = connection.session().known_hosts().unwrap();
            assert!(
                native
                    .read_file(&f.known_hosts, ssh2::KnownHostFileKind::OpenSSH)
                    .is_err()
            );
            drop(native);
            assert!(establish(key(&f), f.known_hosts.clone()).is_err());
        }
        #[test]
        fn late_loader_and_resolver_results_are_disposed_without_later_effects() {
            use std::sync::{Mutex, mpsc};
            for stall_load in [false, true] {
                let f = common::SshdFixture::new();
                let text = fs::read_to_string(&f.known_hosts).unwrap();
                let known = f.known_hosts.clone();
                let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                listener.set_nonblocking(true).unwrap();
                let address = listener.local_addr().unwrap();
                let key = Key::ssh("git", "localhost", address.port());
                let (started, entered) = mpsc::channel();
                let (release, finish_gate) = mpsc::channel();
                let gate = Arc::new(Mutex::new(finish_gate));
                let load_gate = gate.clone();
                let load_started = started.clone();
                let resolved = Arc::new(AtomicBool::new(false));
                let observed = resolved.clone();
                let mut job = Job::start(None, Duration::from_millis(10), move |c| {
                    ssh_network::establish_with(
                        &key,
                        &known,
                        &c,
                        move |_, _| {
                            if stall_load {
                                load_started.send(()).unwrap();
                                load_gate.lock().unwrap().recv().unwrap();
                            }
                            Ok(text)
                        },
                        move |_, _, _| {
                            observed.store(true, Ordering::SeqCst);
                            if !stall_load {
                                started.send(()).unwrap();
                                gate.lock().unwrap().recv().unwrap();
                            }
                            Ok(vec![address])
                        },
                    )
                })
                .unwrap();
                entered.recv_timeout(Duration::from_secs(2)).unwrap();
                job.cancel();
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    match job.poll_disposed(&mut Context::from_waker(Waker::noop())) {
                        Poll::Ready(Err(e)) => {
                            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
                            break;
                        }
                        Poll::Pending => {}
                        _ => panic!("live adapter falsely disposed"),
                    };
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(1));
                }
                release.send(()).unwrap();
                assert_eq!(
                    finish(&mut job).err().unwrap().kind(),
                    io::ErrorKind::ConnectionAborted
                );
                assert_eq!(resolved.load(Ordering::SeqCst), !stall_load);
                assert!(matches!(listener.accept(),Err(e) if e.kind()==io::ErrorKind::WouldBlock));
                assert!(matches!(
                    job.poll_disposed(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Ok(()))
                ));
            }
        }
        #[test]
        fn address_fallback_stops_at_the_first_ssh_negotiation() {
            let f = common::SshdFixture::new();
            let text = fs::read_to_string(&f.known_hosts).unwrap();
            let known = f.known_hosts.clone();
            let destination = key(&f);
            let dead = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let dead_address = dead.local_addr().unwrap();
            drop(dead);
            let live = ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), f.port).into();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |c| {
                    ssh_network::establish_with(
                        &destination,
                        &known,
                        &c,
                        move |_, _| Ok(text),
                        move |_, _, _| Ok(vec![dead_address, live]),
                    )
                },
            )
            .unwrap();
            assert!(finish(&mut job).is_ok());
            let first = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let first_address = first.local_addr().unwrap();
            let second = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            second.set_nonblocking(true).unwrap();
            let second_address = second.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut peer, _) = first.accept().unwrap();
                peer.write_all(b"SSH-2.0-invalid_peer\r\n").unwrap();
            });
            let key = key(&f);
            let known = f.known_hosts.clone();
            let text = fs::read_to_string(&known).unwrap();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(2)),
                Duration::from_secs(1),
                move |c| {
                    ssh_network::establish_with(
                        &key,
                        &known,
                        &c,
                        move |_, _| Ok(text),
                        move |_, _, _| Ok(vec![first_address, second_address]),
                    )
                },
            )
            .unwrap();
            assert!(finish(&mut job).is_err());
            server.join().unwrap();
            assert!(matches!(second.accept(),Err(e)if e.kind()==io::ErrorKind::WouldBlock));
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
            let mut job = Job::start(
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
                    agent_auth::authenticate(
                        connection,
                        key.username.as_deref().unwrap(),
                        &host,
                        c.clone(),
                        || agent_socket::connect(&path, c),
                    )
                },
            )
            .unwrap();
            assert!(finish(&mut job).is_err());
            assert!(matches!(second.accept(),Err(e)if e.kind()==io::ErrorKind::WouldBlock));
            fixture.assert_requests("ssh-ed25519", 0);
        }
        #[test]
        fn invalid_encoding_and_nul_are_refused_before_resolution() {
            let f = common::SshdFixture::new();
            for bytes in [
                vec![255],
                b"# comment\0suffix".to_vec(),
                vec![b'x'; 4 * 1024 * 1024 + 1],
                vec![b'x'; 16 * 1024 + 1],
            ] {
                fs::write(&f.known_hosts, bytes).unwrap();
                let key = key(&f);
                let path = f.known_hosts.clone();
                let resolved = Arc::new(AtomicBool::new(false));
                let observed = resolved.clone();
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(3)),
                    Duration::from_secs(1),
                    move |c| {
                        ssh_network::establish_with(
                            &key,
                            &path,
                            &c,
                            ssh_network::read_regular,
                            move |_, _, _| {
                                observed.store(true, Ordering::SeqCst);
                                Ok(vec![])
                            },
                        )
                    },
                )
                .unwrap();
                assert_eq!(
                    finish(&mut job).err().unwrap().kind(),
                    io::ErrorKind::InvalidInput
                );
                assert!(!resolved.load(Ordering::SeqCst));
            }
        }
    }
}
