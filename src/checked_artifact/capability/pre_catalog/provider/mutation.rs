//! Owner-private physical edges for the first catalog.

use std::ffi::OsStr;
use std::io::{self, Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::platform::HostPlatform;
use super::publication::{PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogRoleObservationV1, RetainedPlatformRoot,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, PlatformCapability,
};
use crate::checked_artifact::catalog::{CatalogRecognizedNameV1, CatalogScratchNameV1};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{CatalogBootstrapRecordV1, ProtocolRecordKindV1};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CatalogMutationFaultV1 {
    ScratchBeforeOpen,
    ScratchAfterOpen,
    PublishBeforeRename,
}

#[cfg(test)]
type CatalogMutationFaultCallbackV1 = (CatalogMutationFaultV1, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::RefCell<Option<CatalogMutationFaultCallbackV1>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn run_next_at(point: CatalogMutationFaultV1, callback: impl FnOnce() + 'static) {
    NEXT_FAULT.with(|slot| {
        let previous = slot.replace(Some((point, Box::new(callback))));
        assert!(
            previous.is_none(),
            "catalog mutation fault already installed"
        );
    });
}

#[cfg(test)]
fn run_fault(point: CatalogMutationFaultV1) {
    NEXT_FAULT.with(|slot| {
        let execute = {
            let mut slot = slot.borrow_mut();
            match slot.as_ref() {
                Some((expected, _)) if *expected == point => slot.take(),
                _ => None,
            }
        };
        if let Some((_, callback)) = execute {
            callback();
        }
    });
}

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
            finish_private_parent_edge(parent)
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
    raw_roles: &RawCatalogRoleObservationV1,
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
    let observed_source = observed_scratch(raw_roles, name)?;
    if create_new != observed_source.is_none() {
        return Err(CheckedFsError::ambiguous(
            "catalog scratch",
            "fresh/recovery classification does not match the observed scratch source",
        ));
    }
    #[cfg(test)]
    run_fault(CatalogMutationFaultV1::ScratchBeforeOpen);
    let options = durable_write_options(create_new);
    let mut file = parent
        .handle()
        .open_with(OsStr::new(leaf), &options)
        .map_err(|source| CheckedFsError::io("open catalog scratch no-follow", source))?;
    #[cfg(test)]
    if create_new {
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapScratchCreate,
        );
    }
    if let Some(observed_source) = observed_source {
        verify_open_file(&mut file, observed_source, "catalog scratch")?;
        #[cfg(test)]
        run_fault(CatalogMutationFaultV1::ScratchAfterOpen);
        verify_named_file(
            parent.handle(),
            OsStr::new(leaf),
            observed_source,
            "catalog scratch",
        )?;
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate catalog scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind truncated catalog scratch", source))?;
    }
    file.write_all(&bytes)
        .map_err(|source| CheckedFsError::io("write catalog scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapScratchWrite,
    );
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush catalog scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapScratchFlush,
    );
    let written_identity = encode_identity(&HostPlatform.file_identity(&file)?);
    let written = ObservedRegularFileV1 {
        identity: &written_identity,
        bytes: &bytes,
    };
    verify_open_file(&mut file, written, "catalog scratch")?;
    drop(file);
    verify_named_file(
        parent.handle(),
        OsStr::new(leaf),
        written,
        "catalog scratch",
    )?;
    sync_created_file_namespace(parent.handle())
}

pub(in crate::checked_artifact::capability::pre_catalog) fn publish_active_record(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    scratch: &CatalogScratchNameV1,
    record: &CatalogBootstrapRecordV1,
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
    let expected = observed_scratch(raw_roles, scratch)?.ok_or_else(|| {
        CheckedFsError::ambiguous(
            "catalog active publication",
            "fresh observation did not retain the scratch source",
        )
    })?;
    let expected_bytes = record.encode_canonical();
    if expected.bytes != expected_bytes {
        return Err(CheckedFsError::ambiguous(
            "catalog active publication",
            "observed scratch bytes do not match the expected record",
        ));
    }
    let options = durable_write_options(false);
    let mut file = parent
        .handle()
        .open_with(OsStr::new(source), &options)
        .map_err(|source| CheckedFsError::io("open publishable catalog scratch", source))?;
    verify_open_file(&mut file, expected, "catalog active publication")?;
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush publishable catalog scratch", source))?;
    drop(file);
    #[cfg(test)]
    run_fault(CatalogMutationFaultV1::PublishBeforeRename);
    publish_verified_no_replace(
        parent.handle(),
        OsStr::new(source),
        parent.handle(),
        OsStr::new(destination),
        PublicationSourceV1::regular_file(expected.identity, expected.bytes),
        "catalog active publication",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapActivePublish,
    );
    verify_named_file(
        parent.handle(),
        OsStr::new(destination),
        expected,
        "catalog active publication",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapActiveReobserve,
    );
    sync_published_namespace(parent.handle())
}

