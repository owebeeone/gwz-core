use crate::filesystem::{FsDirectory, FsFile};
pub(super) use crate::filesystem::{
    FsLegacyDurableIdentity as DurableObjectIdentity, FsLegacyObjectIdentity as ObjectIdentity,
    FsLegacyRenameDomain as RenameDomainProof,
};
use std::path::Path;

impl ObjectIdentity {
    pub(super) fn name_digest(&self) -> [u8; 16] {
        use sha2::{Digest, Sha256};

        Sha256::digest(self.durable.encode())[..16]
            .try_into()
            .expect("SHA-256 has at least sixteen bytes")
    }
}

impl DurableObjectIdentity {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut output = Vec::new();
        match self {
            Self::Linux {
                filesystem_id,
                handle_type,
                file_handle,
            } => {
                output.push(1);
                put_bytes(&mut output, filesystem_id);
                output.extend(handle_type.to_le_bytes());
                put_bytes(&mut output, file_handle);
            }
            Self::Mac {
                volume_uuid,
                persistent_object_id,
            } => {
                output.push(2);
                output.extend(volume_uuid);
                output.extend(persistent_object_id);
            }
            Self::Windows {
                volume_guid,
                file_id,
            } => {
                output.push(3);
                let encoded = volume_guid
                    .iter()
                    .flat_map(|unit| unit.to_le_bytes())
                    .collect::<Vec<_>>();
                put_bytes(&mut output, &encoded);
                output.extend(file_id);
            }
        }
        output
    }

    pub(super) fn decode(input: &[u8]) -> Option<Self> {
        let (&tag, mut tail) = input.split_first()?;
        let value = match tag {
            1 => {
                let (filesystem_id, rest) = take_bytes(tail)?;
                tail = rest;
                let (handle_type, rest) = take_array::<4>(tail)?;
                tail = rest;
                let (file_handle, rest) = take_bytes(tail)?;
                tail = rest;
                Self::Linux {
                    filesystem_id: filesystem_id.to_vec(),
                    handle_type: i32::from_le_bytes(handle_type),
                    file_handle: file_handle.to_vec(),
                }
            }
            2 => {
                let (volume_uuid, rest) = take_array::<16>(tail)?;
                let (persistent_object_id, rest) = take_array::<8>(rest)?;
                tail = rest;
                Self::Mac {
                    volume_uuid,
                    persistent_object_id,
                }
            }
            3 => {
                let (encoded_guid, rest) = take_bytes(tail)?;
                if encoded_guid.len() % 2 != 0 {
                    return None;
                }
                let volume_guid = encoded_guid
                    .chunks_exact(2)
                    .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
                    .collect();
                let (file_id, rest) = take_array::<16>(rest)?;
                tail = rest;
                Self::Windows {
                    volume_guid,
                    file_id,
                }
            }
            _ => return None,
        };
        tail.is_empty().then_some(value)
    }
}

pub(super) fn filesystem_directory_identity(
    directory: &FsDirectory,
) -> std::io::Result<ObjectIdentity> {
    #[cfg(test)]
    if super::capability::handle_probe_is_unavailable() {
        return Err(identity_error(std::io::ErrorKind::Unsupported.into()));
    }
    directory
        .filesystem()
        .legacy_directory_identity(directory)
        .map_err(identity_error)
}
pub(super) fn filesystem_file_identity(file: &FsFile) -> std::io::Result<ObjectIdentity> {
    file.filesystem()
        .legacy_file_identity(file)
        .map_err(identity_error)
}
pub(super) fn filesystem_rename_domain(
    directory: &FsDirectory,
) -> std::io::Result<RenameDomainProof> {
    directory
        .filesystem()
        .legacy_rename_domain(directory)
        .map_err(identity_error)
}
pub(super) fn filesystem_canonical_path_identity(
    directory: &FsDirectory,
    relative: &Path,
) -> std::io::Result<Vec<u8>> {
    directory
        .filesystem()
        .legacy_path_identity(directory, relative)
}
fn identity_error(source: std::io::Error) -> std::io::Error {
    if source.kind() == std::io::ErrorKind::Unsupported {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            super::capability::PERSISTENT_FILESYSTEM_IDENTITY_REMEDY,
        )
    } else {
        source
    }
}
fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend((bytes.len() as u16).to_le_bytes());
    output.extend(bytes);
}

fn take_bytes(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, tail) = take_array::<2>(input)?;
    let length = u16::from_le_bytes(length) as usize;
    (tail.len() >= length).then(|| tail.split_at(length))
}

fn take_array<const N: usize>(input: &[u8]) -> Option<([u8; N], &[u8])> {
    let (value, tail) = input.split_at_checked(N)?;
    Some((value.try_into().ok()?, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_identity_encoding_is_exact_and_full_width() {
        let values = [
            DurableObjectIdentity::Linux {
                filesystem_id: vec![1, 2, 3, 4],
                handle_type: -7,
                file_handle: vec![5, 6, 7, 8, 9],
            },
            DurableObjectIdentity::Mac {
                volume_uuid: [10; 16],
                persistent_object_id: [11; 8],
            },
            DurableObjectIdentity::Windows {
                volume_guid: "volume-guid".encode_utf16().collect(),
                file_id: [12; 16],
            },
        ];
        for value in values {
            assert_eq!(DurableObjectIdentity::decode(&value.encode()), Some(value));
        }
    }

    #[test]
    fn truncated_windows_identity_is_not_equal() {
        let left = DurableObjectIdentity::Windows {
            volume_guid: vec![1, 2, 3],
            file_id: [9; 16],
        };
        let mut right = left.clone();
        let DurableObjectIdentity::Windows { file_id, .. } = &mut right else {
            unreachable!();
        };
        file_id[15] = 8;
        assert_ne!(left, right);
        assert_ne!(left.encode(), right.encode());
    }
}
