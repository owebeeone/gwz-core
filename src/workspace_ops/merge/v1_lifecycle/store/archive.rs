use std::fs;

use crate::durable_fs::{rename_noreplace, sync_dir};
use crate::model::ModelResult;
use crate::workspace_ops::merge::OperationState;

use super::super::checked::{StoredV1Record, V1MutationLease};
use super::ArchiveOutcome;
use super::rewrite::{io_error, path_exists, read_regular, recovery};

pub(super) fn archive(
    lease: &V1MutationLease,
    current: &StoredV1Record,
) -> ModelResult<ArchiveOutcome> {
    if !lease.covers(current.location())
        || !matches!(
            current.record().state,
            OperationState::Completed | OperationState::Aborted
        )
    {
        return Err(recovery(
            "checked v1 archive requires the matching lease and terminal record",
        ));
    }
    let source = current.location().path();
    let merge_root = source
        .parent()
        .ok_or_else(|| recovery("open record path has no parent"))?;
    let done = merge_root.join("done");
    let destination = done.join(
        source
            .file_name()
            .ok_or_else(|| recovery("open record path has no file name"))?,
    );
    let source_exists = path_exists(source)?;
    let destination_exists = path_exists(&destination)?;

    match (source_exists, destination_exists) {
        (false, false) => Err(recovery(
            "checked v1 archive source and destination are absent",
        )),
        (false, true) => {
            require_exact_destination(current, &destination)?;
            Ok(ArchiveOutcome::ReconciledDestination)
        }
        (true, true) => {
            let source_bytes = require_exact_source(current)?;
            let destination_bytes = read_regular(&destination)?;
            if destination_bytes != source_bytes {
                return Err(recovery(
                    "checked v1 archive source and destination bytes differ",
                ));
            }
            fs::remove_file(source).map_err(io_error)?;
            sync_dir(merge_root).map_err(io_error)?;
            sync_dir(&done).map_err(io_error)?;
            Ok(ArchiveOutcome::ReconciledBothCopies)
        }
        (true, false) => {
            let source_bytes = require_exact_source(current)?;
            fs::create_dir_all(&done).map_err(io_error)?;
            require_plain_directory(&done)?;
            match rename_noreplace(source, &destination) {
                Ok(()) => {
                    sync_dir(merge_root).map_err(io_error)?;
                    sync_dir(&done).map_err(io_error)?;
                    require_exact_destination(current, &destination)?;
                    Ok(ArchiveOutcome::Published)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let destination_bytes = read_regular(&destination)?;
                    if destination_bytes != source_bytes {
                        return Err(recovery(
                            "checked v1 archive destination appeared with different bytes",
                        ));
                    }
                    fs::remove_file(source).map_err(io_error)?;
                    sync_dir(merge_root).map_err(io_error)?;
                    sync_dir(&done).map_err(io_error)?;
                    Ok(ArchiveOutcome::ReconciledBothCopies)
                }
                Err(error) => Err(io_error(error)),
            }
        }
    }
}

fn require_plain_directory(path: &std::path::Path) -> ModelResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_dir() && path.canonicalize().map_err(io_error)? == path {
        Ok(())
    } else {
        Err(recovery(format!(
            "checked v1 archive directory '{}' is not a canonical directory",
            path.display()
        )))
    }
}

fn require_exact_source(current: &StoredV1Record) -> ModelResult<Vec<u8>> {
    let bytes = read_regular(current.location().path())?;
    let reopened = StoredV1Record::from_open_bytes(
        current.location().root(),
        current.location().path(),
        &bytes,
    )?;
    if current.same_source_as(&reopened) {
        Ok(bytes)
    } else {
        Err(recovery("checked v1 archive source lineage changed"))
    }
}

fn require_exact_destination(
    current: &StoredV1Record,
    destination: &std::path::Path,
) -> ModelResult<()> {
    let bytes = read_regular(destination)?;
    if super::super::checked::RecordDigest::from_bytes(&bytes) == current.source_digest() {
        Ok(())
    } else {
        Err(recovery(
            "checked v1 archive destination bytes differ from the checked source",
        ))
    }
}
