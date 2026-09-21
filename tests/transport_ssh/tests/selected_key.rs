#![allow(dead_code, unused_imports)]
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod common;
        use common::ssh_connection;
        #[path = "../../../src/git/endpoint/agent_job.rs"]
        mod agent_job;
        #[path = "../../../src/git/endpoint/ssh_key_auth.rs"]
        mod ssh_key_auth;
        #[path = "../../../src/git/endpoint/ssh_key_container.rs"]
        mod ssh_key_container;
        #[path = "../../../src/git/endpoint/ssh_key_snapshot.rs"]
        mod ssh_key_snapshot;
        #[path = "../../../src/git/endpoint/ssh_network.rs"]
        mod ssh_network;
        use agent_job::Job;
        use gwz_transport::pool::Key;
        use ssh_key_snapshot::Registry;
        use std::{
            fs, io,
            path::Path,
            sync::{Arc, mpsc},
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };
        const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nAQID\n-----END RSA PRIVATE KEY-----\n";
        fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
            let until = Instant::now() + Duration::from_secs(8);
            loop {
                if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return result;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn load(r: &Registry, key: Key, path: &Path) -> io::Result<Arc<ssh_key_snapshot::Entry>> {
            let loaded = finish(&mut r.start(key, path.into(), None, Duration::from_secs(1))?)?;
            r.intern(loaded, || Ok(()))
        }
        #[test]
        fn candidates_share_exact_material_and_key_but_never_recycle_tokens() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("key");
            fs::write(&path, PEM).unwrap();
            let r = Registry::new();
            let key = Key::ssh("u", "h", 22);
            let a = load(&r, key.clone(), &path).unwrap();
            let b = load(&r, key.clone(), &path).unwrap();
            assert!(Arc::ptr_eq(&a, &b));
            assert!(!a.proven());
            assert_eq!(r.usage().0, 1);
            let other = load(&r, Key::ssh("u", "h", 23), &path).unwrap();
            assert_ne!(a.identity(), other.identity());
            fs::write(&path, PEM.replace("AQID", "BAUG")).unwrap();
            let changed = load(&r, key.clone(), &path).unwrap();
            assert_ne!(a.identity(), changed.identity());
            fs::write(
                &path,
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAQID\n-----END ENCRYPTED PRIVATE KEY-----",
            )
            .unwrap();
            assert_eq!(
                load(&r, key.clone(), &path).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
            fs::remove_file(&path).unwrap();
            assert!(load(&r, key.clone(), &path).is_err());
            let id = a.identity();
            drop((a, b, other, changed));
            assert_eq!(r.usage(), (0, 0));
            fs::write(&path, PEM).unwrap();
            assert_ne!(id, load(&r, key, &path).unwrap().identity());
        }
        #[test]
        fn quota_is_reserved_before_io_and_includes_cancelled_unjoined_reads() {
            let r = Registry::with_limits(1, 2_000_000, 1024);
            let permit = r.reserve().unwrap();
            assert_eq!(r.usage(), (1, 1281));
            assert_eq!(r.reserve().err().unwrap().kind(), io::ErrorKind::WouldBlock);
            let (started, seen) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let job = Job::start(None, Duration::from_secs(1), move |c| {
                permit.test_read(Key::ssh("u", "h", 22), &c, |buf, _| {
                    started.send(()).unwrap();
                    blocked.recv().unwrap();
                    buf[..PEM.len()].copy_from_slice(PEM.as_bytes());
                    Ok(PEM.len())
                })
            })
            .unwrap();
            seen.recv_timeout(Duration::from_secs(1)).unwrap();
            job.cancel();
            drop(job);
            assert_eq!(r.usage(), (1, 1281));
            assert!(r.reserve().is_err());
            release.send(()).unwrap();
            let until = Instant::now() + Duration::from_secs(3);
            while r.usage().0 != 0 {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            let bytes = Registry::with_limits(64, 1280, 1024);
            assert!(bytes.reserve().is_err());
            let slots = Registry::with_limits(64, 100_000, 128);
            let pins: Vec<_> = (0..64).map(|_| slots.reserve().unwrap()).collect();
            assert!(slots.reserve().is_err());
            drop(pins);
            assert_eq!(slots.usage(), (0, 0));
        }
        #[test]
        fn admission_rejects_nonregular_oversize_invalid_and_stale_handoffs() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("key");
            let r = Registry::new();
            let key = Key::ssh("u", "h", 22);
            for bytes in [
                vec![],
                vec![0; 100],
                vec![255; 100],
                vec![b'a'; (1 << 20) + 1],
            ] {
                fs::write(&path, bytes).unwrap();
                assert_eq!(
                    load(&r, key.clone(), &path).err().unwrap().kind(),
                    io::ErrorKind::InvalidInput
                );
                assert_eq!(r.usage(), (0, 0));
            }
            assert!(load(&r, key.clone(), dir.path()).is_err());
            fs::write(&path, PEM).unwrap();
            let loaded = finish(
                &mut r
                    .start(key.clone(), path.clone(), None, Duration::from_secs(1))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                r.intern(loaded, || Err(io::ErrorKind::TimedOut.into()))
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::TimedOut
            );
            assert_eq!(r.usage(), (0, 0));
            let loaded = finish(&mut r.start(key, path, None, Duration::from_secs(1)).unwrap()).unwrap();
            assert!(Registry::new().intern(loaded, || Ok(())).is_err());
            assert_eq!(r.usage(), (0, 0));
        }
        #[test]
        fn native_auth_uses_snapshot_after_path_replacement_and_promotes_only_live_handoff() {
            for cancel in [false, true] {
                let f = common::SshdFixture::new();
                let key = Key::ssh(&f.user, "127.0.0.1", f.port);
                let r = Registry::new();
                let path = f.temp.path().join("client_ed25519");
                let entry = load(&r, key.clone(), &path).unwrap();
                fs::write(&path, "replaced").unwrap();
                let known = f.known_hosts.clone();
                let pin = entry.clone();
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(5)),
                    Duration::from_secs(1),
                    move |c| {
                        let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                        ssh_key_auth::authenticate(conn, &host, pin, c)
                    },
                )
                .unwrap();
                let verified = finish(&mut job).unwrap();
                assert!(!entry.proven());
                if cancel {
                    assert_eq!(
                        verified
                            .publish(|| Err(io::ErrorKind::ConnectionAborted.into()))
                            .err()
                            .unwrap()
                            .kind(),
                        io::ErrorKind::ConnectionAborted
                    );
                    assert!(!entry.proven());
                } else {
                    let (conn, pin) = verified.publish(|| Ok(())).unwrap();
                    assert!(entry.proven());
                    assert!(Arc::ptr_eq(&pin, &entry));
                    let mut channel = common::SshChannel::new(
                        conn,
                        common::GitService::UploadPack,
                        f.repository.to_str().unwrap(),
                    )
                    .unwrap();
                    common::open(&mut channel);
                    assert_eq!(common::exchange(&mut channel).2, 0);
                    drop(channel);
                    drop(pin);
                }
                drop(entry);
                drop(job);
                assert_eq!(r.usage(), (0, 0));
            }
        }
        #[test]
        fn malformed_unencrypted_material_is_not_native_proof() {
            let f = common::SshdFixture::new();
            let key = Key::ssh(&f.user, "127.0.0.1", f.port);
            let r = Registry::new();
            let path = f.temp.path().join("bad");
            fs::write(&path, PEM).unwrap();
            let entry = load(&r, key.clone(), &path).unwrap();
            let pin = entry.clone();
            let known = f.known_hosts.clone();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(3)),
                Duration::from_secs(1),
                move |c| {
                    let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                    ssh_key_auth::authenticate(conn, &host, pin, c)
                },
            )
            .unwrap();
            assert!(finish(&mut job).is_err());
            assert!(!entry.proven());
            drop(entry);
            assert_eq!(r.usage(), (0, 0));
        }

        #[test]
        fn parallel_same_material_interns_one_unproven_candidate() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("key");
            fs::write(&path, PEM).unwrap();
            let r = Registry::new();
            let barrier = Arc::new(std::sync::Barrier::new(8));
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    let r = r.clone();
                    let path = path.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        let loaded = finish(
                            &mut r
                                .start(Key::ssh("u", "h", 22), path, None, Duration::from_secs(1))
                                .unwrap(),
                        )
                        .unwrap();
                        barrier.wait();
                        r.intern(loaded, || Ok(())).unwrap()
                    })
                })
                .collect();
            let pins: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
            assert!(pins.iter().all(|p| Arc::ptr_eq(p, &pins[0]) && !p.proven()));
            assert_eq!(r.usage().0, 1);
            drop(pins);
            assert_eq!(r.usage(), (0, 0));
        }
        #[test]
        fn fifo_does_not_hang_and_symlinks_follow_current_regular_file() {
            use std::{ffi::CString, os::unix::fs::symlink};
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("pipe");
            let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            let r = Registry::new();
            let key = Key::ssh("u", "h", 22);
            assert_eq!(
                load(&r, key.clone(), &path).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
            let regular = dir.path().join("regular");
            fs::write(&regular, PEM).unwrap();
            let link = dir.path().join("link");
            symlink(&regular, &link).unwrap();
            let pin = load(&r, key.clone(), &link).unwrap();
            fs::remove_file(regular).unwrap();
            assert!(load(&r, key, &link).is_err());
            drop(pin);
            assert_eq!(r.usage(), (0, 0));
        }
        #[test]
        fn native_rsa_openssh_pem_and_pkcs8_authenticate() {
            use std::process::Command;
            for format in ["openssh", "pem", "pkcs8"] {
                let f = common::SshdFixture::new();
                let path = f.temp.path().join("rsa");
                common::run(
                    Command::new("ssh-keygen")
                        .args(["-q", "-t", "rsa", "-b", "2048", "-N", "", "-f"])
                        .arg(&path),
                );
                fs::copy(
                    path.with_extension("pub"),
                    f.temp.path().join("authorized_keys"),
                )
                .unwrap();
                if format != "openssh" {
                    common::run(
                        Command::new("ssh-keygen")
                            .args(["-q", "-p", "-m", "PEM", "-P", "", "-N", "", "-f"])
                            .arg(&path),
                    );
                }
                let selected = if format == "pkcs8" {
                    let output = f.temp.path().join("pkcs8");
                    common::run(
                        Command::new("openssl")
                            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
                            .arg(&path)
                            .arg("-out")
                            .arg(&output),
                    );
                    output
                } else {
                    path
                };
                let r = Registry::new();
                let key = Key::ssh(&f.user, "127.0.0.1", f.port);
                let entry = load(&r, key.clone(), &selected).unwrap();
                let known = f.known_hosts.clone();
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(5)),
                    Duration::from_secs(1),
                    move |c| {
                        let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                        ssh_key_auth::authenticate(conn, &host, entry, c)
                    },
                )
                .unwrap();
                let (conn, pin) = finish(&mut job).unwrap().publish(|| Ok(())).unwrap();
                assert!(pin.proven());
                drop(conn);
                drop(pin);
                assert_eq!(r.usage(), (0, 0));
            }
        }
        #[test]
        fn wrong_host_and_rejected_selected_key_do_not_promote_or_fallback() {
            for wrong_host in [false, true] {
                let f = common::SshdFixture::new();
                let key = Key::ssh(&f.user, "127.0.0.1", f.port);
                let r = Registry::new();
                let entry = load(&r, key.clone(), &f.temp.path().join("client_ed25519")).unwrap();
                let pin = entry.clone();
                let known = f.known_hosts.clone();
                if !wrong_host {
                    fs::write(f.temp.path().join("authorized_keys"), "").unwrap();
                }
                let mut job = Job::start(
                    Some(Instant::now() + Duration::from_secs(3)),
                    Duration::from_secs(1),
                    move |c| {
                        let (conn, mut host) = ssh_network::establish(&key, &known, &c)?;
                        if wrong_host {
                            host[0] ^= 1;
                        }
                        ssh_key_auth::authenticate(conn, &host, pin, c)
                    },
                )
                .unwrap();
                assert_eq!(
                    finish(&mut job).err().unwrap().kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert!(!entry.proven());
                drop(entry);
                assert_eq!(r.usage(), (0, 0));
            }
        }
        #[test]
        fn paused_native_auth_preserves_pin_and_honors_cancellation_and_deadline() {
            for timed in [false, true] {
                let f = common::SshdFixture::new();
                let key = Key::ssh(&f.user, "127.0.0.1", f.port);
                let r = Registry::new();
                let entry = load(&r, key.clone(), &f.temp.path().join("client_ed25519")).unwrap();
                let pin = entry.clone();
                let known = f.known_hosts.clone();
                let (conn, host) = finish(
                    &mut Job::start(None, Duration::from_secs(1), move |c| {
                        ssh_network::establish(&key, &known, &c)
                    })
                    .unwrap(),
                )
                .unwrap();
                let mut paused = common::pause_process_tree(f.child.id());
                let mut job = Job::start(
                    timed.then(|| Instant::now() + Duration::from_millis(150)),
                    Duration::from_secs(1),
                    move |c| ssh_key_auth::authenticate(conn, &host, pin, c),
                )
                .unwrap();
                std::thread::sleep(Duration::from_millis(50));
                assert!(
                    job.poll_result(&mut Context::from_waker(Waker::noop()))
                        .is_pending()
                );
                assert_eq!(r.usage().0, 1);
                if !timed {
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
                assert!(!entry.proven());
                drop(entry);
                assert_eq!(r.usage(), (0, 0));
                paused.resume();
            }
        }
        #[test]
        fn encrypted_workfactor_payload_never_reaches_native_stage() {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            let mut bytes = b"openssh-key-v1\0".to_vec();
            for value in [b"aes256-ctr".as_slice(), b"bcrypt", &[255; 64]] {
                bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
                bytes.extend_from_slice(value);
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("key");
            fs::write(
                &path,
                format!(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
                    STANDARD.encode(bytes)
                ),
            )
            .unwrap();
            let r = Registry::new();
            let mut native_calls = 0;
            let result = load(&r, Key::ssh("u", "h", 22), &path).map(|_| {
                native_calls += 1;
            });
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(native_calls, 0);
            assert_eq!(r.usage(), (0, 0));
        }
    }
}
