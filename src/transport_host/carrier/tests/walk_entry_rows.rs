//! The carrier's entry points take the walk (plan step 2.3e2; 2.3e round 3 Code N3-1 to N3-4 and State S3-1 to S3-3), on
//! Linux and macOS: the listener binds only through a `Walk` made under the process's `NoTriggerGate`, in a directory of
//! this user's with no group or other bit, sets the socket's mode and removes the socket through the walked directory,
//! reports the address walked, and refuses a final link; the server's bind stays in the directory it claimed; the client
//! judges the socket and its directory through the walk's handles, and a walk that ended before the socket is a server
//! that is not there; the kernel's `fs.protected_symlinks` rule. `dial` and `connect_stream` take a `Walk` (Windows: a
//! `CheckedPipe`), so a path the walk did not produce does not compile there (the doc note on `handshake::dial`; the
//! type error is the lane's red evidence). One gate per process is `check_spawn_gate_minting.py`'s rule, and two gates
//! that overlap leave the trigger policy as found (`address_walk_rows`, the one row that sets the process's policy). The
//! other carrier rows reach their listeners through the same entry points (`local::walk_to`, `support::bind_path`).
use super::super::CarrierLimits;
use super::super::handshake::{Bounds, ClientConfig, DialFault, Intent, SERVER_UNAVAILABLE, dial};
use super::super::local::{
    DirectoryFault, LocalAddress, LocalListener, LocalRefusal, PeerPolicy,
    address::{Kind, Place, Probe, Seen, protected_link, walk, walk_with},
    connect_stream, row_gate,
    server_files::{self, FilesRefusal, Record},
    walk_to,
};
use super::server_files_rows::{RowBind, fixture};
use super::support::socket_tempdir;
use std::{
    collections::HashMap,
    os::fd::AsRawFd,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, PermissionsExt, symlink},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A private directory for a row's sockets (`support::socket_tempdir`), its path canonical (no link in it).
fn base() -> (tempfile::TempDir, PathBuf) {
    let dir = socket_tempdir("gwz-entry");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

fn policy() -> PeerPolicy {
    PeerPolicy::current_user().unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

fn private_dir(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// The bind through a walk: the listener reports the address walked (State S3-1: never the descriptor path Linux
/// records), its socket is in the walked directory with mode 0600, and the walked directory reaches the name before
/// and after the bind (State S3-3).
#[test]
fn a_listener_bound_through_the_walk_reports_the_address_walked() {
    let (_guard, dir) = base();
    let address = dir.join("ws-0000000000000001");
    let gate = row_gate();
    let walked = walk(&bytes(&address), &gate).unwrap();
    let directory = walked.directory().try_clone_to_owned().unwrap();
    let name = "ws-0000000000000001";
    assert_eq!(
        rustix::fs::statat(&directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).err(),
        Some(rustix::io::Errno::NOENT)
    );
    let listener = LocalListener::bind(walked, policy(), CarrierLimits::DEFAULT).expect("bind");
    let stat = rustix::fs::statat(&directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).unwrap();
    assert_eq!(
        rustix::fs::FileType::from_raw_mode(stat.st_mode),
        rustix::fs::FileType::Socket
    );
    assert_eq!(listener.address().path(), address);
    assert_eq!(mode(&address), 0o600);
    // The kernel's record of the socket's address, read on a copy of the listener's descriptor.
    // SAFETY: the listener's descriptor is open for the whole statement; the copy is owned and closed by `copy`.
    let copy = unsafe { std::os::fd::BorrowedFd::borrow_raw(listener.as_raw_fd()) }
        .try_clone_to_owned()
        .unwrap();
    let kernel = std::os::unix::net::UnixListener::from(copy)
        .local_addr()
        .unwrap();
    let kernel = kernel.as_pathname().expect("a path address");
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "android"))] {
            assert!(kernel.starts_with("/proc/self/fd/"), "{kernel:?}");
        } else {
            assert_eq!(kernel, address);
        }
    }
    drop(listener);
    assert!(
        std::fs::symlink_metadata(&address).is_err(),
        "the socket was not removed"
    );
}

