#![allow(dead_code)]
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[path = "../../../src/git/endpoint/agent_auth.rs"]
        mod agent_auth;
        #[path = "../../../src/git/endpoint/agent_client.rs"]
        mod agent_client;
        #[path = "../../../src/git/endpoint/agent_job.rs"]
        mod agent_job;
        #[path = "../../../src/git/endpoint/agent_socket.rs"]
        mod agent_socket;
        mod common;
        #[path = "../support/agent_auth.rs"]
        mod support;
        use agent_job::Job;
        use common::ssh_connection;
        use std::{
            io,
            sync::Arc,
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };
        fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return result;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        #[test]
        fn authenticates_ed25519_and_rsa_sha2_and_transfers_a_usable_connection() {
            for method in ["ssh-ed25519", "rsa-sha2-256", "rsa-sha2-512"] {
                let fixture = support::Fixture::new(method, false);
                let (connection, host) = fixture.prepared(method);
                let path = fixture.path.clone();
                let user = fixture.ssh.user.clone();
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(3)),
                    Duration::from_secs(1),
                    move |control| {
                        agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                            agent_socket::connect(&path, control)
                        })
                    },
                )
                .unwrap();
                let connection = finish(&mut job).unwrap();
                drop(job);
                let mut channel = common::SshChannel::new(
                    connection,
                    common::GitService::UploadPack,
                    fixture.ssh.repository.to_str().unwrap(),
                )
                .unwrap();
                common::open(&mut channel);
                let (_, stderr, status) = common::exchange(&mut channel);
                assert_eq!(status, 0, "{stderr:?}");
                drop(channel);
                fixture.assert_requests(method, 1);
            }
        }

        #[test]
        fn wrong_host_is_rejected_before_agent_access() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let (connection, mut host) = fixture.prepared("ssh-ed25519");
            host[0] ^= 1;
            let path = fixture.path.clone();
            let user = fixture.ssh.user.clone();
            let mut job = Job::start(None, Duration::from_secs(1), move |control| {
                agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                    agent_socket::connect(&path, control)
                })
            })
            .unwrap();
            assert_eq!(
                finish(&mut job).err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
            fixture.no_requests();
            fixture.assert_tcp_closed();
        }
        #[test]
        fn stalled_sign_can_be_cancelled_or_expired_and_does_not_stop_an_active_stream() {
            for timed in [false, true] {
                let fixture = support::Fixture::new("ssh-ed25519", true);
                let (connection, host) = fixture.prepared("ssh-ed25519");
                let path = fixture.path.clone();
                let user = fixture.ssh.user.clone();
                let deadline = timed.then(|| Instant::now() + Duration::from_millis(800));
                let mut job = Job::start(deadline, Duration::from_secs(1), move |control| {
                    agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                        agent_socket::connect(&path, control)
                    })
                })
                .unwrap();
                fixture
                    .signing
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
                if !timed {
                    let mut other = common::SshdFixture::new();
                    let connection = other.session();
                    let mut channel = common::SshChannel::new(
                        connection,
                        common::GitService::UploadPack,
                        other.repository.to_str().unwrap(),
                    )
                    .unwrap();
                    common::open(&mut channel);
                    assert_eq!(common::exchange(&mut channel).2, 0);
                    assert!(
                        job.poll_result(&mut Context::from_waker(Waker::noop()))
                            .is_pending()
                    );
                    job.cancel();
                }
                assert_eq!(
                    finish(&mut job).err().unwrap().kind(),
                    if timed {
                        io::ErrorKind::TimedOut
                    } else {
                        io::ErrorKind::ConnectionAborted
                    }
                );
                fixture.assert_tcp_closed();
                fixture.wait_closed();
                fixture.assert_requests("ssh-ed25519", 1);
                assert!(matches!(
                    job.poll_disposed(&mut Context::from_waker(Waker::noop())),
                    Poll::Ready(Ok(()))
                ));
            }
        }
        #[test]
        fn all_rejected_keys_end_without_signing_or_retry() {
            let fixture = support::Fixture::new("ssh-ed25519", false);
            std::fs::write(fixture.ssh.temp.path().join("authorized_keys"), "").unwrap();
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let path = fixture.path.clone();
            let user = fixture.ssh.user.clone();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |control| {
                    agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                        agent_socket::connect(&path, control)
                    })
                },
            )
            .unwrap();
            assert_eq!(
                finish(&mut job).err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
            fixture.assert_requests("ssh-ed25519", 0);
            fixture.assert_tcp_closed();
        }
        #[test]
        fn signing_callback_contains_rust_panic_and_destroys_connection() {
            use std::io::{Read, Write};
            struct Panics {
                socket: std::os::unix::net::UnixStream,
                writes: usize,
            }
            impl Read for Panics {
                fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                    self.socket.read(b)
                }
            }
            impl Write for Panics {
                fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                    if self.writes >= 5 {
                        panic!("injected signing-channel panic");
                    }
                    let n = self.socket.write(b)?;
                    self.writes += n;
                    Ok(n)
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            impl agent_client::Channel for Panics {
                fn wait(&mut self, _: bool, c: &agent_job::Control) -> io::Result<()> {
                    std::thread::sleep(c.quantum()?);
                    c.check()
                }
            }
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let socket = std::os::unix::net::UnixStream::connect(&fixture.path).unwrap();
            socket.set_nonblocking(true).unwrap();
            let user = fixture.ssh.user.clone();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |control| {
                    agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                        Ok(agent_client::Agent::new(
                            Panics { socket, writes: 0 },
                            control,
                        ))
                    })
                },
            )
            .unwrap();
            assert_eq!(finish(&mut job).err().unwrap().kind(), io::ErrorKind::Other);
            fixture.assert_tcp_closed();
            fixture.wait_closed();
            fixture.assert_requests("ssh-ed25519", 0);
        }

        #[test]
        fn malformed_signature_shape_and_algorithm_fail_without_second_sign_request() {
            for (method, fault) in [("ssh-ed25519", 1), ("ssh-ed25519", 2), ("rsa-sha2-256", 1)] {
                let fixture = support::Fixture::new(method, false);
                fixture
                    .fault
                    .store(fault, std::sync::atomic::Ordering::SeqCst);
                let (connection, host) = fixture.prepared(method);
                let path = fixture.path.clone();
                let user = fixture.ssh.user.clone();
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(3)),
                    Duration::from_secs(1),
                    move |control| {
                        agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                            agent_socket::connect(&path, control)
                        })
                    },
                )
                .unwrap();
                assert_eq!(
                    finish(&mut job).err().unwrap().kind(),
                    io::ErrorKind::InvalidData
                );
                fixture.assert_requests(method, 1);
                fixture.assert_tcp_closed();
            }
        }

        #[test]
        fn native_network_wait_retries_with_stable_state_and_is_cancellable() {
            for cancel in [false, true] {
                let fixture = support::Fixture::new("ssh-ed25519", false);
                let (connection, host) = fixture.prepared("ssh-ed25519");
                let mut paused = common::pause_process_tree(fixture.ssh.child.id());
                let path = fixture.path.clone();
                let user = fixture.ssh.user.clone();
                let mut job = Job::start(None, Duration::from_secs(1), move |control| {
                    agent_auth::authenticate(connection, &user, &host, control.clone(), || {
                        agent_socket::connect(&path, control)
                    })
                })
                .unwrap();
                fixture.listed.recv_timeout(Duration::from_secs(3)).unwrap();
                std::thread::sleep(Duration::from_millis(60));
                assert!(
                    job.poll_result(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                if cancel {
                    job.cancel();
                    assert_eq!(
                        finish(&mut job).err().unwrap().kind(),
                        io::ErrorKind::ConnectionAborted
                    );
                    fixture.assert_requests("ssh-ed25519", 0);
                } else {
                    paused.resume();
                    let connection = finish(&mut job).unwrap();
                    drop(connection);
                    fixture.assert_requests("ssh-ed25519", 1);
                }
                fixture.assert_tcp_closed();
                fixture.wait_closed();
                paused.resume();
            }
        }

        #[test]
        fn native_disconnect_is_terminal_before_a_second_identity() {
            use std::sync::Mutex;
            let fixture = support::Fixture::new("ssh-ed25519", false);
            let (connection, host) = fixture.prepared("ssh-ed25519");
            let mut paused = common::pause_process_tree(fixture.ssh.child.id());
            let breaker = fixture.network_handle();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let observed = calls.clone();
            let path = fixture.path.clone();
            let user = fixture.ssh.user.clone();
            let mut disconnected = false;
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |control| {
                    agent_auth::observed_authenticate(
                        connection,
                        &user,
                        &host,
                        control.clone(),
                        || agent_socket::connect(&path, control),
                        |key, rc| {
                            observed.lock().unwrap().push((key.to_vec(), rc));
                            if !disconnected && rc == libssh2_sys::LIBSSH2_ERROR_EAGAIN {
                                breaker.shutdown(std::net::Shutdown::Both).unwrap();
                                disconnected = true;
                            }
                        },
                    )
                },
            )
            .unwrap();
            assert_eq!(finish(&mut job).err().unwrap().kind(), io::ErrorKind::Other);
            let calls = calls.lock().unwrap();
            assert!(
                calls
                    .iter()
                    .any(|(_, rc)| *rc == libssh2_sys::LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED)
            );
            assert!(
                calls.iter().all(|(key, _)| *key == calls[0].0),
                "native failure must not offer a second identity"
            );
            assert_eq!(
                calls.last().unwrap().1,
                libssh2_sys::LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED
            );
            fixture.assert_requests("ssh-ed25519", 0);
            fixture.assert_tcp_closed();
            fixture.wait_closed();
            paused.resume();
        }

    }
}
