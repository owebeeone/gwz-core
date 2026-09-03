use std::io;
use std::os::fd::{AsRawFd, RawFd};

use cap_fs_ext::MetadataExt;
use cap_std::fs::{Dir, File};

use super::super::super::*;
use super::VolumeDescription;
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

/// The volume's name and its two classifications, as a WORDING AID only
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03).
///
/// macOS names the filesystem directly (`statfs.f_fstypename`: `apfs`, `hfs`,
/// `smbfs`, `nfs`, `msdos`) and states locality directly (`MNT_LOCAL`), so
/// neither classification needs a name list here — which is why `remote` is
/// read from the mount flag rather than from `f_fstypename`.
///
/// `volatile` is always `false`, and that is a LIMIT, not a claim: a macOS RAM
/// disk (`hdiutil attach -nomount ram://…` newfs'd and mounted) reports `apfs`
/// or `hfs` with `MNT_LOCAL` set and is indistinguishable from a disk-backed
/// volume through this interface. The consequence is disclosed rather than
/// hidden: a merge on such a volume takes the ABOVE-bar path and activates a
/// catalog whose contents vanish on power loss, exactly as it does today. The
/// identity gate on macOS is unchanged by this step; only Linux gained a
/// volatility refusal (§3.2), because only there does an identity probe
/// actively admit a volatile substrate.
pub(super) fn describe_volume(directory: &Dir) -> Result<VolumeDescription, CheckedFsError> {
    let stat = mounted_filesystem(directory.as_raw_fd())?;
    let length = stat
        .f_fstypename
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(stat.f_fstypename.len());
    let bytes = stat.f_fstypename[..length]
        .iter()
        .map(|unit| *unit as u8)
        .collect::<Vec<_>>();
    let name = String::from_utf8_lossy(&bytes).into_owned();
    Ok(VolumeDescription {
        name: (!name.is_empty()).then_some(name),
        remote: stat.f_flags & libc::MNT_LOCAL as u32 == 0,
        volatile: false,
    })
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
            PlatformCapability::PersistentFilesystemIdentity,
            "query macOS persistent object identity",
        ));
    }
    let attributes = unsafe { attributes.assume_init() };
    if attributes.length as usize != std::mem::size_of::<ObjectAttributes>() {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
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
            PlatformCapability::PersistentFilesystemIdentity,
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
            PlatformCapability::PersistentFilesystemIdentity,
            "filesystem does not promise persistent object identities",
        ));
    }
    if mounted_filesystem(fd)?.f_flags & libc::MNT_LOCAL as u32 == 0 {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "remote macOS filesystems are not an admitted profile",
        ));
    }
    Ok(attributes)
}

/// The one `fstatfs` both the admission gate and the volume description read.
/// The gate consumes `f_flags & MNT_LOCAL`; the description also reads
/// `f_fstypename`, so factoring it keeps one error string for one syscall.
fn mounted_filesystem(fd: RawFd) -> Result<libc::statfs, CheckedFsError> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(fd, stat.as_mut_ptr()) } != 0 {
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query macOS mounted filesystem",
        ));
    }
    Ok(unsafe { stat.assume_init() })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_volume_names_a_local_apple_volume() {
        // DR-1 W2 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03):
        // the positive control for the wording aid on the macOS CI host. A
        // checked-out source tree is on the boot volume, so `f_fstypename` is
        // one of the two Apple formats, `MNT_LOCAL` is set, and the platform
        // reports no volatility at all.
        let directory =
            Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), cap_std::ambient_authority())
                .unwrap();
        let description = describe_volume(&directory).unwrap();
        let name = description
            .name
            .expect("macOS always names the mounted filesystem");
        assert!(matches!(name.as_str(), "apfs" | "hfs"), "got {name}");
        assert!(!description.remote, "the crate's volume is local");
        assert!(!description.volatile, "macOS never reports volatility");
    }
}