/// Plan 2.3e2: a bind whose directory is swapped after the walk sets the mode on the walked directory's entry only.
/// Linux binds through the walked directory, so the socket is there, with its mode, and the entry the swap planted at
/// the address is untouched. macOS binds by the address (no `bindat`), so the socket lands in the new directory: the
/// bind is refused and no mode is set by path.
#[test]
fn a_bind_whose_directory_is_swapped_after_the_walk_sets_the_mode_on_the_walked_entry_only() {
    let (_guard, dir) = base();
    let (here, moved) = (dir.join("a"), dir.join("walked"));
    private_dir(&here);
    let address = here.join("ws");
    let gate = row_gate();
    let walked = walk(&bytes(&address), &gate).unwrap();
    std::fs::rename(&here, &moved).unwrap();
    private_dir(&here);
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "android"))] {
            // A file the swap left at the address, which a mode set by path would change.
            std::fs::write(&address, b"planted").unwrap();
            std::fs::set_permissions(&address, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
    let bound = LocalListener::bind(walked, policy(), CarrierLimits::DEFAULT);
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "android"))] {
            let listener = bound.expect("bound in the walked directory");
            assert!(std::fs::symlink_metadata(moved.join("ws")).unwrap().file_type().is_socket());
            assert_eq!(mode(&moved.join("ws")), 0o600);
            assert_eq!(listener.address().path(), address, "the address of record is the address walked");
            drop(listener);
            assert!(std::fs::symlink_metadata(moved.join("ws")).is_err(), "removed through the walked directory");
        } else {
            let refused = bound.err().expect("the bind is refused");
            assert!(
                matches!(&refused, LocalRefusal::Address { unavailable: false, text } if text.contains("not the directory")),
                "{refused:?}"
            );
            assert!(std::fs::symlink_metadata(moved.join("ws")).is_err());
        }
    }
    // The entry at the address keeps its own mode: nothing set a mode by path.
    assert_ne!(mode(&address), 0o600, "a mode was set by path");
}

/// The listener removes its socket through the walked directory: once the directory is moved and a file planted at the
/// address, the drop removes the socket it bound and leaves the planted file.
#[test]
fn the_socket_is_removed_through_the_walked_directory_never_by_path() {
    let (_guard, dir) = base();
    let (here, moved) = (dir.join("a"), dir.join("walked"));
    private_dir(&here);
    let address = here.join("ws");
    let gate = row_gate();
    let listener = LocalListener::bind(
        walk(&bytes(&address), &gate).unwrap(),
        policy(),
        CarrierLimits::DEFAULT,
    )
    .expect("bind");
    std::fs::rename(&here, &moved).unwrap();
    private_dir(&here);
    std::fs::write(&address, b"planted").unwrap();
    drop(listener);
    assert!(
        std::fs::symlink_metadata(moved.join("ws")).is_err(),
        "the socket bound is left"
    );
    assert_eq!(std::fs::read(&address).unwrap(), b"planted");
}

/// Code N3-3: a final link at the bind name refuses (Linux's own `bind` refuses it; macOS's follows it to the target),
/// so the server's socket is always the address's own entry; a link above the final name is walked as before.
#[test]
fn a_final_link_at_the_bind_name_refuses() {
    let (_guard, dir) = base();
    private_dir(&dir.join("t"));
    let link = dir.join("L");
    symlink(dir.join("t/s"), &link).unwrap();
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "android"))] {
            let kernel = std::os::unix::net::UnixListener::bind(&link).unwrap_err();
            assert_eq!(kernel.kind(), std::io::ErrorKind::AddrInUse, "the kernel's own bind");
        }
    }
    let gate = row_gate();
    let walked = walk(&bytes(&link), &gate).unwrap();
    assert_eq!(*walked.place(), Place::Ended(bytes(&dir.join("t/s"))));
    assert!(walked.final_link());
    let refused = LocalListener::bind(walked, policy(), CarrierLimits::DEFAULT)
        .err()
        .expect("refused");
    assert!(
        matches!(&refused, LocalRefusal::Address { unavailable: false, text } if text.contains("a link")),
        "{refused:?}"
    );
    assert!(
        std::fs::symlink_metadata(dir.join("t/s")).is_err(),
        "bound at the link's target"
    );
    // A link above the final name is not a final link.
    symlink(dir.join("t"), dir.join("M")).unwrap();
    let walked = walk(&bytes(&dir.join("M/s")), &gate).unwrap();
    assert!(!walked.final_link());
    let _listener =
        LocalListener::bind(walked, policy(), CarrierLimits::DEFAULT).expect("bind below a link");
    assert!(
        std::fs::symlink_metadata(dir.join("t/s"))
            .unwrap()
            .file_type()
            .is_socket()
    );
}

