use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};

use cap_fs_ext::MetadataExt;
use cap_std::fs::{Dir, File};

use super::super::super::*;
use super::VolumeDescription;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

const FS_CASEFOLD_FL: libc::c_long = 0x4000_0000;
const MAX_HANDLE_BYTES: usize = 128;
const FS_IOC_GETFSUUID: libc::c_ulong = ior(0x15, 0, 17) as libc::c_ulong;
const FS_IOC_GETFLAGS: libc::c_ulong =
    ior(b'f' as u32, 1, std::mem::size_of::<libc::c_long>() as u32) as libc::c_ulong;

/// Superblock magics the `libc` crate does not publish, taken from
/// `linux/magic.h` (verified against libc 0.2.189: it publishes
/// `TMPFS_MAGIC`, `BTRFS_SUPER_MAGIC`, `EXT4_SUPER_MAGIC`,
/// `F2FS_SUPER_MAGIC`, `FUSE_SUPER_MAGIC`, `NFS_SUPER_MAGIC` and
/// `OVERLAYFS_SUPER_MAGIC`, and none of the six below).
///
/// The `u32` spelling is deliberate: the kernel stores `s_magic` as an
/// `unsigned long` and `statfs.f_type` is the signed `__fsword_t`, so a magic
/// with the top word bit set must be cast through `as libc::c_long` rather
/// than written as a signed literal — which is exactly what libc itself does
/// for `BTRFS_SUPER_MAGIC`.
const RAMFS_MAGIC: libc::c_long = magic(0x8584_58f6);
const XFS_SUPER_MAGIC: libc::c_long = magic(0x5846_5342);
const CIFS_SUPER_MAGIC: libc::c_long = magic(0xff53_4d42);
const SMB2_SUPER_MAGIC: libc::c_long = magic(0xfe53_4d42);
const V9FS_MAGIC: libc::c_long = magic(0x0102_1997);
const CEPH_SUPER_MAGIC: libc::c_long = magic(0x00c3_6400);
/// OpenZFS is out of tree, so this one is not in `linux/magic.h`: it is
/// `ZFS_SUPER_MAGIC` from OpenZFS `include/sys/fs/zfs.h`.
const ZFS_SUPER_MAGIC: libc::c_long = magic(0x2fc1_2fc1);

/// Filesystem names that mean "the bytes are reached over a network"
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03). This is a
/// LABELLED LIST used only to choose the warning's parenthetical; it is never
/// a denylist, because network is a warning REASON and not a hidden name test
/// (charter §0.1). A network volume that can still prove durable identity is
/// admitted by `identity` below regardless of what this list says.
const REMOTE_FILESYSTEM_NAMES: [&str; 8] =
    ["nfs", "nfs4", "cifs", "smb2", "smb3", "9p", "afs", "ceph"];

/// The documented FUSE subtypes whose backing store is remote. `fuse` alone
/// says nothing about locality, so only these named variants classify remote.
const REMOTE_FUSE_SUBTYPES: [&str; 5] = [
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.davfs2",
    "fuse.s3fs",
    "fuse.gcsfuse",
];

/// Filesystem names whose contents do not survive power loss.
const VOLATILE_FILESYSTEM_NAMES: [&str; 2] = ["tmpfs", "ramfs"];

#[repr(C)]
struct FsUuid2 {
    len: u8,
    uuid: [u8; 16],
}

#[repr(C)]
struct LinuxFileHandle {
    handle_bytes: u32,
    handle_type: i32,
    bytes: [u8; MAX_HANDLE_BYTES],
}

/// DR-1 W2, 2026-09-03 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.2): the
/// variant's NAME still says `Ext4` while the admission it stands for is no
/// longer a filesystem-name test — `identity` below admits ext4, xfs, f2fs
/// and anything else that answers `FS_IOC_GETFSUUID` with a nonzero 16-byte
/// UUID and `name_to_handle_at` with a persistent handle. The name stays
/// because `SupportedFilesystemProfile` is a PERSISTED catalog value:
/// renaming it is a catalog-format change, which the charter parks
/// ("What this is not", §0, and the (b) design's dual-tuple migration).
pub(super) const fn support_profile() -> SupportedFilesystemProfile {
    SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1
}

