//! Explicit durable identity values supplied by the closed platform profiles.

use super::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::protocol::generated;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum DurableObjectIdentityV1 {
    LinuxExt4 {
        external_filesystem_uuid: [u8; 16],
        handle_type: i32,
        persistent_handle: Vec<u8>,
    },
    Mac {
        volume_uuid: [u8; 16],
        persistent_object_id: [u8; 8],
    },
    WindowsNtfs {
        volume_guid_utf16: Vec<u16>,
        file_id_128: [u8; 16],
    },
}

impl DurableObjectIdentityV1 {
    pub(in crate::checked_artifact) const fn support_profile(
        &self,
    ) -> super::SupportedFilesystemProfile {
        match self {
            Self::LinuxExt4 { .. } => super::SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
            Self::Mac { .. } => super::SupportedFilesystemProfile::MacPersistentObjectIdV1,
            Self::WindowsNtfs { .. } => super::SupportedFilesystemProfile::WindowsNtfsFileId128V1,
        }
    }

    /// DR-1 W2, 2026-09-03 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.2): the
    /// constructor's NAME and its `LinuxExt4` variant say `ext4`, and since
    /// this step the Linux provider builds them for every filesystem that
    /// answers `FS_IOC_GETFSUUID` with a nonzero 16-byte UUID and
    /// `name_to_handle_at` with a persistent handle — xfs and f2fs included.
    /// Both names stay because the variant is a PERSISTED catalog value (it
    /// maps to `generated::CheckedDurableIdentityKind::LinuxExt4` below):
    /// renaming it is a catalog-format change, which the charter parks. The
    /// value contract below is unchanged and is about well-formedness, not
    /// about which filesystem produced the bytes.
    pub(in crate::checked_artifact) fn linux_ext4(
        external_filesystem_uuid: [u8; 16],
        handle_type: i32,
        persistent_handle: Vec<u8>,
    ) -> Result<Self, CheckedFsError> {
        if external_filesystem_uuid == [0; 16]
            || handle_type <= 0
            || !(1..=128).contains(&persistent_handle.len())
        {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::DurableObjectIdentity,
                "ext4 identity needs a nonzero external UUID, positive handle type, and 1..=128 handle bytes",
            ));
        }
        Ok(Self::LinuxExt4 {
            external_filesystem_uuid,
            handle_type,
            persistent_handle,
        })
    }

    pub(in crate::checked_artifact) fn mac(
        volume_uuid: [u8; 16],
        persistent_object_id: [u8; 8],
    ) -> Result<Self, CheckedFsError> {
        if volume_uuid == [0; 16] || persistent_object_id == [0; 8] {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::DurableObjectIdentity,
                "macOS identity needs nonzero volume and persistent object identifiers",
            ));
        }
        Ok(Self::Mac {
            volume_uuid,
            persistent_object_id,
        })
    }

    pub(in crate::checked_artifact) fn windows_ntfs(
        volume_guid_utf16: Vec<u16>,
        file_id_128: [u8; 16],
    ) -> Result<Self, CheckedFsError> {
        if volume_guid_utf16.is_empty() || file_id_128 == [0; 16] {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::DurableObjectIdentity,
                "NTFS identity needs a volume GUID and nonzero 128-bit file ID",
            ));
        }
        Ok(Self::WindowsNtfs {
            volume_guid_utf16,
            file_id_128,
        })
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    pub(in crate::checked_artifact) fn to_generated(
        &self,
    ) -> generated::CheckedDurableObjectIdentityV1 {
        let mut value = generated::CheckedDurableObjectIdentityV1::default();
        match self {
            Self::LinuxExt4 {
                external_filesystem_uuid,
                handle_type,
                persistent_handle,
            } => {
                value.kind = generated::CheckedDurableIdentityKind::LinuxExt4;
                value.linux_external_filesystem_uuid = Some(external_filesystem_uuid.to_vec());
                value.linux_handle_type = Some(i64::from(*handle_type));
                value.linux_persistent_handle = Some(persistent_handle.clone());
            }
            Self::Mac {
                volume_uuid,
                persistent_object_id,
            } => {
                value.kind = generated::CheckedDurableIdentityKind::Mac;
                value.mac_volume_uuid = Some(volume_uuid.to_vec());
                value.mac_persistent_object_id = Some(persistent_object_id.to_vec());
            }
            Self::WindowsNtfs {
                volume_guid_utf16,
                file_id_128,
            } => {
                value.kind = generated::CheckedDurableIdentityKind::WindowsNtfs;
                value.windows_volume_guid_utf16le = Some(
                    volume_guid_utf16
                        .iter()
                        .flat_map(|unit| unit.to_le_bytes())
                        .collect(),
                );
                value.windows_file_id_128 = Some(file_id_128.to_vec());
            }
        }
        value
    }

    pub(in crate::checked_artifact) fn decode_canonical(
        input: &[u8],
    ) -> Result<Self, CheckedFsError> {
        let fail = || CheckedFsError::ambiguous("durable identity", "invalid taut record");
        let cbor = crate::cbor::try_decode(input).map_err(|_| fail())?;
        let wire =
            generated::CheckedDurableObjectIdentityV1::from_cbor(&cbor).map_err(|_| fail())?;
        let value = match wire.kind {
            generated::CheckedDurableIdentityKind::LinuxExt4 => Self::linux_ext4(
                required_array(wire.linux_external_filesystem_uuid, fail)?,
                i32::try_from(wire.linux_handle_type.ok_or_else(fail)?).map_err(|_| fail())?,
                wire.linux_persistent_handle.ok_or_else(fail)?,
            )?,
            generated::CheckedDurableIdentityKind::Mac => Self::mac(
                required_array(wire.mac_volume_uuid, fail)?,
                required_array(wire.mac_persistent_object_id, fail)?,
            )?,
            generated::CheckedDurableIdentityKind::WindowsNtfs => {
                let encoded = wire.windows_volume_guid_utf16le.ok_or_else(fail)?;
                if encoded.len() % 2 != 0 {
                    return Err(fail());
                }
                let guid = encoded
                    .chunks_exact(2)
                    .map(|part| u16::from_le_bytes([part[0], part[1]]))
                    .collect();
                Self::windows_ntfs(guid, required_array(wire.windows_file_id_128, fail)?)?
            }
        };
        (value.encode_canonical() == input)
            .then_some(value)
            .ok_or_else(fail)
    }
}

fn required_array<const N: usize>(
    value: Option<Vec<u8>>,
    fail: impl Fn() -> CheckedFsError,
) -> Result<[u8; N], CheckedFsError> {
    value.ok_or_else(&fail)?.try_into().map_err(|_| fail())
}
