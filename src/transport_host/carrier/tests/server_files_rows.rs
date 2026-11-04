//! The server's listener files (step 2.3f): the `ws-<key>` names, the lock and its record, the file rule, the
//! stale-socket test, the clean exit and the link check, over real files and sockets.
use super::super::CarrierLimits;
use super::super::local::{
    PeerPolicy, RuntimeDir,
    server_files::{
        self, Claim, FileFault, FilesRefusal, IdentityKey, Keys, Record, Recorded, ServerKey,
        ServerListener, ServerRecord,
    },
};
use super::server_files_fork_rows::alone;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// A private directory for the server's files, and a workspace root.
pub(super) struct Fixture {
    runtime: RuntimeDir,
    dir: PathBuf,
    root: PathBuf,
    _guards: [tempfile::TempDir; 2],
}

pub(super) fn fixture() -> Fixture {
    let files = tempfile::Builder::new()
        .prefix("gwz")
        .tempdir()
        .expect("tempdir");
    let workspace = tempfile::Builder::new()
        .prefix("gwz-ws")
        .tempdir()
        .expect("tempdir");
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            std::fs::set_permissions(files.path(), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        } else {
            // Windows: the directory keeps the ACL it inherits; the file rule judges each file's owner.
        }
    }
    Fixture {
        runtime: RuntimeDir::at(files.path()).expect("a private directory"),
        dir: files.path().to_path_buf(),
        root: workspace.path().to_path_buf(),
        _guards: [files, workspace],
    }
}

impl Fixture {
    fn key(&self) -> ServerKey {
        server_key(&self.root)
    }

    fn lock(&self) -> PathBuf {
        self.dir.join(self.key().lock_name())
    }

    pub(super) fn identity_lock(&self) -> PathBuf {
        self.dir
            .join(Keys::of_root(&self.root).unwrap().identity.lock_name())
    }

    pub(super) fn claim(&self) -> Result<Claim, FilesRefusal> {
        server_files::claim_within(&self.runtime, &self.root, None, BOUND)
    }

    pub(super) fn start(&self) -> ServerListener {
        self.claim()
            .expect("claim")
            .bind(policy(), CarrierLimits::DEFAULT)
            .expect("bind")
    }

    fn record(&self) -> Record {
        server_files::read_lock_record(&self.dir, &self.key().lock_name()).expect("read the record")
    }
}

fn server_key(root: &Path) -> ServerKey {
    Keys::of_root(root).unwrap().server
}

/// The stale test's connection bound in the rows: short, so that a row that waits for it is quick.
const BOUND: Duration = Duration::from_millis(300);

fn policy() -> PeerPolicy {
    PeerPolicy::current_user().unwrap()
}

fn address_path(server: &ServerListener) -> PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            server.address().path().to_path_buf()
        } else {
            PathBuf::from(server.address().pipe_name())
        }
    }
}