pub(super) fn dir_identity(
    directory: &Dir,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    let queryable = descriptor_query_fd(directory)?;
    identity(
        queryable.as_raw_fd(),
        &directory.dir_metadata().map_err(io_identity)?,
    )
}

pub(super) fn file_identity(
    file: &File,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    identity(file.as_raw_fd(), &file.metadata().map_err(io_identity)?)
}

pub(super) fn parent_mode(parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
    // DR-1 (a0), 2026-09-03: no filesystem-type test here. `FS_IOC_GETFLAGS` refuses on
    // its own where the driver lacks `fileattr_get`, and every path reaching this
    // function has already passed `identity` (`catalog_lease/target.rs`), which is the
    // ONE admission gate. A second magic-number test here bought nothing and hid the
    // real gate behind three.
    let queryable = descriptor_query_fd(parent)?;
    let mut flags: libc::c_long = 0;
    if unsafe { libc::ioctl(queryable.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) } != 0 {
        return Err(query_error(
            PlatformCapability::PathEquivalence,
            "query Linux directory attribute flags",
        ));
    }
    Ok(if flags & FS_CASEFOLD_FL == 0 {
        PathComponentMode::Sensitive
    } else {
        PathComponentMode::AsciiCaseFold
    })
}

pub(super) fn rename_domain(directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
    // DR-1 (a0), 2026-09-03: no filesystem-type test here either — `statx(MNT_ID)` is a
    // VFS field present on every Linux filesystem and refuses on its own; `identity`
    // above is the admission gate.
    let mount_id = mount_identity(directory)?.ok_or_else(|| {
        CheckedFsError::unsupported(
            PlatformCapability::AtomicRenameDomain,
            "filesystem does not expose a mount identity",
        )
    })?;
    Ok(mount_id.to_be_bytes().to_vec())
}

/// The volume's name and its two classifications, as a WORDING AID only
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03).
///
/// Nothing here decides admission. The one classification that does —
/// volatility — is applied by `identity` below on its own `fstatfs`, so a
/// caller that never calls this function still gets the volatile refusal, and
/// a caller that calls it cannot widen or narrow admission by what it reads.
///
/// The name comes from `/proc/self/mountinfo`, because that is the only
/// source that distinguishes the FUSE subtypes (`fuse.sshfs` vs `fuse.s3fs`)
/// and the network variants (`nfs4` vs `nfs`) that the warning's parenthetical
/// turns on; the superblock magic cannot, since every FUSE mount reports
/// `FUSE_SUPER_MAGIC`. The magic table is the fallback for a host without
/// `/proc` or without `STATX_MNT_ID` (kernels < 5.8), and `None` — rendered
/// `unknown` by the warning — is the honest answer when neither can name it.
///
/// `fstatfs` and `statx` are traversal-class operations, which the kernel
/// answers on the `O_PATH` capability descriptor itself (see
/// `descriptor_query_fd`), so this reads the volume without reopening it.
pub(super) fn describe_volume(directory: &Dir) -> Result<VolumeDescription, CheckedFsError> {
    let f_type = filesystem_type(directory.as_raw_fd())?;
    let name = mounted_filesystem_name(directory)
        .or_else(|| magic_filesystem_name(f_type).map(str::to_owned));
    Ok(classify_volume(name))
}

/// Reopens a directory capability as a descriptor that carries
/// descriptor-consuming operations (`ioctl`, `fsync`).
///
/// cap-std directory capabilities are `O_PATH` descriptors on Linux. The
/// kernel resolves descriptor-consuming operations through the descriptor
/// lookup that refuses `O_PATH` files with `EBADF` before any filesystem
/// code runs, while traversal-class operations (`fstatfs`, `statx`,
/// `name_to_handle_at`, `openat`) accept them — which is why the
/// superblock-magic read succeeds on the very descriptor whose UUID ioctl
/// reported `EBADF` on the ARM64 matrix. Reopening `.` through the capability
/// performs no path re-resolution — the descriptor itself anchors the lookup —
/// so the result names the same directory object (pinned by the
/// invocation-identity test below). Failures are never downgraded here: a dead
/// descriptor reports `EBADF` and a recycled non-directory descriptor reports
/// `ENOTDIR`, both as hard I/O errors, so genuine descriptor-lifecycle defects
/// stay loud.
fn descriptor_query_fd(directory: impl AsFd) -> Result<OwnedFd, CheckedFsError> {
    rustix::fs::openat(
        directory,
        c".",
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|source| {
        CheckedFsError::io(
            "reopen Linux directory for descriptor-consuming queries",
            io::Error::from_raw_os_error(source.raw_os_error()),
        )
    })
}

