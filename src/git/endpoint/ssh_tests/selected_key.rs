use crate::git::endpoint::agent_job;
use crate::git::endpoint::ssh_fixture as common;
use crate::git::endpoint::ssh_key_auth;
use crate::git::endpoint::ssh_key_snapshot;
use crate::git::endpoint::ssh_network;
use crate::git::endpoint::ssh_pool;
use crate::git::endpoint::ssh_setup;
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
    let job = Job::start_isolated(None, Duration::from_secs(1), move |c| {
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
fn native_auth_uses_snapshot_after_path_replacement_and_releases_its_pin() {
    for handoff in [true, false] {
        let f = common::SshdFixture::new();
        let key = Key::ssh(&f.user, "127.0.0.1", f.port);
        let r = Registry::new();
        let path = f.temp.path().join("client_ed25519");
        let entry = load(&r, key.clone(), &path).unwrap();
        fs::write(&path, "replaced").unwrap();
        let known = f.known_hosts.clone();
        let pin = entry.clone();
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(5)),
            Duration::from_secs(1),
            move |c| {
                let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {})
            },
        )
        .unwrap();
        let verified = finish(&mut job).unwrap();
        if !handoff {
            // A setup that ends before its handoff drops the connection, then its pin.
            drop(verified);
        } else {
            let (conn, pin) = verified.into_parts();
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
/// A selected key's login waits for the server's replies, not for a
/// quantum (20 ms) at each: it takes two exchanges, which a slept
/// quantum makes 40 ms at the least (47 ms measured); awaiting the
/// server takes about 18 ms, most of it signing and sshd's checks.
#[test]
fn a_selected_key_login_waits_for_the_server_not_for_a_quantum() {
    let f = common::SshdFixture::new();
    let key = Key::ssh(&f.user, "127.0.0.1", f.port);
    let r = Registry::new();
    let path = f.temp.path().join("client_ed25519");
    let entry = load(&r, key.clone(), &path).unwrap();
    let mut fastest = Duration::MAX;
    for _ in 0..5 {
        let (key, known, pin) = (key.clone(), f.known_hosts.clone(), entry.clone());
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(5)),
            Duration::from_secs(1),
            move |c| {
                let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                let started = Instant::now();
                let verified =
                    ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {})?;
                drop(verified);
                Ok(started.elapsed())
            },
        )
        .unwrap();
        fastest = fastest.min(finish(&mut job).unwrap());
    }
    println!("fastest selected-key login: {fastest:?}");
    assert!(
        fastest < Duration::from_millis(35),
        "no login took under {fastest:?}: a quantum is slept, not the server awaited"
    );
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
    let mut job = Job::start_isolated(
        Some(Instant::now() + Duration::from_secs(3)),
        Duration::from_secs(1),
        move |c| {
            let (conn, host) = ssh_network::establish(&key, &known, &c)?;
            ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {})
        },
    )
    .unwrap();
    assert!(finish(&mut job).is_err());
    drop(entry);
    assert_eq!(r.usage(), (0, 0));
}

#[test]
fn parallel_same_material_interns_one_candidate() {
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
    assert!(pins.iter().all(|p| Arc::ptr_eq(p, &pins[0])));
    assert_eq!(r.usage().0, 1);
    drop(pins);
    assert_eq!(r.usage(), (0, 0));
}
#[test]
fn a_pipe_or_device_path_does_not_hang_the_key_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = common::special_file(dir.path());
    let start = Instant::now();
    assert_eq!(
        load(&Registry::new(), Key::ssh("u", "h", 22), &path)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}
// Creating a symbolic link needs a privilege a Windows logon may not hold.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn symlinks_follow_the_current_regular_file() {
            use std::os::unix::fs::symlink;
            let dir = tempfile::tempdir().unwrap();
            let r = Registry::new();
            let key = Key::ssh("u", "h", 22);
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
    }
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
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(5)),
            Duration::from_secs(1),
            move |c| {
                let (conn, host) = ssh_network::establish(&key, &known, &c)?;
                ssh_key_auth::authenticate_reporting(conn, &host, entry, c, || {}, || {})
            },
        )
        .unwrap();
        let outcome = finish(&mut job);
        if !native_reads(format) {
            // Refused before libssh2 sees the key, and quickly: its CNG backend blocks on a form it cannot read.
            assert!(outcome.is_err(), "{format}");
            drop(outcome);
            assert_eq!(r.usage(), (0, 0));
            continue;
        }
        let (conn, pin) = outcome.unwrap().into_parts();
        drop(conn);
        drop(pin);
        assert_eq!(r.usage(), (0, 0));
    }
}
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// libssh2 on OpenSSL reads every form the row makes.
        fn native_reads(_format: &str) -> bool {
            true
        }
    } else {
        /// libssh2 on Windows (CNG) reads only the traditional PEM form of an RSA key.
        fn native_reads(format: &str) -> bool {
            format == "pem"
        }
    }
}
#[test]
fn wrong_host_and_rejected_selected_key_do_not_fall_back() {
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
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(3)),
            Duration::from_secs(1),
            move |c| {
                let (conn, mut host) = ssh_network::establish(&key, &known, &c)?;
                if wrong_host {
                    host[0] ^= 1;
                }
                ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {})
            },
        )
        .unwrap();
        assert_eq!(
            finish(&mut job).err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
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
            &mut Job::start_isolated(None, Duration::from_secs(1), move |c| {
                ssh_network::establish(&key, &known, &c)
            })
            .unwrap(),
        )
        .unwrap();
        let mut paused = common::pause_process_tree(f.child.id());
        let mut job = Job::start_isolated(
            timed.then(|| Instant::now() + Duration::from_millis(150)),
            Duration::from_secs(1),
            move |c| ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {}),
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
    let (mut conn, breaker) = common::handshaken(f.port, 3000).unwrap();
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
    let mut connector = ssh_setup::SetupConnector::isolated(
        Instant::now(),
        Duration::from_secs(1),
        move |_: &Key, _: &gwz_transport::pool::Identity, _| {
            count.fetch_add(1, Ordering::SeqCst);
            let (conn, host, pin) = owner.take().unwrap();
            Ok(Box::new(move |c| {
                ssh_key_auth::authenticate_reporting(conn, &host, pin, c, || {}, || {})
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
    ssh_key_auth::authenticate_reporting(conn, host, entry, c, || {}, || {})
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
            finish(&mut Job::start_isolated(
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