#[test]
fn a_key_is_ws_and_16_hex_digits_of_the_canonical_root_path_and_two_roots_differ() {
    let one = fixture();
    let other = fixture();
    let name = one.key().socket_name();
    assert_eq!(name.len(), 19, "{name}");
    assert!(name.starts_with("ws-"), "{name}");
    assert!(
        name[3..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{name}"
    );
    assert_eq!(one.key().lock_name(), format!("{name}.lock"));
    assert_eq!(one.key().log_name(), format!("{name}.log"));
    assert_ne!(one.key(), other.key());
    let missing = one.root.join("never-created");
    assert!(matches!(
        server_files::claim(&one.runtime, &missing, None),
        Err(FilesRefusal::NoIdentity { .. })
    ));
}

/// Operator decision 2026-11-03 (2.3f round 2 P2-1, option (a)): the socket name is the hash of the canonical path of
/// the root (split doc §1 row 25), not of its filesystem identity.
#[test]
fn the_socket_name_is_the_hash_of_the_canonical_root_path() {
    use sha2::{Digest, Sha256};
    let fixture = fixture();
    let canonical = std::fs::canonicalize(&fixture.root).unwrap();
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(fixture.key().socket_name(), format!("ws-{}", &hex[..16]));
}

/// The split doc's definition, as a fixed answer: SHA-256 of `/work/ws` is `fce99486135bc1ea...`.
#[test]
fn the_socket_name_of_a_known_path_is_its_sha256() {
    let key = ServerKey::of_canonical(Path::new("/work/ws"));
    assert_eq!(key.socket_name(), "ws-fce99486135bc1ea");
    assert_eq!(key.lock_name(), "ws-fce99486135bc1ea.lock");
}

#[test]
fn a_second_start_finds_the_first_through_its_record() {
    let fixture = fixture();
    let first = fixture.start();
    match fixture.claim() {
        Err(FilesRefusal::Held {
            lock,
            record: Record::Complete(record),
        }) => {
            // The identity lock is taken first, so it is the one a second start finds held.
            assert_eq!(lock, fixture.identity_lock());
            assert_eq!(record.pid, std::process::id());
            assert_eq!(record.address, address_path(&first));
            assert_eq!(record.root, std::fs::canonicalize(&fixture.root).unwrap());
            assert_eq!(record.log, fixture.dir.join(fixture.key().log_name()));
        }
        Err(other) => panic!("the second start should find the first: {other}"),
        Ok(_) => panic!("the second start should find the first"),
    }
    first.close();
}

#[test]
fn a_clean_exit_removes_the_socket_clears_the_record_and_releases_the_lock() {
    alone!(|| {
        let fixture = fixture();
        let server = fixture.start();
        let address = address_path(&server);
        assert!(matches!(fixture.record(), Record::Complete(_)));
        server.close();
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                assert!(std::fs::symlink_metadata(&address).is_err(), "the socket outlived the server");
            } else {
                let _ = address;
            }
        }
        assert_eq!(
            fixture.record(),
            Record::None,
            "the record outlived the server"
        );
        let identity_name = Keys::of_root(&fixture.root).unwrap().identity.lock_name();
        assert_eq!(
            server_files::read_lock_record(&fixture.dir, &identity_name).unwrap(),
            Record::None,
            "the identity record outlived the server"
        );
        assert!(fixture.lock().is_file(), "the lock file stays");
        assert!(
            fixture.identity_lock().is_file(),
            "the identity lock file stays"
        );
        let next = fixture.claim().expect("the lock is released");
        assert_eq!(next.previous(), &Record::None);
    });
}

#[test]
fn a_record_round_trips_and_a_partial_one_reads_as_a_starting_holder() {
    let record = ServerRecord {
        pid: 4242,
        address: PathBuf::from("/run/user/1/gwz/ws-0123456789abcdef"),
        root: PathBuf::from("/work/a %41 name\nwith a newline"),
        log: PathBuf::from("/var/log/ws.log"),
    };
    let bytes = record.encode().unwrap();
    assert_eq!(Record::parse(&bytes), Record::Complete(record.clone()));
    assert_eq!(
        bytes.iter().filter(|byte| **byte == b'\n').count(),
        6,
        "one field a line"
    );
    assert_eq!(Record::parse(&bytes[..bytes.len() - 4]), Record::Incomplete);
    assert_eq!(Record::parse(b""), Record::None);
    assert_eq!(Record::parse(b"not a record\n"), Record::Incomplete);
    assert!(Record::parse(&bytes).names(Path::new("/run/user/1/gwz/ws-0123456789abcdef")));
    assert!(!Record::parse(&bytes).names(Path::new("/run/user/1/gwz/ws-0123456789abcdee")));
}

/// A file of the user's that a link at the lock or the log could reach: 0600 on Unix, the user's on Windows.
fn private_file(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        } else {
            // Windows: the file is the user's, with the ACL it inherits.
        }
    }
}

/// State S-1: a hard link is a second name of another file, so the file rule refuses it before anything is written.
#[test]
fn a_hard_link_planted_at_the_lock_or_the_log_is_refused_and_its_target_untouched() {
    alone!(|| {
        let fixture = fixture();
        let target = fixture.root.join("id_ed25519");
        private_file(&target, b"PRIVATE KEY BYTES");
        std::fs::hard_link(&target, fixture.lock()).unwrap();
        let refused = fixture.claim().map(|claim| {
            claim
                .bind(policy(), CarrierLimits::DEFAULT)
                .map(ServerListener::close)
        });
        assert!(
            matches!(refused, Err(FilesRefusal::File { ref path, fault: FileFault::HardLink }) if *path == fixture.lock()),
            "a hard link at the lock was used: {:?}",
            refused.err()
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"PRIVATE KEY BYTES",
            "the lock's link target was written"
        );
        std::fs::remove_file(fixture.lock()).unwrap();
        std::fs::hard_link(&target, fixture.dir.join(fixture.key().log_name())).unwrap();
        let claim = fixture.claim().expect("claim");
        assert!(
            matches!(
                claim.open_log(),
                Err(FilesRefusal::File {
                    fault: FileFault::HardLink,
                    ..
                })
            ),
            "a hard link at the log was opened"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"PRIVATE KEY BYTES",
            "the log's link target was written"
        );
    });
}