/// The catalog's ONE admission gate — still true after DR-1 W2, and now
/// identity-based rather than a filesystem-name test.
///
/// DR-1 (a0) (2026-09-03) removed the two gratuitous `require_ext4` calls and
/// left this one; DR-1 W2 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.2,
/// 2026-09-03) removes the function itself. What admits a Linux volume is the
/// evidence: a nonzero 16-byte `FS_IOC_GETFSUUID` volume UUID plus a
/// persistent `name_to_handle_at` handle. ext4, xfs and f2fs are admitted
/// alike (they call `super_set_uuid`); btrfs is refused by the ioctl itself
/// (`ENOTTY`, no `super_set_uuid` in `fs/btrfs/`), as are pre-6.9 kernels,
/// every network mount, and every filesystem without persistent handles.
///
/// The volatility refusal in front of the ioctl is the one name-shaped test
/// that remains, and it exists because identity alone would admit a volume
/// whose contents do not survive power loss: tmpfs calls `super_set_uuid`
/// with a RANDOM per-mount UUID on every kernel that has the ioctl at all
/// (`mm/shmem.c:4405` at v6.9). Keeping the refusal inside the provider also
/// keeps R0-L's negative tmpfs row true.
///
/// **It is a CATALOG ADMISSION refusal only** (charter §0.1, the operator's
/// ruling of 2026-09-03). The merge still STARTS on tmpfs: W3's decision
/// point maps every probe-time `Unsupported` — this one included — onto the
/// warning path and runs the merge without activating the catalog. Only
/// `--filesystem-strict` turns it into a refusal. Nothing here may become a
/// default merge refusal.
fn identity(
    fd: RawFd,
    metadata: &impl MetadataExt,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    refuse_volatile_filesystem(fd)?;
    let uuid = filesystem_uuid(fd)?;
    let (handle_type, handle) = persistent_handle(fd)?;
    let durable = DurableObjectIdentityV1::linux_ext4(uuid, handle_type, handle)?;
    let mut invocation = Vec::with_capacity(16);
    invocation.extend_from_slice(&metadata.dev().to_be_bytes());
    invocation.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(ObjectIdentityFact::new(durable, invocation))
}

/// Charter §3.2's volatility refusal. See `identity` for why it is here and
/// why it must never become a default merge refusal.
fn refuse_volatile_filesystem(fd: RawFd) -> Result<(), CheckedFsError> {
    let f_type = filesystem_type(fd)?;
    if f_type == libc::TMPFS_MAGIC || f_type == RAMFS_MAGIC {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "volatile filesystem: contents do not survive power loss",
        ));
    }
    Ok(())
}

fn filesystem_type(fd: RawFd) -> Result<libc::c_long, CheckedFsError> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(fd, stat.as_mut_ptr()) } != 0 {
        return Err(CheckedFsError::io(
            "query Linux filesystem type",
            io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { stat.assume_init() }.f_type)
}

fn filesystem_uuid(fd: RawFd) -> Result<[u8; 16], CheckedFsError> {
    let mut value = FsUuid2 {
        len: 0,
        uuid: [0; 16],
    };
    if unsafe { libc::ioctl(fd, FS_IOC_GETFSUUID, &mut value) } != 0 {
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query external filesystem UUID",
        ));
    }
    if value.len != 16 || value.uuid == [0; 16] {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "filesystem returned an absent or malformed external UUID",
        ));
    }
    Ok(value.uuid)
}

