use super::*;
use crate::filesystem::{
    FsLegacyDurableIdentity as DurableObjectIdentity,
    FsLegacyInvocationIdentity as InvocationObjectIdentity,
    FsLegacyObjectIdentity as ObjectIdentity, FsLegacyRenameDomain as RenameDomainProof,
};
pub(super) use platform::*;
fn persistent_identity_unsupported() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "persistent filesystem identity is unsupported",
    )
}

/// The same refusal for a probe that failed with an `errno`.
///
/// The downgrade allowlist is the catalog provider's
/// (`provider/platform/linux.rs`'s `query_error`): only substrates that
/// genuinely lack the capability are converted. Everything else — `EBADF` and
/// its lifecycle-defect siblings above all — stays a loud raw error, because a
/// dead descriptor must never masquerade as a graceful capability downgrade.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn persistent_identity_error(source: std::io::Error) -> std::io::Error {
    #[cfg(target_os = "linux")]
    const DOWNGRADED: &[i32] = &[libc::EOPNOTSUPP, libc::ENOSYS, libc::ENOTTY, libc::EINVAL];
    #[cfg(target_os = "macos")]
    const DOWNGRADED: &[i32] = &[libc::ENOTSUP, libc::EINVAL, libc::ENOTTY];

    match source.raw_os_error() {
        Some(code) if DOWNGRADED.contains(&code) => persistent_identity_unsupported(),
        _ => source,
    }
}

pub(super) fn path_identity(root: &Dir, relative: &Path) -> std::io::Result<Vec<u8>> {
    let sensitive = platform::case_sensitive(root)?;
    super::super::retained::encode_path_identity(relative, |component| {
        platform::component_identity(component, sensitive)
    })
}

#[cfg(unix)]
mod unix {
    use std::os::fd::{AsFd, AsRawFd};

    pub(crate) fn invocation_identity(
        value: &impl AsFd,
    ) -> std::io::Result<super::InvocationObjectIdentity> {
        let stat = rustix::fs::fstat(value)?;
        Ok(super::InvocationObjectIdentity::Unix {
            device: stat.st_dev as u64,
            inode: stat.st_ino as u64,
        })
    }