/// Code P2-3 / State S-2: the identity key hashes the whole 128-bit file ID (Windows `FileIdInfo`), so two IDs that
/// share their low 64 bits (ReFS) are two keys.
#[test]
fn an_identity_key_counts_the_high_half_of_a_128_bit_file_id() {
    use crate::workspace_identity::WorkspaceIdentity;
    let low = WorkspaceIdentity {
        volume: 7,
        file: 0x0123_4567_89ab_cdef,
    };
    let high = WorkspaceIdentity {
        volume: 7,
        file: (1u128 << 64) | 0x0123_4567_89ab_cdef,
    };
    assert_ne!(
        IdentityKey::of_identity(low),
        IdentityKey::of_identity(high)
    );
    assert_ne!(
        IdentityKey::of_identity(low),
        IdentityKey::of_identity(WorkspaceIdentity {
            volume: 8,
            file: low.file
        })
    );
    assert!(IdentityKey::of_identity(low).lock_name().starts_with("id-"));
}

/// A logged step of an order: it records its name when it runs (a closure) or when it drops (a value).
struct Step<'a>(&'a std::cell::RefCell<Vec<&'static str>>, &'static str);

impl Drop for Step<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().push(self.1);
    }
}

/// Code P3-1 / State S-3 and round 2 P2-1: the clean exit removes the socket, clears the workspace record and releases
/// that lock, then clears the identity record and releases the identity lock last (the reverse of the order taken).
#[test]
fn the_clean_exit_runs_socket_record_lock_identity_record_identity_lock_in_that_order() {
    let log = std::cell::RefCell::new(Vec::new());
    server_files::exit_in_order(
        Step(&log, "socket"),
        |_: &Step| log.borrow_mut().push("record"),
        Step(&log, "lock"),
        |_: &Step| log.borrow_mut().push("identity record"),
        Step(&log, "identity lock"),
    );
    assert_eq!(
        *log.borrow(),
        [
            "socket",
            "record",
            "lock",
            "identity record",
            "identity lock"
        ]
    );
}

/// Two directories' worth of workspace roots that one directory's two spellings stand in for: `keys` of `root`, with
/// the identity key of `identity` (a bind mount, or an inode number reused): the seam of `claim_keyed`, no real mount
/// and no real inode-reuse race.
fn keys_as(root: &Path, identity: u64) -> Keys {
    use crate::workspace_identity::WorkspaceIdentity;
    Keys {
        identity: IdentityKey::of_identity(WorkspaceIdentity {
            volume: 1,
            file: u128::from(identity),
        }),
        ..Keys::of_root(root).unwrap()
    }
}

fn workspace() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("gwz-ws")
        .tempdir()
        .expect("tempdir")
}

fn start_keyed(fixture: &Fixture, keys: Keys) -> ServerListener {
    server_files::claim_keyed(&fixture.runtime, keys, None, BOUND)
        .expect("claim")
        .bind(policy(), CarrierLimits::DEFAULT)
        .expect("bind")
}

/// Round 2 P2-1 (a): two spellings of one directory (two canonical paths, one identity) are two `ws-<key>` names but one
/// server: the second start refuses as held on the identity lock and names the first server's address, before it
/// creates anything of its own.
#[test]
fn a_second_server_through_another_spelling_of_a_running_servers_directory_refuses_naming_the_first()
 {
    let fixture = fixture();
    let spelling = workspace();
    let first = start_keyed(&fixture, keys_as(&fixture.root, 7));
    let other = keys_as(spelling.path(), 7);
    assert_ne!(other.server, fixture.key(), "two spellings, two names");
    let other_lock = fixture.dir.join(other.server.lock_name());
    match server_files::claim_keyed(&fixture.runtime, other, None, BOUND) {
        Err(FilesRefusal::Held {
            lock,
            record: Record::Complete(record),
        }) => {
            assert_eq!(
                lock,
                fixture
                    .dir
                    .join(IdentityKey::of_identity(identity_of(7)).lock_name())
            );
            assert_eq!(record.address, address_path(&first));
        }
        Err(other) => panic!("the second spelling should find the first: {other}"),
        Ok(_) => panic!("the second spelling started a second server"),
    }
    assert!(
        !other_lock.exists(),
        "the identity lock is taken before the workspace lock"
    );
    first.close();
}

fn identity_of(file: u64) -> crate::workspace_identity::WorkspaceIdentity {
    crate::workspace_identity::WorkspaceIdentity {
        volume: 1,
        file: u128::from(file),
    }
}