fn persistent_handle(fd: RawFd) -> Result<(i32, Vec<u8>), CheckedFsError> {
    let mut value = LinuxFileHandle {
        handle_bytes: MAX_HANDLE_BYTES as u32,
        handle_type: 0,
        bytes: [0; MAX_HANDLE_BYTES],
    };
    let mut mount_id = 0;
    if unsafe {
        libc::name_to_handle_at(
            fd,
            c"".as_ptr(),
            std::ptr::addr_of_mut!(value).cast::<libc::file_handle>(),
            &mut mount_id,
            libc::AT_EMPTY_PATH,
        )
    } != 0
    {
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query retained empty-path file handle",
        ));
    }
    let length = value.handle_bytes as usize;
    if value.handle_type <= 0 || !(1..=MAX_HANDLE_BYTES).contains(&length) {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "filesystem returned an unsupported persistent handle",
        ));
    }
    Ok((value.handle_type, value.bytes[..length].to_vec()))
}

/// The directory's `STATX_MNT_ID` — the mount identity that both names the
/// rename domain and selects the `/proc/self/mountinfo` row. `Ok(None)` means
/// the kernel answered without filling the mask (pre-5.8), which each caller
/// interprets for itself.
fn mount_identity(directory: &Dir) -> Result<Option<u64>, CheckedFsError> {
    let stat = rustix::fs::statx(
        directory,
        "",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::MNT_ID,
    )
    .map_err(|source| {
        CheckedFsError::io(
            "query Linux rename domain",
            io::Error::from_raw_os_error(source.raw_os_error()),
        )
    })?;
    Ok((stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() != 0).then_some(stat.stx_mnt_id))
}

/// The directory's own mountinfo row's filesystem type, or `None` when the
/// host cannot answer (no `STATX_MNT_ID`, no `/proc`, or no matching row).
/// Best-effort by construction: this is a wording aid, so an unavailable
/// answer degrades to the magic table rather than failing the caller.
fn mounted_filesystem_name(directory: &Dir) -> Option<String> {
    let mount_id = mount_identity(directory).ok().flatten()?;
    let table = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    mountinfo_fstype(&table, mount_id).map(str::to_owned)
}

/// The filesystem type of the `/proc/self/mountinfo` row whose MOUNT ID
/// (field 1) is `mount_id` — the same identity `statx(STATX_MNT_ID)` returns.
///
/// Row shape (`Documentation/filesystems/proc.rst`,
/// `fs/proc_namespace.c::show_mountinfo`):
///
/// ```text
/// 36 35 98:0 /root /mount/point rw,noatime master:1 - ext3 /dev/root rw
/// (1)(2) (3)   (4)      (5)        (6)       (7)   (8)(9)   (10)   (11)
/// ```
///
/// Fields 7.. are OPTIONAL and variable in number, terminated by the `-`
/// separator at (8); the filesystem type is the field after it, and it carries
/// the FUSE subtype (`fuse.sshfs`) that the warning's parenthetical needs.
///
/// Field counting by whitespace is exact even for paths containing spaces:
/// the kernel writes fields (4) and (5) through `seq_path_root(.., " \t\n\\")`,
/// which escapes space, tab, newline and backslash as octal (`\040`, `\011`,
/// `\012`, `\134`), so no path field can ever contain a literal separator or
/// be mistaken for the `-`. Nothing here needs the path itself, so the escapes
/// are read past rather than decoded; the pin below is the row that proves the
/// counting survives one.
fn mountinfo_fstype(table: &str, mount_id: u64) -> Option<&str> {
    table.lines().find_map(|row| {
        let mut fields = row.split_ascii_whitespace();
        if fields.next()?.parse::<u64>().ok()? != mount_id {
            return None;
        }
        // Fields (2)..(6) are fixed; the optional fields start at (7).
        let mut optional = fields.skip(5);
        optional.find(|field| *field == "-")?;
        optional.next()
    })
}