    pub(crate) fn mounted_filesystem_id(value: &impl AsRawFd) -> std::io::Result<[u8; 8]> {
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { libc::fstatfs(value.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        let mut output = [0_u8; 8];
        let source = std::ptr::addr_of!(stat.f_fsid).cast::<u8>();
        let count = output.len().min(std::mem::size_of_val(&stat.f_fsid));
        unsafe { std::ptr::copy_nonoverlapping(source, output.as_mut_ptr(), count) };
        Ok(output)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::OsStr;
    use std::os::fd::{AsFd, AsRawFd};
    use std::os::unix::ffi::OsStrExt;

    use cap_std::fs::{Dir, File};

    use super::{DurableObjectIdentity, ObjectIdentity, RenameDomainProof, unix};

    const MAX_HANDLE_BYTES: usize = 128;

    #[repr(C)]
    struct LinuxFileHandle {
        handle_bytes: u32,
        handle_type: i32,
        bytes: [u8; MAX_HANDLE_BYTES],
    }

    fn identity(value: &(impl AsFd + AsRawFd)) -> std::io::Result<ObjectIdentity> {
        let mut handle = LinuxFileHandle {
            handle_bytes: MAX_HANDLE_BYTES as u32,
            handle_type: 0,
            bytes: [0; MAX_HANDLE_BYTES],
        };
        let mut mount_id = 0;
        let empty = c"";
        let result = unsafe {
            libc::name_to_handle_at(
                value.as_raw_fd(),
                empty.as_ptr(),
                std::ptr::addr_of_mut!(handle).cast::<libc::file_handle>(),
                &mut mount_id,
                libc::AT_EMPTY_PATH,
            )
        };
        if result != 0 {
            return Err(super::persistent_identity_error(
                std::io::Error::last_os_error(),
            ));
        }
        let handle_length = handle.handle_bytes as usize;
        if handle_length == 0 || handle_length > MAX_HANDLE_BYTES {
            return Err(super::persistent_identity_unsupported());
        }
        Ok(ObjectIdentity {
            durable: DurableObjectIdentity::Linux {
                filesystem_id: unix::mounted_filesystem_id(value)?.to_vec(),
                handle_type: handle.handle_type,
                file_handle: handle.bytes[..handle_length].to_vec(),
            },
            invocation: unix::invocation_identity(value)?,
        })
    }

    pub(crate) fn dir_object_identity(dir: &Dir) -> std::io::Result<ObjectIdentity> {
        identity(dir)
    }

    pub(crate) fn file_object_identity(file: &File) -> std::io::Result<ObjectIdentity> {
        identity(file)
    }

    pub(crate) fn rename_domain(dir: &Dir) -> std::io::Result<RenameDomainProof> {
        let stat = rustix::fs::statx(
            dir.as_fd(),
            "",
            rustix::fs::AtFlags::EMPTY_PATH,
            rustix::fs::StatxFlags::MNT_ID,
        )?;
        if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 {
            return Err(super::persistent_identity_unsupported());
        }
        Ok(RenameDomainProof::LinuxMountId(stat.stx_mnt_id))
    }

    pub(crate) fn case_sensitive(_root: &Dir) -> std::io::Result<bool> {
        Ok(true)
    }

    pub(crate) fn component_identity(
        component: &OsStr,
        _case_sensitive: bool,
    ) -> std::io::Result<Vec<u8>> {
        Ok(component.as_bytes().to_vec())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::OsStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    use cap_std::fs::{Dir, File};
    use unicode_normalization::UnicodeNormalization;

    use super::{DurableObjectIdentity, ObjectIdentity, RenameDomainProof, unix};

    #[repr(C, packed(4))]
    struct MacObjectAttributes {
        length: u32,
        persistent_object_id: [u8; 8],
    }

    #[repr(C, packed(4))]
    struct MacVolumeAttributes {
        length: u32,
        capabilities: libc::vol_capabilities_attr_t,
        volume_uuid: [u8; 16],
    }

    fn object_attributes(value: &impl AsRawFd) -> std::io::Result<MacObjectAttributes> {
        let mut list = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_OBJPERMANENTID,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut attributes = std::mem::MaybeUninit::<MacObjectAttributes>::zeroed();
        if unsafe {
            libc::fgetattrlist(
                value.as_raw_fd(),
                std::ptr::addr_of_mut!(list).cast(),
                attributes.as_mut_ptr().cast(),
                std::mem::size_of::<MacObjectAttributes>(),
                0,
            )
        } != 0
        {
            return Err(super::persistent_identity_error(
                std::io::Error::last_os_error(),
            ));
        }
        let attributes = unsafe { attributes.assume_init() };
        if attributes.length as usize != std::mem::size_of::<MacObjectAttributes>() {
            return Err(super::persistent_identity_unsupported());
        }
        Ok(attributes)
    }

    fn volume_attributes(value: &impl AsRawFd) -> std::io::Result<MacVolumeAttributes> {
        let mut list = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES | libc::ATTR_VOL_UUID,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut attributes = std::mem::MaybeUninit::<MacVolumeAttributes>::zeroed();
        if unsafe {
            libc::fgetattrlist(
                value.as_raw_fd(),
                std::ptr::addr_of_mut!(list).cast(),
                attributes.as_mut_ptr().cast(),
                std::mem::size_of::<MacVolumeAttributes>(),
                0,
            )
        } != 0
        {
            return Err(super::persistent_identity_error(
                std::io::Error::last_os_error(),
            ));
        }
        let attributes = unsafe { attributes.assume_init() };
        if attributes.length as usize != std::mem::size_of::<MacVolumeAttributes>() {
            return Err(super::persistent_identity_unsupported());
        }
        let capabilities = unsafe { std::ptr::addr_of!(attributes.capabilities).read_unaligned() };
        let format = libc::VOL_CAPABILITIES_FORMAT;
        if capabilities.valid[format] & libc::VOL_CAP_FMT_PERSISTENTOBJECTIDS == 0
            || capabilities.capabilities[format] & libc::VOL_CAP_FMT_PERSISTENTOBJECTIDS == 0
        {
            return Err(super::persistent_identity_unsupported());
        }
        Ok(attributes)
    }

    fn identity(value: &(impl std::os::fd::AsFd + AsRawFd)) -> std::io::Result<ObjectIdentity> {
        let object = object_attributes(value)?;
        let volume = volume_attributes(value)?;
        Ok(ObjectIdentity {
            durable: DurableObjectIdentity::Mac {
                volume_uuid: volume.volume_uuid,
                persistent_object_id: object.persistent_object_id,
            },
            invocation: unix::invocation_identity(value)?,
        })
    }

    pub(crate) fn dir_object_identity(dir: &Dir) -> std::io::Result<ObjectIdentity> {
        identity(dir)
    }

    pub(crate) fn file_object_identity(file: &File) -> std::io::Result<ObjectIdentity> {
        identity(file)
    }

    pub(crate) fn rename_domain(dir: &Dir) -> std::io::Result<RenameDomainProof> {
        Ok(RenameDomainProof::MacMountedFileSystem(
            unix::mounted_filesystem_id(dir)?,
        ))
    }

    pub(crate) fn case_sensitive(root: &Dir) -> std::io::Result<bool> {
        let attributes = volume_attributes(root)?;
        let capabilities = unsafe { std::ptr::addr_of!(attributes.capabilities).read_unaligned() };
        let format = libc::VOL_CAPABILITIES_FORMAT;
        if capabilities.valid[format] & libc::VOL_CAP_FMT_CASE_SENSITIVE == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "filesystem does not report path case-equivalence rules",
            ));
        }
        Ok(capabilities.capabilities[format] & libc::VOL_CAP_FMT_CASE_SENSITIVE != 0)
    }

    pub(crate) fn component_identity(
        component: &OsStr,
        case_sensitive: bool,
    ) -> std::io::Result<Vec<u8>> {
        let bytes = component.as_bytes();
        let text = std::str::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "non-UTF-8 macOS path identity is unsupported",
            )
        })?;
        let normalized = text.nfd().collect::<String>();
        Ok(if case_sensitive {
            normalized.into_bytes()
        } else {
            normalized.to_lowercase().into_bytes()
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;

    use cap_std::fs::{Dir, File};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
        VOLUME_NAME_GUID,
    };

    use super::{
        DurableObjectIdentity, InvocationObjectIdentity, ObjectIdentity, RenameDomainProof,
    };

    fn facts(value: &impl AsRawHandle) -> std::io::Result<(Vec<u16>, [u8; 16])> {
        let handle = value.as_raw_handle();
        let mut info = FILE_ID_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                std::ptr::addr_of_mut!(info).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let mut path = vec![0_u16; 1024];
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                path.as_mut_ptr(),
                path.len() as u32,
                VOLUME_NAME_GUID,
            )
        };
        if length == 0 || length as usize >= path.len() {
            return Err(std::io::Error::last_os_error());
        }
        path.truncate(length as usize);
        let separator = path
            .iter()
            .enumerate()
            .skip(4)
            .find_map(|(index, unit)| (*unit == b'\\' as u16).then_some(index))
            .ok_or_else(super::persistent_identity_unsupported)?;
        path.truncate(separator);
        Ok((path, info.FileId.Identifier))
    }

    fn identity(value: &impl AsRawHandle) -> std::io::Result<ObjectIdentity> {
        let (volume_guid, file_id) = facts(value)?;
        Ok(ObjectIdentity {
            durable: DurableObjectIdentity::Windows {
                volume_guid: volume_guid.clone(),
                file_id,
            },
            invocation: InvocationObjectIdentity::Windows {
                volume_guid,
                file_id,
            },
        })
    }

    pub(crate) fn dir_object_identity(dir: &Dir) -> std::io::Result<ObjectIdentity> {
        identity(dir)
    }

    pub(crate) fn file_object_identity(file: &File) -> std::io::Result<ObjectIdentity> {
        identity(file)
    }

    pub(crate) fn rename_domain(dir: &Dir) -> std::io::Result<RenameDomainProof> {
        let (volume_guid, _) = facts(dir)?;
        Ok(RenameDomainProof::WindowsMountedVolume(volume_guid))
    }

    pub(crate) fn case_sensitive(_root: &Dir) -> std::io::Result<bool> {
        Ok(false)
    }

    pub(crate) fn component_identity(
        component: &OsStr,
        _case_sensitive: bool,
    ) -> std::io::Result<Vec<u8>> {
        let text =
            String::from_utf16(&component.encode_wide().collect::<Vec<_>>()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "non-Unicode Windows path identity is unsupported",
                )
            })?;
        Ok(text
            .to_lowercase()
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use std::ffi::OsStr;

    use cap_std::fs::{Dir, File};

    use super::{ObjectIdentity, RenameDomainProof};

    fn unsupported() -> std::io::Error {
        super::persistent_identity_unsupported()
    }

    pub(crate) fn dir_object_identity(_dir: &Dir) -> std::io::Result<ObjectIdentity> {
        Err(unsupported())
    }

    pub(crate) fn file_object_identity(_file: &File) -> std::io::Result<ObjectIdentity> {
        Err(unsupported())
    }

    pub(crate) fn rename_domain(_dir: &Dir) -> std::io::Result<RenameDomainProof> {
        Err(unsupported())
    }

    pub(crate) fn case_sensitive(_root: &Dir) -> std::io::Result<bool> {
        Err(unsupported())
    }

    pub(crate) fn component_identity(
        _component: &OsStr,
        _case_sensitive: bool,
    ) -> std::io::Result<Vec<u8>> {
        Err(unsupported())
    }
}