/// Round 2 P2-1 (a): the identity lock carries no grant. A root that reuses a gone server's identity gets its own
/// address (its own canonical path's name), the lock it finds free holds no record of the first, and a grant naming the
/// first's address does not match it.
#[test]
fn a_root_that_reuses_a_gone_servers_identity_gets_its_own_address_and_inherits_nothing() {
    alone!(|| {
        let fixture = fixture();
        let reuse = workspace();
        let first = start_keyed(&fixture, keys_as(&fixture.root, 9));
        let first_address = address_path(&first);
        first.close();
        let identity_name = IdentityKey::of_identity(identity_of(9)).lock_name();
        assert_eq!(
            server_files::read_lock_record(&fixture.dir, &identity_name).unwrap(),
            Record::None,
            "the free identity lock still names the first server"
        );
        let second = start_keyed(&fixture, keys_as(reuse.path(), 9));
        assert_ne!(
            address_path(&second),
            first_address,
            "the address follows the path, not the inode"
        );
        let Record::Complete(record) =
            server_files::read_lock_record(&fixture.dir, &identity_name).unwrap()
        else {
            panic!("the second server did not record itself in the identity lock");
        };
        assert_eq!(record.address, address_path(&second));
        assert!(
            !Record::Complete(record).names(&first_address),
            "a grant naming the first address matches the second"
        );
        second.close();
    });
}