/// The fallback namer: a small superblock-magic table over the filesystems
/// the charter §3.3 names. It cannot distinguish FUSE subtypes or NFS
/// versions — that is what `mountinfo_fstype` above is for — so it answers
/// with the family name and `None` for anything it does not know.
fn magic_filesystem_name(f_type: libc::c_long) -> Option<&'static str> {
    Some(match f_type {
        libc::EXT4_SUPER_MAGIC => "ext4",
        XFS_SUPER_MAGIC => "xfs",
        libc::BTRFS_SUPER_MAGIC => "btrfs",
        libc::F2FS_SUPER_MAGIC => "f2fs",
        ZFS_SUPER_MAGIC => "zfs",
        libc::TMPFS_MAGIC => "tmpfs",
        RAMFS_MAGIC => "ramfs",
        libc::NFS_SUPER_MAGIC => "nfs",
        CIFS_SUPER_MAGIC => "cifs",
        SMB2_SUPER_MAGIC => "smb2",
        CEPH_SUPER_MAGIC => "ceph",
        libc::FUSE_SUPER_MAGIC => "fuse",
        V9FS_MAGIC => "9p",
        libc::OVERLAYFS_SUPER_MAGIC => "overlay",
        _ => return None,
    })
}

fn classify_volume(name: Option<String>) -> VolumeDescription {
    let remote = name.as_deref().is_some_and(is_remote_filesystem_name);
    let volatile = name
        .as_deref()
        .is_some_and(|name| VOLATILE_FILESYSTEM_NAMES.contains(&name));
    VolumeDescription {
        name,
        remote,
        volatile,
    }
}

fn is_remote_filesystem_name(name: &str) -> bool {
    REMOTE_FILESYSTEM_NAMES.contains(&name) || REMOTE_FUSE_SUBTYPES.contains(&name)
}

/// `u32` → `__fsword_t`, matching libc's own `u32_cast_long`: the top-bit
/// magics must reinterpret rather than sign-extend a signed literal.
const fn magic(value: u32) -> libc::c_long {
    value as libc::c_long
}

fn query_error(capability: PlatformCapability, operation: &'static str) -> CheckedFsError {
    let source = io::Error::last_os_error();
    match source.raw_os_error() {
        Some(libc::EOPNOTSUPP | libc::ENOSYS | libc::ENOTTY | libc::EINVAL) => {
            CheckedFsError::unsupported(capability, source.to_string())
        }
        _ => CheckedFsError::io(operation, source),
    }
}

fn io_identity(source: io::Error) -> CheckedFsError {
    CheckedFsError::io("read Linux invocation identity", source)
}

