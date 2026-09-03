use std::io;
use std::os::windows::io::AsRawHandle;

use cap_std::fs::{Dir, File};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_CASE_SENSITIVE_INFO, FILE_ID_INFO, FileCaseSensitiveInfo, FileIdInfo,
    GETFINALPATHNAMEBYHANDLE_FLAGS, GetDriveTypeW, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, VOLUME_NAME_DOS, VOLUME_NAME_GUID,
};

use super::super::super::*;
use super::VolumeDescription;
use crate::checked_artifact::capability::{
    ObjectIdentityFact, PathComponentMode, PlatformCapability,
};

const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

/// `winbase.h`'s `DRIVE_REMOTE`. `windows-sys` 0.61 publishes it under
/// `Win32::System::WindowsProgramming`, a feature this crate does not enable
/// for one integer, so it is restated here with its header cited.
const DRIVE_REMOTE: u32 = 4;

/// The prefix `GetFinalPathNameByHandleW(.., VOLUME_NAME_DOS)` produces for a
/// path on a network share. It is the second of the charter's two remote
/// tests because the FIRST one cannot answer there: `VOLUME_NAME_GUID` is
/// documented as unsupported for network shares, so `GetDriveTypeW` never
/// gets a volume root to judge.
const UNC_FINAL_PATH_PREFIX: &str = r"\\?\UNC\";

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

/// The volume's name and its two classifications, as a WORDING AID only
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03).
///
/// Lenient by construction, unlike the identity path above: every probe here
/// degrades to "cannot say" rather than refusing, because a warning that
/// cannot name the filesystem still has to print (`unknown`). `remote` is the
/// OR of the charter's two tests — a `DRIVE_REMOTE` volume root, and a
/// `\\?\UNC\` final path for the shares whose volume root cannot be asked at
/// all. `volatile` is always `false`: Windows exposes no interface that
/// separates a RAM disk from a fixed volume, the same limit macOS has.
pub(super) fn describe_volume(directory: &Dir) -> Result<VolumeDescription, CheckedFsError> {
    let handle = directory.as_raw_handle();
    let remote = final_path(handle, VOLUME_NAME_DOS).is_ok_and(|path| is_unc_final_path(&path))
        || volume_guid(handle).is_ok_and(|guid| volume_root_drive_type(&guid) == DRIVE_REMOTE);
    Ok(VolumeDescription {
        name: filesystem_name(handle).ok(),
        remote,
        volatile: false,
    })
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

/// The Windows admission gate. DR-1 W2 KEEPS it (charter §3.2 and the
/// operator's ruling of 2026-09-03, §0.1 "Windows keeps `require_ntfs`"):
/// unlike Linux, this name test has no verified capability replacement yet,
/// and its replacement — `GetVolumeInformationByHandleW`'s
/// `FILE_SUPPORTS_OPEN_BY_FILE_ID` flag, which NTFS and ReFS set and
/// FAT/exFAT do not — is not to be built blind from a host that cannot run
/// the Windows matrix. It is charter §8 item 3 and the next Windows-verified
/// step's work.
fn require_ntfs(handle: std::os::windows::io::RawHandle) -> Result<(), CheckedFsError> {
    if filesystem_name(handle)? != "NTFS" {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "only local NTFS is an admitted Windows profile",
        ));
    }
    Ok(())
}

/// The one `GetVolumeInformationByHandleW` name fetch: the gate above
/// compares it to `NTFS`, the volume description reports it verbatim.
fn filesystem_name(handle: std::os::windows::io::RawHandle) -> Result<String, CheckedFsError> {
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
    String::from_utf16(&filesystem[..length]).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PersistentFilesystemIdentity,
            "filesystem name is not valid UTF-16",
        )
    })
}

/// `GetFinalPathNameByHandleW` under either naming, shared by the volume-GUID
/// identity fetch and the description's UNC test.
fn final_path(
    handle: std::os::windows::io::RawHandle,
    flags: GETFINALPATHNAMEBYHANDLE_FLAGS,
) -> Result<Vec<u16>, CheckedFsError> {
    let mut path = vec![0_u16; 1024];
    let length =
        unsafe { GetFinalPathNameByHandleW(handle, path.as_mut_ptr(), path.len() as u32, flags) };
    if length == 0 || length as usize >= path.len() {
        return Err(query_error(
            PlatformCapability::PersistentFilesystemIdentity,
            "query local NTFS volume GUID",
        ));
    }
    path.truncate(length as usize);
    Ok(path)
}

fn is_unc_final_path(path: &[u16]) -> bool {
    path.starts_with(&UNC_FINAL_PATH_PREFIX.encode_utf16().collect::<Vec<_>>())
}

/// `GetDriveTypeW` wants a ROOT, so the volume GUID gains the trailing
/// backslash `volume_guid` deliberately truncates away, plus the NUL the
/// `PCWSTR` contract needs.
fn volume_root_drive_type(volume_guid: &[u16]) -> u32 {
    let mut root = volume_guid.to_vec();
    root.push(u16::from(b'\\'));
    root.push(0);
    unsafe { GetDriveTypeW(root.as_ptr()) }
}

fn volume_guid(handle: std::os::windows::io::RawHandle) -> Result<Vec<u16>, CheckedFsError> {
    let mut path = final_path(handle, VOLUME_NAME_GUID)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unc_final_path_classifies_remote_and_a_volume_guid_path_does_not() {
        // DR-1 W2 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.3, 2026-09-03):
        // the half of the Windows remote test that needs no volume, so it
        // runs on every Windows runner regardless of what is mounted. The
        // other half (`GetDriveTypeW` on the volume root) needs a real remote
        // volume and is therefore matrix work, not a unit test.
        let unc = r"\\?\UNC\server\share\dir"
            .encode_utf16()
            .collect::<Vec<_>>();
        assert!(is_unc_final_path(&unc));
        let guid = r"\\?\Volume{00000000-0000-0000-0000-000000000000}\dir"
            .encode_utf16()
            .collect::<Vec<_>>();
        assert!(!is_unc_final_path(&guid));
        // A path that merely CONTAINS the token is not a UNC final path.
        let decoy = r"\\?\Volume{0}\UNC\share"
            .encode_utf16()
            .collect::<Vec<_>>();
        assert!(!is_unc_final_path(&decoy));
        assert!(!is_unc_final_path(&[]));
    }
}