/// A file system of directories, links and files, as a probe sees it: the walk's decisions on any host.
struct Fake(HashMap<&'static [u8], Seen<String, ()>>);

impl Probe for Fake {
    type Dir = String;
    type Leaf = ();

    fn root(&mut self) -> Result<String, Kind> {
        Ok(String::new())
    }

    fn look(&mut self, dir: &String, name: &[u8]) -> Result<Seen<String, ()>, Kind> {
        let path = format!("{dir}/{}", String::from_utf8_lossy(name));
        Ok(match self.0.get(path.as_bytes()) {
            Some(Seen::Link(target)) => Seen::Link(target.clone()),
            Some(Seen::Directory(_)) => Seen::Directory(path),
            Some(Seen::Other(())) => Seen::Other(()),
            Some(Seen::Missing) | None => Seen::Missing,
        })
    }

    fn socket_directory(&mut self, _: &String) -> Result<(), Kind> {
        Ok(())
    }
}

#[test]
fn a_walk_records_whether_its_final_name_is_a_link() {
    let mut fake = Fake(HashMap::from([
        (&b"/w"[..], Seen::Directory(String::new())),
        (&b"/w/t"[..], Seen::Directory(String::new())),
        (&b"/w/L"[..], Seen::Link(b"/w/t/s".to_vec())),
        (&b"/w/R"[..], Seen::Link(b"t/s".to_vec())),
        (&b"/w/D"[..], Seen::Link(b"t".to_vec())),
        (&b"/w/t/s"[..], Seen::Other(())),
    ]));
    for (address, final_link) in [
        (&b"/w/L"[..], true),
        (b"/w/R", true),
        (b"/w/D/s", false),
        (b"/w/t/s", false),
        (b"/w/t/missing", false),
    ] {
        let walked = walk_with(address, &mut fake).unwrap();
        assert_eq!(
            walked.final_link(),
            final_link,
            "{}",
            String::from_utf8_lossy(address)
        );
    }
}

/// Code N3-3: Linux's `fs.protected_symlinks` rule (`may_follow_link`), judged on any host.
#[test]
fn the_protected_symlinks_rule_is_the_kernels() {
    // Another user's link in a sticky directory others may write (`/tmp`): refused.
    assert!(protected_link(0o41777, 0, 1000, 1001));
    // The follower's own link, and the directory owner's: followed.
    assert!(!protected_link(0o41777, 0, 1001, 1001));
    assert!(!protected_link(0o41777, 1000, 1000, 1001));
    // Not sticky, or not writable by others: followed.
    assert!(!protected_link(0o40777, 0, 1000, 1001));
    assert!(!protected_link(0o41775, 0, 1000, 1001));
}

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        /// Code N3-3 on a real Linux host, as root (who can give a link another owner): a link another user owns in a
        /// sticky directory others may write is refused by the walk, as the kernel refuses to follow it. Run with
        /// `--ignored`; without root the row fails naming the missing fixture.
        #[test]
        #[ignore = "needs root to give a link another owner"]
        fn a_link_the_kernel_will_not_follow_is_refused() {
            // SAFETY: `geteuid` has no preconditions.
            if unsafe { libc::geteuid() } != 0 {
                panic!("no fixture: the row needs root to give a link another owner");
            }
            let (_guard, dir) = base();
            let sticky = dir.join("sticky");
            std::fs::create_dir(&sticky).unwrap();
            std::fs::set_permissions(&sticky, std::fs::Permissions::from_mode(0o1777)).unwrap();
            private_dir(&dir.join("t"));
            let link = sticky.join("L");
            symlink(dir.join("t"), &link).unwrap();
            let name = std::ffi::CString::new(link.as_os_str().as_bytes()).unwrap();
            // SAFETY: `name` is NUL-terminated.
            assert_eq!(unsafe { libc::lchown(name.as_ptr(), 4242, 4242) }, 0);
            let kernel = std::fs::metadata(dir.join("sticky/L/."));
            let refusal = walk(&bytes(&link.join("ws")), &row_gate()).unwrap_err();
            assert_eq!(refusal.kind, Kind::ProtectedLink, "{refusal}; the kernel's own lookup: {kernel:?}");
        }

        /// Code F5 / State F5 (2.3e2 round 1): the walked directory swapped after the walk (renamed away, a link to
        /// another listener's directory at its path): the client's connect reaches the walked listener through the
        /// walk's descriptor, never the planted one.
        #[test]
        fn a_client_connect_reaches_the_walked_listener_after_a_swap() {
            let (_guard, dir) = base();
            let (here, other) = (dir.join("a"), dir.join("b"));
            private_dir(&here);
            private_dir(&other);
            let bind = |path: &Path| super::support::bind_path(path, policy(), CarrierLimits::DEFAULT).unwrap();
            let (walked, planted) = (bind(&here.join("ws")), bind(&other.join("ws")));
            for listener in [&walked, &planted] {
                // SAFETY: the listener's descriptor is open for the whole statement.
                let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(listener.as_raw_fd()) };
                rustix::io::ioctl_fionbio(fd, true).unwrap();
            }
            let gate = row_gate();
            let walk = walk_to(&LocalAddress::from_path(&here.join("ws")), &gate).unwrap();
            std::fs::rename(&here, dir.join("a-walked")).unwrap();
            symlink(&other, &here).unwrap();
            let _client = connect_stream(walk, &policy(), soon()).expect("connected");
            assert!(walked.accept().is_ok(), "the walked listener has no connection");
            assert!(planted.accept().is_err(), "the planted listener was reached");
        }
    }
}