const fn ior(kind: u32, number: u32, size: u32) -> u32 {
    (2 << 30) | (size << 16) | (kind << 8) | number
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use cap_std::fs::Dir;

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Scratch {
        root: PathBuf,
        dir: Dir,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gwz-linux-descriptor-{label}-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            Self { root, dir }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn is_hard_io(error: &CheckedFsError, expected: i32) -> bool {
        matches!(
            error,
            CheckedFsError::Io { source, .. } if source.raw_os_error() == Some(expected)
        )
    }

    fn crate_directory() -> Dir {
        Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), cap_std::ambient_authority()).unwrap()
    }

    #[test]
    fn capability_directory_descriptors_are_traversal_only_on_linux() {
        // Substrate sentinel: cap-std 4 opens directory capabilities `O_PATH`
        // on Linux (ambient opens hard-code it; checked opens add it for
        // every read-only directory open), which is the entire reason the
        // reopen seam exists. If this pin flips, the capability library
        // changed its descriptor doctrine: the reopen seam stays correct on
        // real descriptors, but revisit it together with this sentinel.
        let scratch = Scratch::new("sentinel");
        let flags = unsafe { libc::fcntl(scratch.dir.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "F_GETFL must succeed on a live capability");
        assert_eq!(
            flags & libc::O_PATH,
            libc::O_PATH,
            "cap-std directory capabilities are expected to be O_PATH on Linux"
        );
    }

    #[test]
    fn descriptor_query_fd_names_the_same_directory_object() {
        // The reopen must not change which object is being queried: `.`
        // resolves through the descriptor itself, not through any stored
        // path, so device and inode must match the capability exactly.
        let scratch = Scratch::new("same-object");
        let reopened = descriptor_query_fd(&scratch.dir).unwrap();
        let reopened_stat = rustix::fs::fstat(&reopened).unwrap();
        let capability_stat = rustix::fs::fstat(&scratch.dir).unwrap();
        assert_eq!(reopened_stat.st_dev, capability_stat.st_dev);
        assert_eq!(reopened_stat.st_ino, capability_stat.st_ino);
    }

    #[test]
    fn descriptor_query_fd_carries_descriptor_consuming_operations() {
        // Red before the seam: `fsync` on (a dup of) the capability itself
        // reports `EBADF` on every Linux filesystem.
        let scratch = Scratch::new("fsync");
        let reopened = descriptor_query_fd(&scratch.dir).unwrap();
        rustix::fs::fsync(&reopened)
            .expect("the reopened descriptor must accept descriptor-consuming operations");
    }

    #[test]
    fn a_dead_descriptor_still_fails_closed_with_ebadf() {
        // Discrimination pin: the seam must not convert genuinely invalid
        // descriptors into anything softer than a hard I/O error. A
        // descriptor number at or above `RLIMIT_NOFILE` can never be
        // allocated by the kernel (allocation ranges over [0, rlim_cur)),
        // so it is a race-free stand-in for a closed descriptor even while
        // concurrent tests open files.
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        let dead_number: RawFd = i64::try_from(limit.rlim_cur)
            .ok()
            .filter(|current| *current <= i64::from(i32::MAX - 8))
            .map_or(i32::MAX - 8, |current| current as RawFd);
        // SAFETY: the number is non-negative and unallocatable under the
        // current descriptor limit, so it cannot alias a live descriptor;
        // the kernel answers every lookup on it with `EBADF`.
        let dead = unsafe { BorrowedFd::borrow_raw(dead_number) };
        let error = descriptor_query_fd(dead).unwrap_err();
        assert!(is_hard_io(&error, libc::EBADF), "got {error:?}");
    }

    #[test]
    fn a_recycled_non_directory_descriptor_still_fails_closed() {
        // Discrimination pin: a descriptor number recycled onto some other
        // object kind is a lifecycle defect, and the `O_DIRECTORY` reopen
        // reports it as a hard `ENOTDIR` instead of proceeding.
        let scratch = Scratch::new("wrong-object");
        std::fs::write(scratch.root.join("plain"), b"payload").unwrap();
        let file = scratch.dir.open("plain").unwrap();
        let error = descriptor_query_fd(&file).unwrap_err();
        assert!(is_hard_io(&error, libc::ENOTDIR), "got {error:?}");
    }

    #[test]
    fn query_error_downgrades_the_documented_capability_refusals() {
        // The graceful `unsupported` downgrade stays reserved for substrates
        // that genuinely lack the capability (for example `FS_IOC_GETFSUUID`
        // on pre-6.9 kernels, where the ioctl does not exist, reports `ENOTTY` on a real descriptor).
        unsafe { *libc::__errno_location() = libc::ENOTTY };
        let error = query_error(PlatformCapability::PersistentFilesystemIdentity, "probe");
        assert!(
            matches!(error, CheckedFsError::Unsupported { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn query_error_keeps_bad_descriptors_as_hard_io_errors() {
        // EBADF is deliberately NOT in the downgrade allowlist: with the
        // reopen seam in place, a bad descriptor can only mean a genuinely
        // dead or recycled descriptor — a defect that must stay loud rather
        // than masquerade as a graceful capability downgrade.
        unsafe { *libc::__errno_location() = libc::EBADF };
        let error = query_error(PlatformCapability::PersistentFilesystemIdentity, "probe");
        assert!(is_hard_io(&error, libc::EBADF), "got {error:?}");
    }

    #[test]
    fn dir_identity_never_reports_ebadf_for_a_live_capability() {
        // Red before the seam: every Linux `dir_identity` died at the UUID
        // ioctl with `EBADF` on the `O_PATH` capability, independent of the
        // filesystem (linux.rs:126 × 50 on the ARM64 matrix). Post-seam the
        // outcome is substrate-dependent — identity where the volume answers
        // `FS_IOC_GETFSUUID` and `name_to_handle_at`, the documented
        // `unsupported` downgrade elsewhere — but `EBADF` is impossible for a
        // live capability.
        let scratch = Scratch::new("dir-identity");
        if let Err(error) = dir_identity(&scratch.dir) {
            assert!(!is_hard_io(&error, libc::EBADF), "got {error:?}");
        }
    }

    #[test]
    fn parent_mode_never_reports_ebadf_for_a_live_capability() {
        // Same substrate class as `dir_identity`: `FS_IOC_GETFLAGS` is a
        // descriptor-consuming ioctl, so the pre-seam capability descriptor
        // reported `EBADF` before the filesystem could answer.
        let scratch = Scratch::new("parent-mode");
        if let Err(error) = parent_mode(&scratch.dir) {
            assert!(!is_hard_io(&error, libc::EBADF), "got {error:?}");
        }
    }

    #[test]
    fn mountinfo_names_the_row_whose_mount_id_matches() {
        // DR-1 W2 (charter §3.3): the row is selected by MOUNT ID, not by
        // path prefix — two mounts of the same device differ only there.
        let table = "\
23 28 0:22 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
36 25 8:1 / /home rw,relatime shared:29 - ext4 /dev/sda1 rw,data=ordered
";
        assert_eq!(mountinfo_fstype(table, 36), Some("ext4"));
        assert_eq!(mountinfo_fstype(table, 23), Some("proc"));
        assert_eq!(mountinfo_fstype(table, 99), None);
    }

    #[test]
    fn mountinfo_counts_fields_past_an_escaped_mount_point() {
        // The kernel escapes space, tab, newline and backslash in the root and
        // mount-point fields, so a mount point containing a space is still ONE
        // whitespace-delimited field and the separator search lands on the real
        // `-`. A parser that split on raw spaces would read `disk` as the mount
        // options and `-` as an optional field, and would name the wrong type.
        let table = "42 25 8:2 /sub\\040dir /mnt/my\\040disk rw,relatime - btrfs /dev/sdb1 rw\n";
        assert_eq!(mountinfo_fstype(table, 42), Some("btrfs"));
    }

    #[test]
    fn mountinfo_skips_every_optional_field_before_the_separator() {
        // Fields 7.. are variable in number and may be absent entirely; both
        // shapes must reach the same filesystem-type field.
        let many = "51 25 0:44 / /srv rw shared:1 master:2 propagate_from:3 unbindable - nfs4 srv:/export rw\n";
        let none = "52 25 0:45 / /srv2 rw - nfs4 srv:/export2 rw\n";
        assert_eq!(mountinfo_fstype(many, 51), Some("nfs4"));
        assert_eq!(mountinfo_fstype(none, 52), Some("nfs4"));
    }

    #[test]
    fn mountinfo_keeps_the_fuse_subtype() {
        // The whole reason the mountinfo row beats the superblock magic:
        // every FUSE mount reports `FUSE_SUPER_MAGIC`, so only this field
        // distinguishes a remote `fuse.sshfs` from a local `fuse.ntfs-3g`.
        let table = "77 25 0:59 / /mnt/remote rw,nosuid,nodev - fuse.sshfs user@host:/ rw\n";
        assert_eq!(mountinfo_fstype(table, 77), Some("fuse.sshfs"));
    }

    #[test]
    fn the_magic_table_names_the_charters_filesystems() {
        // Charter §3.3's fallback table, and the honest `None` outside it.
        for (magic, expected) in [
            (libc::EXT4_SUPER_MAGIC, "ext4"),
            (XFS_SUPER_MAGIC, "xfs"),
            (libc::BTRFS_SUPER_MAGIC, "btrfs"),
            (libc::F2FS_SUPER_MAGIC, "f2fs"),
            (ZFS_SUPER_MAGIC, "zfs"),
            (libc::TMPFS_MAGIC, "tmpfs"),
            (RAMFS_MAGIC, "ramfs"),
            (libc::NFS_SUPER_MAGIC, "nfs"),
            (CIFS_SUPER_MAGIC, "cifs"),
            (SMB2_SUPER_MAGIC, "smb2"),
            (CEPH_SUPER_MAGIC, "ceph"),
            (libc::FUSE_SUPER_MAGIC, "fuse"),
            (V9FS_MAGIC, "9p"),
            (libc::OVERLAYFS_SUPER_MAGIC, "overlay"),
        ] {
            assert_eq!(magic_filesystem_name(magic), Some(expected));
        }
        assert_eq!(magic_filesystem_name(libc::PROC_SUPER_MAGIC), None);
        // The top-bit magics must not sign-extend: `f_type` carries the
        // kernel's `unsigned long` value widened into `__fsword_t`.
        assert_eq!(CIFS_SUPER_MAGIC, 0xff53_4d42);
        assert_eq!(RAMFS_MAGIC, 0x8584_58f6);
    }

    #[test]
    fn volume_classification_marks_only_the_documented_network_and_volatile_names() {
        for remote in ["nfs", "nfs4", "cifs", "smb2", "smb3", "9p", "afs", "ceph"] {
            let description = classify_volume(Some(remote.to_owned()));
            assert!(description.remote, "{remote} must classify remote");
            assert!(!description.volatile, "{remote} is not volatile");
        }
        for remote in REMOTE_FUSE_SUBTYPES {
            assert!(
                classify_volume(Some(remote.to_owned())).remote,
                "{remote} must classify remote"
            );
        }
        for volatile in ["tmpfs", "ramfs"] {
            let description = classify_volume(Some(volatile.to_owned()));
            assert!(description.volatile, "{volatile} must classify volatile");
            assert!(!description.remote, "{volatile} is not remote");
        }
        // `fuse` alone says nothing about locality, and a local filesystem is
        // neither. `None` — rendered `unknown` — claims nothing either.
        for local in ["ext4", "xfs", "btrfs", "f2fs", "zfs", "overlay", "fuse"] {
            let description = classify_volume(Some(local.to_owned()));
            assert!(!description.remote, "{local} must not classify remote");
            assert!(!description.volatile, "{local} must not classify volatile");
        }
        let unknown = classify_volume(None);
        assert!(unknown.name.is_none() && !unknown.remote && !unknown.volatile);
    }

    #[test]
    fn dir_identity_refuses_a_volatile_filesystem() {
        // Charter §3.2 on a REAL tmpfs: identity alone would admit it (tmpfs
        // publishes a random per-mount UUID from the ioctl's first kernel), so
        // the refusal must come from the volatility test in front of the
        // ioctl, with the charter's own detail string.
        let Ok(dir) = Dir::open_ambient_dir("/dev/shm", cap_std::ambient_authority()) else {
            println!("skipped: /dev/shm is not present on this host");
            return;
        };
        let f_type = filesystem_type(dir.as_raw_fd()).unwrap();
        if f_type != libc::TMPFS_MAGIC {
            println!("skipped: /dev/shm is not tmpfs on this host (f_type {f_type:#x})");
            return;
        }
        let error = dir_identity(&dir).expect_err("a volatile volume is not admitted");
        assert!(
            matches!(
                &error,
                CheckedFsError::Unsupported { capability, detail }
                    if *capability == PlatformCapability::PersistentFilesystemIdentity
                        && detail == "volatile filesystem: contents do not survive power loss"
            ),
            "got {error:?}"
        );
        // And the description agrees, which is what words W3's warning.
        let description = describe_volume(&dir).unwrap();
        assert_eq!(description.name.as_deref(), Some("tmpfs"));
        assert!(description.volatile && !description.remote);
    }

    #[test]
    fn describe_volume_names_the_volume_the_crate_lives_on() {
        // The positive control for the wording aid on whatever the CI host
        // actually is: it must produce a name (mountinfo first, magic table
        // second) and must not call a checked-out source tree volatile.
        let description = describe_volume(&crate_directory()).unwrap();
        assert!(
            description.name.is_some(),
            "the crate's own volume must be nameable"
        );
        assert!(!description.volatile, "got {:?}", description.name);
    }
}
