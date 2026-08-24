use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use sha2::{Digest, Sha256};

use super::compatibility_errors::decode_error;
use super::{
    FileMergeStore, MERGE_DIR, MergeStore, RecordLocation, TEMP_SEQUENCE, io_error, recovery_error,
    validate_merge_id,
};
use crate::durable_fs::{rename_durable, sync_dir};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::record_wire::{
    PreparedOpenV0Upgrade, PreparedV1Upgrade, decode_production_v0, prepare_upgrade,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AtomicUpgradeFault {
    None,
    BeforeStageWrite,
    AfterStageFsync,
    BeforeAtomicRename,
    AfterRenameBeforeVerification,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AtomicUpgradeOutcome {
    ValidUnlisted,
    Upgraded {
        rule_id: String,
        next_action: String,
    },
}

pub(crate) fn upgrade_open_v0<B: GitBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    writer_version: &str,
    fault: AtomicUpgradeFault,
) -> ModelResult<AtomicUpgradeOutcome> {
    validate_merge_id(merge_id)?;
    let open = FileMergeStore.discover_open(root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::OperationNotFound,
            format!("merge record '{merge_id}' was not found"),
        )
    })?;
    if open.merge_id != merge_id {
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("merge record '{merge_id}' was not found"),
        ));
    }
    let target = root.join(MERGE_DIR).join(format!("{merge_id}.yaml"));
    require_regular_file(&target)?;
    let source = fs::read(&target).map_err(io_error)?;
    let decoded = decode_production_v0(&source)
        .map_err(|error| decode_error(&target, merge_id, RecordLocation::Open, error))?;
    if decoded.record().merge_id != merge_id {
        return Err(recovery_error(format!(
            "merge record id '{}' does not match upgrade target '{merge_id}'",
            decoded.record().merge_id
        )));
    }
    let prepared = match prepare_upgrade(backend, root, &decoded, writer_version)? {
        PreparedOpenV0Upgrade::ValidUnlisted => {
            return Ok(AtomicUpgradeOutcome::ValidUnlisted);
        }
        PreparedOpenV0Upgrade::Eligible(prepared) => prepared,
    };
    publish_prepared(&target, &source, &prepared, fault)?;
    Ok(AtomicUpgradeOutcome::Upgraded {
        rule_id: prepared.rule_id.clone(),
        next_action: prepared.next_action.clone(),
    })
}

fn publish_prepared(
    target: &Path,
    source: &[u8],
    prepared: &PreparedV1Upgrade,
    fault: AtomicUpgradeFault,
) -> ModelResult<()> {
    if fault == AtomicUpgradeFault::BeforeStageWrite {
        return Err(injected_fault(fault));
    }

    let parent = target
        .parent()
        .ok_or_else(|| recovery_error("merge upgrade path has no parent"))?;
    let (temporary, mut file) = create_unique_temporary(target)?;
    let staged = file
        .write_all(prepared.bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    if fault == AtomicUpgradeFault::AfterStageFsync {
        return Err(injected_fault(fault));
    }

    let staged = fs::read(&temporary).map_err(io_error)?;
    if staged != prepared.bytes() {
        return Err(recovery_error(
            "staged merge-record upgrade bytes differ from the prepared bytes",
        ));
    }
    prepared.verify_bytes(&staged)?;
    let staged_sha256 = digest(&staged);

    if fs::read(target).map_err(io_error)? != source {
        return Err(recovery_error(format!(
            "merge record at '{}' changed while its upgrade was being prepared",
            target.display()
        )));
    }
    if fault == AtomicUpgradeFault::BeforeAtomicRename {
        return Err(injected_fault(fault));
    }

    if let Err(error) = rename_durable(&temporary, target, true) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    sync_dir(parent).map_err(io_error)?;
    if fault == AtomicUpgradeFault::AfterRenameBeforeVerification {
        return Err(injected_fault(fault));
    }

    let published = fs::read(target).map_err(io_error)?;
    if digest(&published) != staged_sha256 {
        return Err(recovery_error(
            "published merge-record upgrade hash differs from its verified staged hash",
        ));
    }
    prepared.verify_bytes(&published)
}

fn create_unique_temporary(target: &Path) -> ModelResult<(PathBuf, File)> {
    loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = target.with_extension(format!(
            "yaml.{}.{}.upgrade.tmp",
            std::process::id(),
            sequence
        ));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn require_regular_file(path: &Path) -> ModelResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file() {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!(
                "merge record at '{}' is unreadable: record path is not a regular file",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn injected_fault(fault: AtomicUpgradeFault) -> ModelError {
    recovery_error(format!(
        "injected atomic merge-record upgrade fault at {fault:?}"
    ))
}