/// A connect's bound in these rows: nothing here waits on a full queue.
fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

/// A directory root owns that others may enter (`/tmp`), for the rows about a directory that is not the user's own.
fn root_owned() -> PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(target_vendor = "apple")] {
            PathBuf::from("/private/tmp")
        } else {
            PathBuf::from("/tmp")
        }
    }
}

/// A unique name for a row's entry in a shared directory.
fn unique(prefix: &str) -> String {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).unwrap();
    format!("{prefix}-{:016x}", u64::from_le_bytes(random))
}

/// Removes a row's socket from a shared directory when the row ends, however it ends.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// State F1 / Code F4 (2.3e2 round 1): a client whose server directory does not exist yet, below a directory that is not
/// the user's own, is told that nothing answers there (`server_unavailable`), not that the directory above is unusable.
#[test]
fn a_connect_below_a_missing_directory_is_server_unavailable() {
    let address = LocalAddress::from_path(
        &root_owned()
            .join(unique("gwz-missing"))
            .join("ws-0000000000000001"),
    );
    let policy = policy();
    let gate = row_gate();
    let refused = connect_stream(walk_to(&address, &gate).expect("the walk"), &policy, soon())
        .err()
        .expect("nothing to connect to");
    assert!(
        matches!(
            &refused,
            LocalRefusal::Address {
                unavailable: true,
                ..
            }
        ),
        "{refused:?}: {refused}"
    );
    let config = ClientConfig {
        supported: 1..=2,
        client: "gwz 1.1.0".into(),
        limits: None,
        peer_pid: None,
        bounds: Bounds::DEFAULT,
    };
    let fault = dial(
        walk_to(&address, &gate).expect("the walk"),
        &policy,
        CarrierLimits::DEFAULT,
        &config,
        &Intent::Open,
    )
    .err()
    .expect("nothing to dial");
    assert!(
        matches!(
            fault,
            DialFault::Connect(LocalRefusal::Address {
                unavailable: true,
                ..
            })
        ),
        "{fault:?}"
    );
    assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
}

