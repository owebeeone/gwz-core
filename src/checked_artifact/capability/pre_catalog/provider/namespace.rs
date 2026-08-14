use std::io;

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::super::*;
use super::filesystem::PlatformProviderV1;
use super::retained::{RetainedDirectory, RetainedPlatformRoot, encode_identity};
use crate::checked_artifact::capability::{PathComponentMode, PlatformCapability};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};

pub(super) type NamespaceFacts = Vec<(Vec<u8>, Vec<u8>)>;

pub(super) fn observe(
    retained: &RetainedPlatformRoot,
    platform: &impl PlatformProviderV1,
    root: CatalogPrivateRootV1,
) -> Result<NamespaceFacts, CheckedFsError> {
    let mut facts = Vec::new();
    facts
        .try_reserve_exact(CatalogPrivateNameV1::ALL.len())
        .map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "private namespace observation allocation failed",
            )
        })?;
    for name in CatalogPrivateNameV1::ALL {
        let path = name.relative_bytes(root);
        let fact = match retained.private_parent() {
            Some(parent) => observe_leaf(parent, name.leaf_bytes(), platform)?,
            None => vec![0],
        };
        facts.push((path, fact));
    }
    Ok(facts)
}

fn observe_leaf(
    parent: &RetainedDirectory,
    name: &[u8],
    platform: &impl PlatformProviderV1,
) -> Result<Vec<u8>, CheckedFsError> {
    let name = std::str::from_utf8(name).expect("fixed catalog name is ASCII");
    reject_equivalent_alias(parent, name)?;
    match parent.handle().symlink_metadata(name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(vec![0]),
        Err(source) => Err(CheckedFsError::io("observe private namespace leaf", source)),
        Ok(metadata) => {
            let mut value = Vec::new();
            if metadata.is_dir() && !metadata.is_symlink() {
                let directory = parent.handle().open_dir_nofollow(name).map_err(|source| {
                    CheckedFsError::io("open private directory no-follow", source)
                })?;
                value.push(1);
                value.extend_from_slice(&encode_identity(&platform.dir_identity(&directory)?));
            } else if metadata.is_file() && !metadata.is_symlink() {
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let file = parent
                    .handle()
                    .open_with(name, &options)
                    .map_err(|source| CheckedFsError::io("open private file no-follow", source))?;
                value.push(2);
                value.extend_from_slice(&encode_identity(&platform.file_identity(&file)?));
            } else {
                value.push(if metadata.is_symlink() { 3 } else { 4 });
                value.extend_from_slice(&metadata.dev().to_be_bytes());
                value.extend_from_slice(&metadata.ino().to_be_bytes());
            }
            Ok(value)
        }
    }
}

fn reject_equivalent_alias(
    parent: &RetainedDirectory,
    expected: &str,
) -> Result<(), CheckedFsError> {
    for entry in parent
        .handle()
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate private namespace", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read private namespace", source))?;
        let observed = entry.file_name();
        let Some(observed) = observed.to_str() else {
            continue;
        };
        let equivalent = match parent.mode() {
            PathComponentMode::Sensitive => observed == expected,
            PathComponentMode::AsciiCaseFold => {
                observed.is_ascii() && observed.eq_ignore_ascii_case(expected)
            }
        };
        if equivalent && observed != expected {
            return Err(CheckedFsError::ambiguous(
                "private namespace spelling",
                format!("noncanonical alias '{observed}' conflicts with '{expected}'"),
            ));
        }
    }
    Ok(())
}
