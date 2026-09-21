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
        #[path = "../../../src/git/endpoint/ssh_admission.rs"]
        mod ssh_admission;
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
        use agent_job::Job;
        use common::ssh_channel;
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
        fn native_disconnect_is_io_at_the_setup_boundary_without_retry() {
            use ssh_pool::{Connector, Resource};
            use std::{
                net::{Shutdown, TcpStream},
                sync::atomic::{AtomicUsize, Ordering},
            };
            let f = common::SshdFixture::new();
            let key = Key::ssh(&f.user, "127.0.0.1", f.port);
            let r = Registry::new();
            let entry = load(&r, key.clone(), &f.temp.path().join("client_ed25519")).unwrap();
            let identity = entry.identity();
            let socket = TcpStream::connect(("127.0.0.1", f.port)).unwrap();
            let breaker = socket.try_clone().unwrap();
            let mut conn = common::SshConnection::new(socket).unwrap();
            conn.session().set_timeout(3000);
            conn.session().handshake().unwrap();
            let host = conn.session().host_key().unwrap().0.to_vec();
            let mut known = conn.session().known_hosts().unwrap();
            known
                .read_file(&f.known_hosts, ssh2::KnownHostFileKind::OpenSSH)
                .unwrap();
            assert!(matches!(
                known.check_port("127.0.0.1", f.port, &host),
                ssh2::CheckResult::Match
            ));
            drop(known);
            let mut paused = common::pause_process_tree(f.child.id());
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let mut owner = Some((conn, host, entry.clone()));
            let mut connector = ssh_setup::SetupConnector::new(
                Instant::now(),
                Duration::from_secs(1),
                move |_: &Key, _: &gwz_transport::pool::Identity| {
                    count.fetch_add(1, Ordering::SeqCst);
                    let (conn, host, pin) = owner.take().unwrap();
                    Ok(Box::new(move |c| {
                        ssh_key_auth::authenticate(conn, &host, pin, c)
                            .map(|_| -> ssh_setup::Authenticated { panic!("unexpected auth success") })
                    }) as ssh_setup::Setup)
                },
            );
            let mut resource = connector.start(&key, &identity, Some(3000)).unwrap();
            let mut cx = Context::from_waker(Waker::noop());
            assert!(resource.poll_connected(&mut cx).is_pending());
            std::thread::sleep(Duration::from_millis(60));
            breaker.shutdown(Shutdown::Both).unwrap();
            let until = Instant::now() + Duration::from_secs(3);
            let failure = loop {
                if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
                    break result.unwrap_err();
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(failure.code, gwz_transport::protocol::ErrorCode::Io);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(!entry.proven());
            drop(resource);
            drop(entry);
            assert_eq!(r.usage(), (0, 0));
            paused.resume();
        }
        // Exact wrapper around the auth entry, with a veto so a classifier
        // regression fails safely instead of executing a hostile KDF.
        fn native_dispatch(
            conn: common::SshConnection,
            host: &[u8],
            entry: Arc<ssh_key_snapshot::Entry>,
            c: Arc<agent_job::Control>,
            calls: &std::sync::atomic::AtomicUsize,
            veto: bool,
        ) -> io::Result<ssh_key_auth::Verified> {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if veto {
                return Err(io::ErrorKind::Other.into());
            }
            ssh_key_auth::authenticate(conn, host, entry, c)
        }
        #[test]
        fn valid_extreme_encrypted_containers_never_dispatch_native_auth() {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            use std::{
                process::Command,
                sync::atomic::{AtomicUsize, Ordering},
            };
            let f = common::SshdFixture::new();
            let dir = f.temp.path();
            let good = dir.join("client_ed25519");
            let openssh = dir.join("encrypted-openssh");
            fs::copy(&good, &openssh).unwrap();
            common::run(
                Command::new("ssh-keygen")
                    .args(["-q", "-p", "-a", "1", "-P", "", "-N", "fixture-only", "-f"])
                    .arg(&openssh),
            );
            let text = fs::read_to_string(&openssh).unwrap();
            let mut body = STANDARD
                .decode(
                    text.lines()
                        .filter(|line| !line.starts_with("-----"))
                        .collect::<String>(),
                )
                .unwrap();
            fn field<'a>(b: &'a [u8], at: &mut usize) -> &'a [u8] {
                let n = u32::from_be_bytes(b[*at..*at + 4].try_into().unwrap()) as usize;
                *at += 4;
                let out = &b[*at..*at + n];
                *at += n;
                out
            }
            let mut at = 15;
            assert_eq!(field(&body, &mut at), b"aes256-ctr");
            assert_eq!(field(&body, &mut at), b"bcrypt");
            let options = field(&body, &mut at);
            let mut sub = 0;
            assert!(!field(options, &mut sub).is_empty());
            assert_eq!(options.len(), sub + 4);
            assert_eq!(u32::from_be_bytes(options[sub..].try_into().unwrap()), 1);
            body[at - 4..at].copy_from_slice(&u32::MAX.to_be_bytes());
            fs::write(
                &openssh,
                format!(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
                    STANDARD.encode(body)
                ),
            )
            .unwrap();
            let pem = dir.join("encrypted-pem");
            common::run(
                Command::new("ssh-keygen")
                    .args([
                        "-q",
                        "-t",
                        "rsa",
                        "-b",
                        "2048",
                        "-m",
                        "PEM",
                        "-N",
                        "fixture-only",
                        "-f",
                    ])
                    .arg(&pem),
            );
            assert!(fs::read_to_string(&pem).unwrap().contains("DEK-Info:"));
            let pkcs8 = dir.join("encrypted-pkcs8");
            common::run(
                Command::new("openssl")
                    .args(["pkcs8", "-topk8", "-in"])
                    .arg(&pem)
                    .args([
                        "-passin",
                        "pass:fixture-only",
                        "-passout",
                        "pass:fixture-only",
                        "-iter",
                        "65536",
                        "-out",
                    ])
                    .arg(&pkcs8),
            );
            let text = fs::read_to_string(&pkcs8).unwrap();
            assert!(text.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----"));
            let mut body = STANDARD
                .decode(
                    text.lines()
                        .filter(|line| !line.starts_with("-----"))
                        .collect::<String>(),
                )
                .unwrap();
            // Navigate DER parameters, never search the random ciphertext.
            fn tlv(b: &[u8], at: &mut usize, tag: u8) -> std::ops::Range<usize> {
                assert_eq!(b[*at], tag);
                let length = b[*at + 1];
                *at += 2;
                let size = if length & 128 == 0 {
                    length as usize
                } else {
                    let count = (length & 127) as usize;
                    let mut n = 0;
                    for byte in &b[*at..*at + count] {
                        n = (n << 8) | *byte as usize;
                    }
                    *at += count;
                    n
                };
                let value = *at..*at + size;
                assert!(value.end <= b.len());
                *at = value.end;
                value
            }
            fn sequence(b: &[u8], at: &mut usize) {
                *at = tlv(b, at, 0x30).start;
            }
            let mut at = 0;
            sequence(&body, &mut at); // EncryptedPrivateKeyInfo
            sequence(&body, &mut at); // AlgorithmIdentifier
            let pbes2 = tlv(&body, &mut at, 6);
            assert_eq!(&body[pbes2], &[42, 134, 72, 134, 247, 13, 1, 5, 13]);
            sequence(&body, &mut at); // PBES2 parameters
            sequence(&body, &mut at); // KDF AlgorithmIdentifier
            let pbkdf2 = tlv(&body, &mut at, 6);
            assert_eq!(&body[pbkdf2], &[42, 134, 72, 134, 247, 13, 1, 5, 12]);
            sequence(&body, &mut at); // PBKDF2 parameters
            let _salt = tlv(&body, &mut at, 4);
            let iterations = tlv(&body, &mut at, 2);
            assert_eq!(&body[iterations.clone()], &[1, 0, 0]);
            body[iterations].copy_from_slice(&[127, 255, 255]); // 8,388,607; DER lengths unchanged.
            fs::write(
                &pkcs8,
                format!(
                    "-----BEGIN ENCRYPTED PRIVATE KEY-----\n{}\n-----END ENCRYPTED PRIVATE KEY-----\n",
                    STANDARD.encode(body)
                ),
            )
            .unwrap();
            let r = Registry::new();
            for (path, veto) in [(openssh, true), (pem, true), (pkcs8, true), (good, false)] {
                let calls = Arc::new(AtomicUsize::new(0));
                let count = calls.clone();
                let key = Key::ssh(&f.user, "127.0.0.1", f.port);
                let known = f.known_hosts.clone();
                let result = load(&r, key.clone(), &path).and_then(|entry| {
                    finish(&mut Job::start(
                        Some(Instant::now() + Duration::from_secs(3)),
                        Duration::from_secs(1),
                        move |c| {
                            let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                            native_dispatch(conn, &host, entry, c, &count, veto)
                        },
                    )?)
                });
                if veto {
                    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
                    assert_eq!(calls.load(Ordering::SeqCst), 0);
                } else {
                    let verified = result.unwrap();
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    drop(verified);
                }
                assert_eq!(r.usage(), (0, 0));
            }
        }
    }
}