/// Code F3 (2.3e2 round 1): the listener binds only in a directory of this user's with no group or other bit, judged
/// through the walked directory's descriptor, as the client judges it; nothing is created otherwise.
#[test]
fn a_bind_into_a_directory_others_can_enter_is_refused() {
    let (_guard, dir) = base();
    let wide = dir.join("wide");
    std::fs::create_dir(&wide).unwrap();
    std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o755)).unwrap();
    let address = wide.join("ws-0000000000000001");
    let gate = row_gate();
    let refused = LocalListener::bind(
        walk(&bytes(&address), &gate).unwrap(),
        policy(),
        CarrierLimits::DEFAULT,
    )
    .err()
    .expect("refused");
    assert_eq!(
        refused,
        LocalRefusal::Directory(DirectoryFault::GroupOrOtherAccess)
    );
    assert!(
        std::fs::symlink_metadata(&address).is_err(),
        "a socket was created"
    );
}

/// Code F5 / State F5 (2.3e2 round 1): the client judges the socket's directory through the walked descriptor: one
/// others can enter is refused, and the listener there sees no connection.
#[test]
fn a_client_refuses_a_socket_whose_directory_others_can_enter() {
    let (_guard, dir) = base();
    let wide = dir.join("wide");
    std::fs::create_dir(&wide).unwrap();
    std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o755)).unwrap();
    let address = wide.join("ws");
    let listener = UnixListener::bind(&address).unwrap();
    std::fs::set_permissions(&address, std::fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let gate = row_gate();
    let refused = connect_stream(
        walk_to(&LocalAddress::from_path(&address), &gate).unwrap(),
        &policy(),
        soon(),
    )
    .err()
    .expect("refused");
    assert_eq!(
        refused,
        LocalRefusal::Directory(DirectoryFault::GroupOrOtherAccess)
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// Code F5 / State F5 (2.3e2 round 1): a socket in a directory root owns is refused as another owner's (as root, whose
/// own it is, as one others can enter: `/tmp` is 1777).
#[test]
fn a_client_refuses_a_socket_in_a_directory_of_another_owner() {
    let address = root_owned().join(unique("gwz-other-owner"));
    let listener = UnixListener::bind(&address).unwrap();
    let _removed = Removed(address.clone());
    std::fs::set_permissions(&address, std::fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let gate = row_gate();
    let refused = connect_stream(
        walk_to(&LocalAddress::from_path(&address), &gate).unwrap(),
        &policy(),
        soon(),
    )
    .err()
    .expect("refused");
    // SAFETY: `geteuid` has no preconditions.
    let fault = if unsafe { libc::geteuid() } == 0 {
        DirectoryFault::GroupOrOtherAccess
    } else {
        DirectoryFault::OtherOwner
    };
    assert_eq!(refused, LocalRefusal::Directory(fault));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// State F3 / Code F3 (2.3e2 round 1, RV-4): the claimed directory renamed away and another made at its path, with a
/// file planted at `ws-<key>` in the claimed one: the bind's walk reaches the new directory, which is not the claimed
/// one, so the bind is refused, nothing is bound in either, and the record is cleared.
#[test]
fn a_bind_whose_walk_reaches_another_directory_than_the_claimed_one_is_refused() {
    let fixture = fixture();
    let claim = fixture.claim().expect("claim");
    let name = fixture.key().socket_name();
    let claimed = fixture.dir.with_file_name(unique("gwz-claimed"));
    std::fs::rename(&fixture.dir, &claimed).unwrap();
    private_dir(&fixture.dir);
    std::fs::write(claimed.join(&name), b"planted").unwrap();
    let refused = claim.bind_row().err();
    let left = std::fs::symlink_metadata(fixture.dir.join(&name)).is_ok();
    let planted = std::fs::read(claimed.join(&name));
    let record = server_files::read_lock_record(&claimed, &fixture.key().lock_name());
    std::fs::remove_dir_all(&claimed).unwrap();
    assert!(
        matches!(
            &refused,
            Some(FilesRefusal::Bind(LocalRefusal::Address {
                unavailable: false,
                ..
            }))
        ),
        "{refused:?}"
    );
    assert!(!left, "a socket was bound in the new directory");
    assert_eq!(planted.unwrap(), b"planted");
    assert_eq!(record.unwrap(), Record::None, "the record was not cleared");
}
