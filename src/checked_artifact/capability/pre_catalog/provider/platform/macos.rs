use std::io;
use std::os::fd::{AsRawFd, RawFd};

use cap_fs_ext::MetadataExt;
use cap_std::fs::{Dir, File};

use super::super::super::*;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

#[repr(C, packed(4))]
struct ObjectAttributes {
    length: u32,
    persistent_object_id: [u8; 8],
}

#[repr(C, packed(4))]
struct VolumeAttributes {
    length: u32,
    capabilities: libc::vol_capabilities_attr_t,
    volume_uuid: [u8; 16],
}

pub(super) const fn support_profile() -> SupportedFilesystemProfile {
    SupportedFilesystemProfile::MacPersistentObjectIdV1
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
    set_errno(0);
    let result = unsafe { libc::fpathconf(parent.as_raw_fd(), libc::_PC_CASE_SENSITIVE) };
    match result {
        0 => Ok(PathComponentMode::AsciiCaseFold),
        1 => Ok(PathComponentMode::Sensitive),
        -1 if errno() == 0 => Err(CheckedFsError::unsupported(
            PlatformCapability::PathEquivalence,
            "filesystem does not report per-parent lookup mode",
        )),
        -1 => Err(query_error(
            PlatformCapability::PathEquivalence,
            "query macOS parent lookup mode",
        )),
        _ => Err(CheckedFsError::unsupported(
            PlatformCapability::PathEquivalence,
            "filesystem returned a noncanonical lookup mode",
        )),
    }
}

pub(super) fn rename_domain(directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
    Ok(volume_attributes(directory.as_raw_fd())?
        .volume_uuid
        .to_vec())
}

fn identity(
    fd: RawFd,
    metadata: &impl MetadataExt,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    let object = object_attributes(fd)?;
    let volume = volume_attributes(fd)?;
    let durable = DurableObjectIdentityV1::mac(volume.volume_uuid, object.persistent_object_id)?;
    let mut invocation = Vec::with_capacity(16);
    invocation.extend_from_slice(&metadata.dev().to_be_bytes());
    invocation.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(ObjectIdentityFact::new(durable, invocation))
}

fn object_attributes(fd: RawFd) -> Result<ObjectAttributes, CheckedFsError> {
    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_OBJPERMANENTID,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut attributes = std::mem::MaybeUninit::<ObjectAttributes>::zeroed();
    if unsafe {
        libc::fgetattrlist(
            fd,
            std::ptr::addr_of_mut!(list).cast(),
            attributes.as_mut_ptr().cast(),
            std::mem::size_of::<ObjectAttributes>(),
            0,
        )
    } != 0
    {
        return Err(query_error(
            PlatformCapability::DurableObjectIdentity,
            "query macOS persistent object identity",
        ));
    }
    let attributes = unsafe { attributes.assume_init() };
    if attributes.length as usize != std::mem::size_of::<ObjectAttributes>() {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "filesystem returned a noncanonical object identity",
        ));
    }
    Ok(attributes)
}

fn volume_attributes(fd: RawFd) -> Result<VolumeAttributes, CheckedFsError> {
    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut attributes = std::mem::MaybeUninit::<VolumeAttributes>::zeroed();
    if unsafe {
        libc::fgetattrlist(
            fd,
            std::ptr::addr_of_mut!(list).cast(),
            attributes.as_mut_ptr().cast(),
            std::mem::size_of::<VolumeAttributes>(),
            0,
        )
    } != 0
    {
        return Err(query_error(
            PlatformCapability::DurableObjectIdentity,
            "query macOS volume identity",
        ));
    }
    let attributes = unsafe { attributes.assume_init() };
    let capabilities = unsafe { std::ptr::addr_of!(attributes.capabilities).read_unaligned() };
    let format = libc::VOL_CAPABILITIES_FORMAT;
    if attributes.length as usize != std::mem::size_of::<VolumeAttributes>()
        || capabilities.valid[format] & libc::VOL_CAP_FMT_PERSISTENTOBJECTIDS == 0
        || capabilities.capabilities[format] & libc::VOL_CAP_FMT_PERSISTENTOBJECTIDS == 0
    {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "filesystem does not promise persistent object identities",
        ));
    }
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(fd, stat.as_mut_ptr()) } != 0 {
        return Err(query_error(
            PlatformCapability::DurableObjectIdentity,
            "query macOS mounted filesystem",
        ));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_flags & libc::MNT_LOCAL as u32 == 0 {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::DurableObjectIdentity,
            "remote macOS filesystems are not an admitted profile",
        ));
    }
    Ok(attributes)
}

fn query_error(capability: PlatformCapability, operation: &'static str) -> CheckedFsError {
    let source = io::Error::last_os_error();
    match source.raw_os_error() {
        Some(libc::ENOTSUP | libc::EINVAL | libc::ENOTTY) => {
            CheckedFsError::unsupported(capability, source.to_string())
        }
        _ => CheckedFsError::io(operation, source),
    }
}

fn io_identity(source: io::Error) -> CheckedFsError {
    CheckedFsError::io("read macOS invocation identity", source)
}

fn errno() -> i32 {
    unsafe { *libc::__error() }
}

fn set_errno(value: i32) {
    unsafe { *libc::__error() = value };
}
