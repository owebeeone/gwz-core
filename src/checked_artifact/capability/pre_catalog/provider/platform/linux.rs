use std::io;
use std::os::fd::{AsRawFd, RawFd};

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
    identity(
        directory.as_raw_fd(),
        &directory.dir_metadata().map_err(io_identity)?,
    )
}

pub(super) fn file_identity(
    file: &File,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    identity(file.as_raw_fd(), &file.metadata().map_err(io_identity)?)
}

pub(super) fn parent_mode(parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
    require_ext4(parent.as_raw_fd())?;
    let mut flags: libc::c_long = 0;
    if unsafe { libc::ioctl(parent.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) } != 0 {
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
        Some(code)
            if matches!(
                code,
                libc::EOPNOTSUPP | libc::ENOSYS | libc::ENOTTY | libc::EINVAL
            ) =>
        {
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
