use std::io;
use std::os::windows::io::AsRawHandle;

use cap_std::fs::{Dir, File};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_CASE_SENSITIVE_INFO, FILE_ID_INFO, FileCaseSensitiveInfo, FileIdInfo,
    GetFileInformationByHandleEx, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
    VOLUME_NAME_GUID,
};

use super::super::super::*;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

pub(super) const fn support_profile() -> SupportedFilesystemProfile {
    SupportedFilesystemProfile::WindowsNtfsFileId128V1
}

pub(super) fn dir_identity(
    directory: &Dir,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    identity(directory)
}

pub(super) fn file_identity(
    file: &File,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    identity(file)
}

pub(super) fn parent_mode(parent: &Dir) -> Result<PathComponentMode, CheckedFsError> {
    let mut info = FILE_CASE_SENSITIVE_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            parent.as_raw_handle(),
            FileCaseSensitiveInfo,
            std::ptr::addr_of_mut!(info).cast(),
            std::mem::size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    } == 0
    {
        return Err(query_error(
            PlatformCapability::PathEquivalence,
            "query NTFS per-directory case mode",
        ));
    }
    Ok(if info.Flags & FILE_CS_FLAG_CASE_SENSITIVE_DIR == 0 {
        PathComponentMode::AsciiCaseFold
    } else {
        PathComponentMode::Sensitive
    })
}

pub(super) fn rename_domain(directory: &Dir) -> Result<Vec<u8>, CheckedFsError> {
    let (volume_guid, _) = facts(directory)?;
    Ok(encode_utf16(&volume_guid))
}

fn identity(
    value: &impl AsRawHandle,
) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>, CheckedFsError> {
    let (volume_guid, file_id) = facts(value)?;
    let durable = DurableObjectIdentityV1::windows_ntfs(volume_guid.clone(), file_id)?;
    let mut invocation = encode_utf16(&volume_guid);
    invocation.extend_from_slice(&file_id);
    Ok(ObjectIdentityFact::new(durable, invocation))
}

fn facts(value: &impl AsRawHandle) -> Result<(Vec<u16>, [u8; 16]), CheckedFsError> {
    let handle = value.as_raw_handle();
    require_ntfs(handle)?;
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
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query NTFS 128-bit file identity",
        ));
    }
    let volume_guid = volume_guid(handle)?;
    if info.FileId.Identifier == [0; 16] {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "NTFS returned a zero file identity",
        ));
    }
    Ok((volume_guid, info.FileId.Identifier))
}

fn require_ntfs(handle: std::os::windows::io::RawHandle) -> Result<(), CheckedFsError> {
    let mut filesystem = [0_u16; 32];
    if unsafe {
        GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    } == 0
    {
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query Windows filesystem profile",
        ));
    }
    let length = filesystem
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(filesystem.len());
    let name = String::from_utf16(&filesystem[..length]).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "filesystem name is not valid UTF-16",
        )
    })?;
    if name != "NTFS" {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "only local NTFS is an admitted Windows profile",
        ));
    }
    Ok(())
}

fn volume_guid(handle: std::os::windows::io::RawHandle) -> Result<Vec<u16>, CheckedFsError> {
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
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query local NTFS volume GUID",
        ));
    }
    path.truncate(length as usize);
    let prefix = "\\\\?\\Volume{".encode_utf16().collect::<Vec<_>>();
    if !path.starts_with(&prefix) {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "opened object is not on a local volume GUID path",
        ));
    }
    let separator = path
        .iter()
        .enumerate()
        .skip(prefix.len())
        .find_map(|(index, unit)| (*unit == b'\\' as u16).then_some(index))
        .ok_or_else(|| {
            CheckedFsError::unsupported(
                PlatformCapability::PersistentFilesystemIdentity,
                "opened object did not expose a complete volume GUID",
            )
        })?;
    path.truncate(separator);
    Ok(path)
}

fn encode_utf16(value: &[u16]) -> Vec<u8> {
    value.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn query_error(capability: PlatformCapability, operation: &'static str) -> CheckedFsError {
    let source = io::Error::last_os_error();
    match source.raw_os_error() {
        Some(1 | 50 | 87) => CheckedFsError::unsupported(capability, source.to_string()),
        _ => CheckedFsError::io(operation, source),
    }
}