#[derive(Clone, Copy)]
struct ObservedRegularFileV1<'a> {
    identity: &'a [u8],
    bytes: &'a [u8],
}

fn observed_scratch<'a>(
    raw_roles: &'a RawCatalogRoleObservationV1,
    expected: &CatalogScratchNameV1,
) -> Result<Option<ObservedRegularFileV1<'a>>, CheckedFsError> {
    let mut result = None;
    for row in &raw_roles.rows {
        let CatalogRecognizedNameV1::Scratch(name) = &row.role else {
            continue;
        };
        if name.as_ref() != expected {
            continue;
        }
        let RawCatalogEntryFactV1::RegularFile {
            identity,
            bytes: RawCatalogBytesV1::Bounded(bytes),
        } = &row.fact
        else {
            return Err(CheckedFsError::ambiguous(
                "catalog scratch",
                "observed scratch is not a bounded regular file",
            ));
        };
        if result.is_some() {
            return Err(CheckedFsError::ambiguous(
                "catalog scratch",
                "multiple retained rows name the expected scratch",
            ));
        }
        result = Some(ObservedRegularFileV1 { identity, bytes });
    }
    Ok(result)
}

fn verify_open_file(
    file: &mut cap_std::fs::File,
    expected: ObservedRegularFileV1<'_>,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    if encode_identity(&HostPlatform.file_identity(file)?) != expected.identity {
        return Err(CheckedFsError::ambiguous(
            fact,
            "opened source identity does not match the fresh aggregate observation",
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog source", source))?;
    if read_bounded(file, expected.bytes.len())? != expected.bytes {
        return Err(CheckedFsError::ambiguous(
            fact,
            "opened source bytes do not match the fresh aggregate observation",
        ));
    }
    Ok(())
}

fn verify_named_file(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    expected: ObservedRegularFileV1<'_>,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("reopen named catalog source", source))?;
    verify_open_file(&mut file, expected, fact)
}

fn read_bounded(
    file: &mut cap_std::fs::File,
    expected_len: usize,
) -> Result<Vec<u8>, CheckedFsError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected_len).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog source verification allocation failed",
        )
    })?;
    file.take((expected_len + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog source", source))?;
    Ok(bytes)
}

fn durable_write_options(create_new: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(create_new)
        .truncate(false)
        .follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH;
        options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    }
    options
}

#[cfg(not(windows))]
fn finish_private_parent_edge(directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::sync_parent(directory)
        .map_err(|source| CheckedFsError::io("flush created Git GWZ parent", source))
}

#[cfg(windows)]
fn finish_private_parent_edge(_directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    // This edge publishes no durable authority. The owner must fully re-enter
    // preflight; loss of the empty parent is the original Missing state.
    Ok(())
}

#[cfg(not(windows))]
fn sync_created_file_namespace(directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::sync_parent(directory)
        .map_err(|source| CheckedFsError::io("flush catalog scratch namespace", source))
}

#[cfg(windows)]
fn sync_created_file_namespace(_directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    // The nonempty scratch is created through FILE_FLAG_WRITE_THROUGH and
    // then flushed with FlushFileBuffers via sync_all. NTFS includes metadata
    // changes produced by the write-through request.
    Ok(())
}

#[cfg(not(windows))]
fn sync_published_namespace(directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::sync_parent(directory)
        .map_err(|source| CheckedFsError::io("flush catalog active publication", source))
}

#[cfg(windows)]
fn sync_published_namespace(_directory: &cap_std::fs::Dir) -> Result<(), CheckedFsError> {
    // rename_relative uses FILE_FLAG_WRITE_THROUGH for the source handle.
    Ok(())
}