/// Round 2 P2-1 (a): one canonical path with another identity (the directory was replaced under a running server) still
/// refuses on the workspace lock, naming the running server.
#[test]
fn a_replaced_directory_at_a_running_servers_path_refuses_on_the_workspace_lock() {
    let fixture = fixture();
    let first = start_keyed(&fixture, keys_as(&fixture.root, 11));
    match server_files::claim_keyed(&fixture.runtime, keys_as(&fixture.root, 12), None, BOUND) {
        Err(FilesRefusal::Held {
            lock,
            record: Record::Complete(record),
        }) => {
            assert_eq!(lock, fixture.lock());
            assert_eq!(record.address, address_path(&first));
        }
        Err(other) => panic!("the second start should find the first: {other}"),
        Ok(_) => panic!("the second start started a second server on one path"),
    }
    first.close();
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::super::local::LocalListener;
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};

        fn mode(path: &Path) -> u32 {
            std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
        }

        fn socket_at(fixture: &Fixture) -> PathBuf {
            fixture.dir.join(fixture.key().socket_name())
        }

        /// A socket at the server's name whose listener has gone: what a dead server, or a foreign program, leaves.
        fn dead_socket(path: &Path) {
            drop(UnixListener::bind(path).expect("a socket"));
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(UnixListener::bind(path).is_err(), "the socket file stays");
        }

        fn mkfifo(path: &Path) {
            let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: a NUL-terminated path.
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0, "mkfifo");
        }

        fn stale_refusal(result: Result<Claim, FilesRefusal>) -> server_files::StaleRefusal {
            match result {
                Err(FilesRefusal::Stale { refusal, .. }) => refusal,
                Err(other) => panic!("a stale refusal was expected: {other:?}"),
                Ok(_) => panic!("a stale refusal was expected, and the start went on"),
            }
        }

        #[test]
        fn a_symlink_spelling_of_the_root_has_the_roots_key() {
            let fixture = fixture();
            let link = fixture.dir.join("root-link");
            std::os::unix::fs::symlink(&fixture.root, &link).unwrap();
            assert_eq!(server_key(&link), fixture.key());
        }

        #[test]
        fn a_symlink_planted_at_the_lock_or_the_log_is_never_followed() {
            alone!(|| {
                let fixture = fixture();
                let target = fixture.dir.join("target");
                std::fs::write(&target, b"keep").unwrap();
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
                std::os::unix::fs::symlink(&target, fixture.lock()).unwrap();
                match fixture.claim() {
                    Err(FilesRefusal::File { path, fault }) => {
                        assert_eq!(path, fixture.lock());
                        assert_eq!(fault, FileFault::Link);
                    }
                    other => panic!("a link at the lock should refuse: {:?}", other.err()),
                }
                std::fs::remove_file(fixture.lock()).unwrap();
                std::os::unix::fs::symlink(&target, fixture.dir.join(fixture.key().log_name())).unwrap();
                let claim = fixture.claim().expect("claim");
                assert!(matches!(claim.open_log(), Err(FilesRefusal::File { fault: FileFault::Link, .. })));
                assert_eq!(std::fs::read(&target).unwrap(), b"keep", "the link's target was written");
            });
        }

        /// The file rule is one for both locks: a link at the identity lock is never followed and its target untouched.
        #[test]
        fn a_symlink_planted_at_the_identity_lock_is_never_followed() {
            let fixture = fixture();
            let target = fixture.dir.join("target");
            std::fs::write(&target, b"keep").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::os::unix::fs::symlink(&target, fixture.identity_lock()).unwrap();
            match fixture.claim() {
                Err(FilesRefusal::File { path, fault }) => {
                    assert_eq!(path, fixture.identity_lock());
                    assert_eq!(fault, FileFault::Link);
                }
                other => panic!("a link at the identity lock should refuse: {:?}", other.err()),
            }
            assert_eq!(std::fs::read(&target).unwrap(), b"keep", "the link's target was written");
        }

        #[test]
        fn a_fifo_planted_at_the_lock_or_the_log_never_blocks_the_start() {
            alone!(|| {
                let fixture = fixture();
                mkfifo(&fixture.lock());
                let (sender, receiver) = std::sync::mpsc::channel();
                let runtime = RuntimeDir::at(&fixture.dir).unwrap();
                let root = fixture.root.clone();
                std::thread::spawn(move || {
                    let lock = server_files::claim_within(&runtime, &root, None, BOUND).err();
                    std::fs::remove_file(runtime.path().join(server_key(&root).lock_name())).unwrap();
                    mkfifo(&runtime.path().join(server_key(&root).log_name()));
                    let log = server_files::claim_within(&runtime, &root, None, BOUND).map(|claim| claim.open_log().err());
                    sender.send((lock, log.ok().flatten())).unwrap();
                });
                let (lock, log) = receiver.recv_timeout(Duration::from_secs(10)).expect("a FIFO blocked the start");
                assert!(matches!(lock, Some(FilesRefusal::File { fault: FileFault::NotRegular, .. })), "{lock:?}");
                assert!(matches!(log, Some(FilesRefusal::File { fault: FileFault::NotRegular, .. })), "{log:?}");
            });
        }

        #[test]
        fn a_lock_file_open_to_group_or_other_is_refused_and_left_as_it_was() {
            let fixture = fixture();
            std::fs::write(fixture.lock(), b"someone's bytes").unwrap();
            std::fs::set_permissions(fixture.lock(), std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(matches!(
                fixture.claim(),
                Err(FilesRefusal::File { fault: FileFault::GroupOrOtherAccess, .. })
            ));
            assert_eq!(mode(&fixture.lock()), 0o644, "the file was repaired");
            assert_eq!(std::fs::read(fixture.lock()).unwrap(), b"someone's bytes", "the file was truncated");
        }

        #[test]
        fn an_explicit_log_in_a_shared_directory_opens_and_a_link_there_is_still_refused() {
            alone!(|| {
                let fixture = fixture();
                let shared = fixture.root.join("logs");
                std::fs::create_dir(&shared).unwrap();
                std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
                let log = shared.join("server.log");
                let claim = server_files::claim_within(&fixture.runtime, &fixture.root, Some(&log), BOUND).unwrap();
                drop(claim.open_log().expect("an explicit log"));
                assert_eq!(mode(&log), 0o600);
                drop(claim);
                std::fs::remove_file(&log).unwrap();
                std::os::unix::fs::symlink(fixture.dir.join("elsewhere"), &log).unwrap();
                let claim = server_files::claim_within(&fixture.runtime, &fixture.root, Some(&log), BOUND).unwrap();
                assert!(matches!(claim.open_log(), Err(FilesRefusal::File { fault: FileFault::Link, .. })));
            });
        }

        #[test]
        fn new_lock_files_are_0600_and_the_socket_0600() {
            let fixture = fixture();
            let server = fixture.start();
            assert_eq!(mode(&fixture.lock()), 0o600);
            assert_eq!(mode(&fixture.identity_lock()), 0o600);
            assert_eq!(mode(server.address().path()), 0o600);
            assert_eq!(server.address().path(), socket_at(&fixture));
            server.close();
        }

        #[test]
        fn a_foreign_socket_is_never_removed() {
            alone!(|| {
                let fixture = fixture();
                let path = socket_at(&fixture);
                dead_socket(&path);
                assert_eq!(stale_refusal(fixture.claim()), server_files::StaleRefusal::Unrecorded);
                assert!(std::fs::symlink_metadata(&path).is_ok(), "a foreign socket was removed");
                // One that something answers on is refused too, naming the listener.
                std::fs::remove_file(&path).unwrap();
                let live = UnixListener::bind(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                assert_eq!(
                    stale_refusal(fixture.claim()),
                    server_files::StaleRefusal::Answering { pid: Some(std::process::id()) }
                );
                assert!(std::fs::symlink_metadata(&path).is_ok());
                drop(live);
            });
        }

        #[test]
        fn a_full_accept_queue_is_not_shown_stale() {
            use socket2::{Domain, SockAddr, Socket, Type};
            let fixture = fixture();
            let path = socket_at(&fixture);
            let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
            listener.bind(&SockAddr::unix(&path).unwrap()).unwrap();
            listener.listen(0).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            // Fill the queue until a connect is refused (macOS) or would block (Linux).
            let mut queued = Vec::new();
            loop {
                let client = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
                client.set_nonblocking(true).unwrap();
                match client.connect(&SockAddr::unix(&path).unwrap()) {
                    Ok(()) => queued.push(client),
                    Err(_) => break,
                }
                assert!(queued.len() < 1024, "the accept queue never filled");
            }
            let refusal = stale_refusal(fixture.claim());
            assert!(
                matches!(refusal, server_files::StaleRefusal::Unrecorded | server_files::StaleRefusal::Timeout),
                "{refusal:?}"
            );
            assert!(std::fs::symlink_metadata(&path).is_ok(), "a live socket was removed");
            drop(queued);
        }

        #[test]
        fn a_record_naming_another_address_blocks_removal() {
            alone!(|| {
                let fixture = fixture();
                let path = socket_at(&fixture);
                dead_socket(&path);
                let other = ServerRecord {
                    pid: 1,
                    address: fixture.dir.join("ws-ffffffffffffffff"),
                    root: fixture.root.clone(),
                    log: fixture.dir.join("log"),
                };
                std::fs::write(fixture.lock(), other.encode().unwrap()).unwrap();
                std::fs::set_permissions(fixture.lock(), std::fs::Permissions::from_mode(0o600)).unwrap();
                assert_eq!(stale_refusal(fixture.claim()), server_files::StaleRefusal::Unrecorded);
                assert!(std::fs::symlink_metadata(&path).is_ok(), "a socket no record names was removed");
            });
        }

        /// A crash leaves what a clean exit removes: the socket file, whose listener has gone, and the record that
        /// names it (the record is written before the bind, so every socket a server bound has one).
        #[test]
        fn a_crash_leaves_a_stale_socket_that_the_next_start_recovers() {
            alone!(|| {
                let fixture = fixture();
                let address = socket_at(&fixture);
                dead_socket(&address);
                let crashed = ServerRecord {
                    pid: 1,
                    address: address.clone(),
                    root: fixture.root.clone(),
                    log: fixture.dir.join(fixture.key().log_name()),
                };
                std::fs::write(fixture.lock(), crashed.encode().unwrap()).unwrap();
                std::fs::set_permissions(fixture.lock(), std::fs::Permissions::from_mode(0o600)).unwrap();
                let claim = fixture.claim().expect("the next start recovers");
                assert!(claim.previous().names(&address), "{:?}", claim.previous());
                let server = claim.bind(policy(), CarrierLimits::DEFAULT).expect("bind over the stale socket");
                assert_eq!(server.address().path(), address);
                server.close();
            });
        }

        #[test]
        fn a_bind_that_fails_leaves_no_record() {
            let fixture = fixture();
            let claim = fixture.claim().expect("claim");
            // Something takes the name between the stale test and the bind.
            std::fs::write(socket_at(&fixture), b"").unwrap();
            assert!(matches!(claim.bind(policy(), CarrierLimits::DEFAULT), Err(FilesRefusal::Bind(_))));
            assert_eq!(fixture.record(), Record::None);
        }

        #[test]
        fn unlinking_or_replacing_the_socket_or_the_lock_is_reported_by_the_link_check() {
            alone!(|| {
                use server_files::LinkLost;
                let fixture = fixture();
                let server = fixture.start();
                assert_eq!(server.check_links(), Ok(()));
                std::fs::remove_file(server.address().path()).unwrap();
                assert_eq!(server.check_links(), Err(LinkLost::Socket));
                drop(server);

                let server = fixture.start();
                std::fs::remove_file(fixture.lock()).unwrap();
                assert_eq!(server.check_links(), Err(LinkLost::Lock));
                std::fs::write(fixture.lock(), b"").unwrap();
                assert_eq!(server.check_links(), Err(LinkLost::Lock), "another file at the lock's name");
                drop(server);

                // The identity lock is part of the check: without it a second spelling could start a second server.
                std::fs::remove_file(fixture.lock()).unwrap();
                let server = fixture.start();
                assert_eq!(server.check_links(), Ok(()));
                std::fs::remove_file(fixture.identity_lock()).unwrap();
                assert_eq!(server.check_links(), Err(LinkLost::IdentityLock));
                std::fs::write(fixture.identity_lock(), b"").unwrap();
                assert_eq!(server.check_links(), Err(LinkLost::IdentityLock), "another file at the identity lock's name");
                drop(server);
            });
        }

        /// Code P3-1 / State S-3: the start writes the record before it binds, and a bind that fails clears it; a
        /// record that fails binds nothing.
        ///
        /// Round 2 Code P3-N1 / State N-1: the order is held by types. `record_then_bind`'s bind step takes a
        /// [`Recorded`], which only `write_record` makes (its field is private to the `lock` module), so the call
        /// site's mutation ORD2, `|| Ok(())` as the record step with the record written inside the bind step after
        /// `bind_at`, does not compile: `Ok(())` is not a `Result<Recorded, _>`, and the bind step cannot make the
        /// proof it is handed. The compile error is logged in `rem2-mut-ORD2.log`.
        #[test]
        fn the_start_records_before_it_binds() {
            let log = std::cell::RefCell::new(Vec::new());
            let file = tempfile::tempfile().unwrap();
            let sample = ServerRecord {
                pid: 1,
                address: PathBuf::from("/a"),
                root: PathBuf::from("/r"),
                log: PathBuf::from("/l"),
            };
            let record = || {
                log.borrow_mut().push("record");
                server_files::write_record(&file, &sample)
            };
            let bind = |_: Recorded| -> Result<(), FilesRefusal> {
                log.borrow_mut().push("bind");
                Ok(())
            };
            let unrecord = || log.borrow_mut().push("unrecord");
            let bound = server_files::record_then_bind(record, bind, unrecord);
            assert!(bound.is_ok());
            assert_eq!(*log.borrow(), ["record", "bind"]);
            log.borrow_mut().clear();
            let refused = |_: Recorded| -> Result<(), FilesRefusal> {
                log.borrow_mut().push("bind");
                Err(FilesRefusal::Bind(super::super::local::LocalRefusal::NameTaken))
            };
            let bound = server_files::record_then_bind(record, refused, unrecord);
            assert!(bound.is_err());
            assert_eq!(*log.borrow(), ["record", "bind", "unrecord"]);
            log.borrow_mut().clear();
            let no_record = || -> Result<Recorded, FilesRefusal> {
                Err(FilesRefusal::Bind(super::super::local::LocalRefusal::NameTaken))
            };
            let bound = server_files::record_then_bind(no_record, bind, unrecord);
            assert!(bound.is_err());
            assert!(log.borrow().is_empty(), "{:?}", log.borrow());
        }

        /// Code P2-2 (probe PD): the directory renamed away and a new one made at its path; the published address no
        /// longer reaches this server, so the check reports it before a second start takes the workspace.
        #[test]
        fn a_directory_replaced_under_a_running_server_is_reported_by_the_link_check() {
            let fixture = fixture();
            let server = fixture.start();
            assert_eq!(server.check_links(), Ok(()));
            let moved = fixture.dir.with_extension("moved");
            std::fs::rename(&fixture.dir, &moved).unwrap();
            std::fs::create_dir(&fixture.dir).unwrap();
            std::fs::set_permissions(&fixture.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(server.check_links(), Err(server_files::LinkLost::Directory), "the replaced directory went unseen");
            drop(server);
            std::fs::remove_dir_all(&moved).unwrap();
        }

        /// Code P3-3: a refusal names the file and the rule in words, and says what the user can do.
        #[test]
        fn a_refusal_names_the_file_and_the_rule() {
            alone!(|| {
                let fixture = fixture();
                let path = socket_at(&fixture);
                dead_socket(&path);
                let text = fixture.claim().err().expect("refused").to_string();
                assert!(text.contains(&path.display().to_string()), "{text}");
                assert!(text.contains("remove it if nothing uses it"), "{text}");
                std::fs::remove_file(&path).unwrap();

                std::fs::write(fixture.lock(), b"").unwrap();
                std::fs::set_permissions(fixture.lock(), std::fs::Permissions::from_mode(0o644)).unwrap();
                let text = fixture.claim().err().expect("refused").to_string();
                assert!(text.contains(&fixture.lock().display().to_string()), "{text}");
                assert!(text.contains("group or other users have access"), "{text}");

                let blocked = fixture.root.join("blocked");
                let root = blocked.join("workspace");
                std::fs::create_dir_all(&root).unwrap();
                std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
                let refused = server_files::claim(&fixture.runtime, &root, None);
                std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
                let text = refused.err().expect("refused").to_string();
                assert!(text.contains("cannot be identified: permission denied"), "{text}");
            });
        }

        #[test]
        fn a_listener_leaves_a_socket_another_server_bound_at_its_name() {
            let fixture = fixture();
            let first = LocalListener::bind_at(&fixture.dir, "ws-0000000000000001", policy(), CarrierLimits::DEFAULT)
                .unwrap();
            let path = first.address().path().to_path_buf();
            std::fs::remove_file(&path).unwrap();
            let second = LocalListener::bind_at(&fixture.dir, "ws-0000000000000001", policy(), CarrierLimits::DEFAULT)
                .unwrap();
            drop(first);
            assert!(std::fs::symlink_metadata(&path).is_ok(), "the first listener removed the second's socket");
            drop(second);
            assert!(std::fs::symlink_metadata(&path).is_err());
        }
    } else if #[cfg(windows)] {
        #[test]
        fn a_reparse_point_planted_at_the_lock_is_refused() {
            let fixture = fixture();
            let target = fixture.dir.join("target");
            std::fs::create_dir(&target).unwrap();
            let link = crate::git::endpoint::fixture_host::url_repository(&fixture.dir, &target);
            std::fs::rename(&link, fixture.lock()).unwrap();
            assert!(matches!(
                fixture.claim(),
                Err(FilesRefusal::File { fault: FileFault::ReparsePoint, .. })
            ));
        }

        /// Code P2-4: the per-user directory is checked through its handle; a junction there is refused, not
        /// followed.
        #[test]
        fn a_junction_as_the_directory_is_refused() {
            use super::super::local::{DirectoryFault, LocalRefusal};
            let fixture = fixture();
            let target = fixture.root.join("files");
            std::fs::create_dir(&target).unwrap();
            let link = crate::git::endpoint::fixture_host::url_repository(&fixture.dir, &target);
            let runtime = RuntimeDir::at(&link).unwrap();
            assert!(matches!(
                server_files::claim_within(&runtime, &fixture.root, None, BOUND),
                Err(FilesRefusal::Directory(LocalRefusal::Directory(DirectoryFault::Link)))
            ));
        }

        #[test]
        fn the_pipe_is_recorded_only_once_it_exists() {
            let fixture = fixture();
            let claim = fixture.claim().expect("claim");
            assert_eq!(fixture.record(), Record::None, "a record before the pipe exists");
            let server = claim.bind(policy(), CarrierLimits::DEFAULT).expect("bind");
            let Record::Complete(record) = fixture.record() else {
                panic!("no record after the bind");
            };
            assert_eq!(record.address, address_path(&server));
            assert!(record.address.to_str().unwrap().starts_with(r"\\.\pipe\gwz-"));
            assert_eq!(server.check_links(), Ok(()));
            server.close();
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_vendor = "apple")] {
        /// Round 2 P2-1 (a): a firmlink spelling of a root on the Data volume is another canonical path, so it has
        /// another `ws-<key>` name; the second start through it still finds the first server.
        #[test]
        fn a_firmlink_spelling_of_the_root_has_another_name_and_finds_the_running_server() {
            let temp = tempfile::Builder::new().prefix("gwz-key-firmlink").tempdir_in("/private/tmp").unwrap();
            let firmlink = Path::new("/System/Volumes/Data").join(temp.path().strip_prefix("/").unwrap());
            assert!(firmlink.is_dir(), "{}", firmlink.display());
            assert_ne!(std::fs::canonicalize(&firmlink).unwrap(), std::fs::canonicalize(temp.path()).unwrap());
            assert_ne!(
                server_key(&firmlink).socket_name(),
                server_key(temp.path()).socket_name()
            );
            let files = fixture();
            let runtime = &files.runtime;
            let first = server_files::claim_within(runtime, temp.path(), None, BOUND)
                .unwrap()
                .bind(policy(), CarrierLimits::DEFAULT)
                .unwrap();
            match server_files::claim_within(runtime, &firmlink, None, BOUND) {
                Err(FilesRefusal::Held { lock, record: Record::Complete(record) }) => {
                    assert_eq!(lock, files.dir.join(Keys::of_root(temp.path()).unwrap().identity.lock_name()));
                    assert_eq!(Keys::of_root(&firmlink).unwrap().identity, Keys::of_root(temp.path()).unwrap().identity);
                    assert_eq!(record.address, address_path(&first));
                }
                other => panic!("the firmlink spelling should find the first server: {:?}", other.err()),
            }
            first.close();
        }
    } else if #[cfg(windows)] {
        /// A junction to the root has the root's key (the Windows spelling the identity merges and a path does not).
        #[test]
        fn a_junction_spelling_of_the_root_has_the_roots_key() {
            let fixture = fixture();
            let link = crate::git::endpoint::fixture_host::url_repository(&fixture.dir, &fixture.root);
            assert_eq!(server_key(&link), fixture.key());
        }
    }
}
