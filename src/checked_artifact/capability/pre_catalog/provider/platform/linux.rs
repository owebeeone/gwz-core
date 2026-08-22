use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};

use cap_fs_ext::MetadataExt;
use cap_std::fs::{Dir, File};

use super::super::super::*;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

const EXT4_SUPER_MAGIC: libc::c_long = 0xEF53;
const FS_CASEFOLD_FL: libc::c_long = 0x4000_0000;
const MAX_HANDLE_BYTES: usize = 128;
const FS_IOC_GETFSUUID: libc::c_ulong = ior(0x15, 0, 17) as libc::c_ulong;
const FS_IOC_GETFLAGS: libc::c_ulong =
    ior(b'f' as u32, 1, std::mem::size_of::<libc::c_long>() as u32) as libc::c_ulong;

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
    let queryable = descriptor_query_fd(parent)?;
    require_ext4(queryable.as_raw_fd())?;
    let mut flags: libc::c_long = 0;
    if unsafe { libc::ioctl(queryable.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) } != 0 {
        return Err(query_error(
            PlatformCapability::PathEquivalence,
            "query ext4 directory flags",
        ));
    }
    Ok(if flags & FS_CASEFOLD_FL == 0 {
        PathComponentMode::Sensitive
    } else {
        PathComponentMode::AsciiCaseFold
    })
}

pub(super) fn rename_domain(directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
    require_ext4(directory.as_raw_fd())?;
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
    if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::AtomicRenameDomain,
            "filesystem does not expose a mount identity",
        ));
    }
    Ok(stat.stx_mnt_id.to_be_bytes().to_vec())
}

/// Reopens a directory capability as a descriptor that carries
/// descriptor-consuming operations (`ioctl`, `fsync`).
///
/// cap-std directory capabilities are `O_PATH` descriptors on Linux. The
/// kernel resolves descriptor-consuming operations through the descriptor
/// lookup that refuses `O_PATH` files with `EBADF` before any filesystem
/// code runs, while traversal-class operations (`fstatfs`, `statx`,
/// `name_to_handle_at`, `openat`) accept them — which is why `require_ext4`
/// succeeds on the very descriptor whose UUID ioctl reported `EBADF` on the
/// ARM64 matrix. Reopening `.` through the capability performs no path
/// re-resolution — the descriptor itself anchors the lookup — so the result
/// names the same directory object (pinned by the invocation-identity test
/// below). Failures are never downgraded here: a dead descriptor reports
/// `EBADF` and a recycled non-directory descriptor reports `ENOTDIR`, both
/// as hard I/O errors, so genuine descriptor-lifecycle defects stay loud.
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

fn identity(
    fd: RawFd,
    metadata: &impl MetadataExt,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    require_ext4(fd)?;
    let uuid = filesystem_uuid(fd)?;
    let (handle_type, handle) = persistent_handle(fd)?;
    let durable = DurableObjectIdentityV1::linux_ext4(uuid, handle_type, handle)?;
    let mut invocation = Vec::with_capacity(16);
    invocation.extend_from_slice(&metadata.dev().to_be_bytes());
    invocation.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(ObjectIdentityFact::new(durable, invocation))
}

fn require_ext4(fd: RawFd) -> Result<(), CheckedFsError> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(fd, stat.as_mut_ptr()) } != 0 {
        return Err(CheckedFsError::io(
            "query Linux filesystem type",
            io::Error::last_os_error(),
        ));
    }
    if unsafe { stat.assume_init() }.f_type != EXT4_SUPER_MAGIC {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "only local ext4 with FS_IOC_GETFSUUID is admitted",
        ));
    }
    Ok(())
}

fn filesystem_uuid(fd: RawFd) -> Result<[u8; 16], CheckedFsError> {
    let mut value = FsUuid2 {
        len: 0,
        uuid: [0; 16],
    };
    if unsafe { libc::ioctl(fd, FS_IOC_GETFSUUID, &mut value) } != 0 {
        return Err(query_error(
            PlatformCapability::DurableObjectIdentity,
            "query ext4 external filesystem UUID",
        ));
    }
    if value.len != 16 || value.uuid == [0; 16] {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "ext4 returned an absent or malformed external UUID",
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
            PlatformCapability::DurableObjectIdentity,
            "query retained empty-path file handle",
        ));
    }
    let length = value.handle_bytes as usize;
    if value.handle_type <= 0 || !(1..=MAX_HANDLE_BYTES).contains(&length) {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "ext4 returned an unsupported persistent handle",
        ));
    }
    Ok((value.handle_type, value.bytes[..length].to_vec()))
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
        // on pre-6.8 kernels reports `ENOTTY` on a real descriptor).
        unsafe { *libc::__errno_location() = libc::ENOTTY };
        let error = query_error(PlatformCapability::DurableObjectIdentity, "probe");
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
        let error = query_error(PlatformCapability::DurableObjectIdentity, "probe");
        assert!(is_hard_io(&error, libc::EBADF), "got {error:?}");
    }

    #[test]
    fn dir_identity_never_reports_ebadf_for_a_live_capability() {
        // Red before the seam: every Linux `dir_identity` died at the UUID
        // ioctl with `EBADF` on the `O_PATH` capability, independent of the
        // filesystem (linux.rs:126 × 50 on the ARM64 matrix). Post-seam the
        // outcome is substrate-dependent — identity on ext4 with a 6.8+
        // kernel, the documented `unsupported` downgrade elsewhere — but
        // `EBADF` is impossible for a live capability.
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
}
