//! Owner-private physical edges for the first catalog.

use std::ffi::OsStr;
use std::io::{self, Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::RetainedPlatformRoot;
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::CatalogScratchNameV1;
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{CatalogBootstrapRecordV1, ProtocolRecordKindV1};
use crate::model::ErrorCode;

pub(in crate::checked_artifact::capability::pre_catalog) fn create_git_private_parent(
    retained: &RetainedPlatformRoot,
) -> Result<(), CheckedFsError> {
    let parent = retained.root().handle();
    let name = OsStr::new("gwz");
    match parent.create_dir(name) {
        Ok(()) => {
            let opened = parent.open_dir_nofollow(name).map_err(|source| {
                CheckedFsError::io("reopen created Git GWZ parent no-follow", source)
            })?;
            opened
                .dir_metadata()
                .map_err(|source| CheckedFsError::io("identify created Git GWZ parent", source))?;
            sync_directory(parent, "flush created Git GWZ parent")
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(CheckedFsError::io(
            "create Git GWZ parent no-replace",
            source,
        )),
    }
}

pub(in crate::checked_artifact::capability::pre_catalog) fn write_or_rewrite_scratch(
    retained: &RetainedPlatformRoot,
    name: &CatalogScratchNameV1,
    record: &CatalogBootstrapRecordV1,
    create_new: bool,
) -> Result<(), CheckedFsError> {
    let parent = retained.private_parent().ok_or_else(|| {
        CheckedFsError::ambiguous("catalog scratch", "retained private parent is missing")
    })?;
    let leaf = std::str::from_utf8(name.as_bytes()).map_err(|_| {
        CheckedFsError::ambiguous("catalog scratch", "canonical scratch name is not ASCII")
    })?;
    let bytes = record.encode_canonical();
    if bytes.len() > ProtocolRecordKindV1::CatalogBootstrap.max_bytes() {
        return Err(CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog bootstrap record exceeds its bound",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(create_new)
        .truncate(!create_new)
        .follow(FollowSymlinks::No);
    let mut file = parent
        .handle()
        .open_with(OsStr::new(leaf), &options)
        .map_err(|source| CheckedFsError::io("open catalog scratch no-follow", source))?;
    file.write_all(&bytes)
        .map_err(|source| CheckedFsError::io("write catalog scratch", source))?;
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush catalog scratch", source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog scratch", source))?;
    let mut observed = Vec::new();
    observed.try_reserve_exact(bytes.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog scratch verification allocation failed",
        )
    })?;
    file.take((bytes.len() + 1) as u64)
        .read_to_end(&mut observed)
        .map_err(|source| CheckedFsError::io("reread catalog scratch", source))?;
    if observed != bytes {
        return Err(CheckedFsError::ambiguous(
            "catalog scratch",
            "written bytes changed before publication",
        ));
    }
    sync_directory(parent.handle(), "flush catalog scratch namespace")
}

pub(in crate::checked_artifact::capability::pre_catalog) fn publish_active_record(
    retained: &RetainedPlatformRoot,
    scratch: &CatalogScratchNameV1,
) -> Result<(), CheckedFsError> {
    let parent = retained.private_parent().ok_or_else(|| {
        CheckedFsError::ambiguous(
            "catalog active publication",
            "retained private parent is missing",
        )
    })?;
    let source = std::str::from_utf8(scratch.as_bytes()).map_err(|_| {
        CheckedFsError::ambiguous("catalog active publication", "scratch name is not ASCII")
    })?;
    let destination = std::str::from_utf8(CatalogPrivateNameV1::BootstrapActive.leaf_bytes())
        .expect("fixed active name is ASCII");
    crate::checked_artifact::platform::rename_relative(
        parent.handle(),
        OsStr::new(source),
        parent.handle(),
        OsStr::new(destination),
        false,
        ErrorCode::IoError,
        "catalog active publication",
    )
    .map_err(|source| CheckedFsError::ambiguous("catalog active publication", source.message))?;
    sync_directory(parent.handle(), "flush catalog active publication")
}

fn sync_directory(
    directory: &cap_std::fs::Dir,
    operation: &'static str,
) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::sync_parent(directory)
        .map_err(|source| CheckedFsError::io(operation, source))
}
