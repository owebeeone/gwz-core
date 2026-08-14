use std::ffi::OsStr;
use std::io::{self, Read};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::filesystem::PlatformProviderV1;
use super::retained::{RetainedDirectory, RetainedPlatformRoot, encode_identity};
use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogRetiredFactV1, RawCatalogRoleObservationV1,
    RawCatalogRoleRowV1,
};
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::{
    CatalogParentGrammarV1, CatalogParentObservationV1, CatalogRecognizedNameV1,
};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{InfrastructureSlotV1, ProtocolRecordKindV1};

pub(super) fn observe(
    retained: &RetainedPlatformRoot,
    platform: &impl PlatformProviderV1,
    root: CatalogPrivateRootV1,
) -> Result<RawCatalogRoleObservationV1, CheckedFsError> {
    let Some(parent) = retained.private_parent() else {
        return Ok(RawCatalogRoleObservationV1 {
            enumeration: CatalogParentObservationV1::empty(),
            rows: Vec::new(),
        });
    };
    let grammar = CatalogParentGrammarV1::new(parent.mode());
    let mut scanner = grammar.scanner();
    let mut rows = Vec::new();
    for entry in parent
        .handle()
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog parent", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read catalog parent", source))?;
        let observed_name = entry.file_name();
        let Some(role) = scanner.observe_os_str(&observed_name)? else {
            continue;
        };
        let leaf = canonical_leaf(&role);
        let path = relative_bytes(root, leaf)?;
        let fact = observe_leaf(parent, &observed_name, &role, platform)?;
        rows.try_reserve_exact(1).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog role observation allocation failed",
            )
        })?;
        rows.push(RawCatalogRoleRowV1 { role, path, fact });
    }
    rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(RawCatalogRoleObservationV1 {
        enumeration: scanner.finish(),
        rows,
    })
}

fn canonical_leaf(role: &CatalogRecognizedNameV1) -> &[u8] {
    match role {
        CatalogRecognizedNameV1::Scratch(name) => name.as_bytes(),
        CatalogRecognizedNameV1::Active => CatalogPrivateNameV1::BootstrapActive.leaf_bytes(),
        CatalogRecognizedNameV1::Staging => CatalogPrivateNameV1::BootstrapStaging.leaf_bytes(),
        CatalogRecognizedNameV1::Final => CatalogPrivateNameV1::Final.leaf_bytes(),
    }
}

fn relative_bytes(root: CatalogPrivateRootV1, leaf: &[u8]) -> Result<Vec<u8>, CheckedFsError> {
    let prefix = match root {
        CatalogPrivateRootV1::Workspace => b".gwz".as_slice(),
        CatalogPrivateRootV1::GitDirectory => b"gwz".as_slice(),
    };
    let mut value = Vec::new();
    value
        .try_reserve_exact(prefix.len() + 1 + leaf.len())
        .map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog role path allocation failed",
            )
        })?;
    value.extend_from_slice(prefix);
    value.push(b'/');
    value.extend_from_slice(leaf);
    Ok(value)
}

fn observe_leaf(
    parent: &RetainedDirectory,
    name: &OsStr,
    role: &CatalogRecognizedNameV1,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogEntryFactV1, CheckedFsError> {
    match parent.handle().symlink_metadata(name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Err(CheckedFsError::ambiguous(
            "catalog parent",
            "reserved entry disappeared during aggregate observation",
        )),
        Err(source) => Err(CheckedFsError::io("observe catalog role", source)),
        Ok(metadata) => {
            let mut value = Vec::new();
            if metadata.is_dir() && !metadata.is_symlink() {
                let directory = parent.handle().open_dir_nofollow(name).map_err(|source| {
                    CheckedFsError::io("open catalog directory no-follow", source)
                })?;
                let identity = encode_identity(&platform.dir_identity(&directory)?);
                let retired = if matches!(role, CatalogRecognizedNameV1::Final) {
                    observe_retired(&directory, platform)?
                } else {
                    RawCatalogRetiredFactV1::Missing
                };
                return Ok(RawCatalogEntryFactV1::Directory { identity, retired });
            } else if metadata.is_file() && !metadata.is_symlink() {
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let mut file = parent
                    .handle()
                    .open_with(name, &options)
                    .map_err(|source| {
                        CheckedFsError::io("open catalog record no-follow", source)
                    })?;
                let identity = encode_identity(&platform.file_identity(&file)?);
                let bytes = read_record_bytes(&mut file)?;
                return Ok(RawCatalogEntryFactV1::RegularFile { identity, bytes });
            } else {
                value.push(if metadata.is_symlink() { 3 } else { 4 });
                value.extend_from_slice(&metadata.dev().to_be_bytes());
                value.extend_from_slice(&metadata.ino().to_be_bytes());
            }
            Ok(RawCatalogEntryFactV1::Other(value))
        }
    }
}

fn observe_retired(
    final_directory: &cap_std::fs::Dir,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogRetiredFactV1, CheckedFsError> {
    let name = OsStr::new(InfrastructureSlotV1::CatalogBootstrapRetired.name());
    match final_directory.symlink_metadata(name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Ok(RawCatalogRetiredFactV1::Missing)
        }
        Err(source) => Err(CheckedFsError::io("observe retired catalog record", source)),
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let mut file = final_directory
                .open_with(name, &options)
                .map_err(|source| {
                    CheckedFsError::io("open retired catalog record no-follow", source)
                })?;
            Ok(RawCatalogRetiredFactV1::RegularFile {
                identity: encode_identity(&platform.file_identity(&file)?),
                bytes: read_record_bytes(&mut file)?,
            })
        }
        Ok(metadata) => {
            let mut value = Vec::new();
            value.push(if metadata.is_symlink() { 3 } else { 4 });
            value.extend_from_slice(&metadata.dev().to_be_bytes());
            value.extend_from_slice(&metadata.ino().to_be_bytes());
            Ok(RawCatalogRetiredFactV1::Other(value))
        }
    }
}

fn read_record_bytes(file: &mut cap_std::fs::File) -> Result<RawCatalogBytesV1, CheckedFsError> {
    let limit = ProtocolRecordKindV1::CatalogBootstrap.max_bytes();
    let read_limit = u64::try_from(limit + 1).expect("catalog record limit fits u64");
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog record observation allocation failed",
        )
    })?;
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog record", source))?;
    if bytes.len() > limit {
        Ok(RawCatalogBytesV1::Oversize)
    } else {
        Ok(RawCatalogBytesV1::Bounded(bytes))
    }
}
