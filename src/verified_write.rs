//! The neutral raw publication primitive: a staged, renamed, flushed and
//! RE-READ file write, owned by no lifecycle.
//!
//! **Why this module exists at all** (`GwzM5-8M5d-Charter.md` §3/§4,
//! 2026-09-03). On a volume whose filesystem cannot answer the checked
//! boundary's persistent-handle probe — overlayfs without `nfs_export`, some
//! FUSE mounts — `CheckedArtifact::acquire` refuses, and before M5d that
//! refusal killed a merge start AFTER its warning had already been emitted
//! (ship (1) charter §4.1's recorded limit). The charter's answer is best
//! effort: below the handle bar the merge record's CREATE publishes through
//! this primitive instead, and says so.
//!
//! [`write_atomic_verified`] is that primitive, moved here VERBATIM from the
//! deleted v0 record store (`workspace_ops/merge/store/mod.rs:463-498` at
//! gwz-core `57502e4`), where it was the v0 open-record writer. Nothing about
//! it is new; only its home is. It is deliberately **neutral** — not under
//! `merge/store`, not under `v1_lifecycle/` — for two reasons the charter
//! states: the v0 store is deleted from production, and F-3's seam floor is
//! redefined onto THIS module, so `v1_lifecycle/` may name neither the module
//! nor the function (`check_checked_artifact_boundaries.py`'s
//! `NEUTRAL_RAW_WRITE_FLOOR`, the successor of the derived v0-persistence
//! scan under the J-1 succession ruling of 2026-09-03).
//!
//! **Exactly one production caller.**
//! `checked_artifact::entry::create_merge_store_record` reaches it on the
//! handle-fail arm and nothing else does; the checker pins that count in both
//! directions, so converting the raw arm to the checked door fails closed
//! rather than passing silently.
//!
//! **What it is NOT.** It is not crash recovery. The charter is explicit that
//! a power loss mid-merge on a handle-less volume is operator cleanup, not a
//! `recover()` grammar: this function leaves no catalog entry, no nonce and no
//! reboot-durable identity behind, and no `recover()` of a half-written raw
//! record exists or is owed.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::durable_fs::{rename_durable, sync_dir};
use crate::model::{ErrorCode, ModelError, ModelResult};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Publish `bytes` at `path` durably, then prove it by reading them back.
///
/// The sequence is the v0 store's own, unchanged: a fresh `O_EXCL` temporary
/// beside the target, `write_all` + `sync_all`, `rename_durable` onto the
/// target, `sync_dir` on the parent, then a re-read byte compare. Every early
/// exit removes the temporary. `rename_durable`'s `replace = true` is
/// harmless on the one path that reaches this function: `create_open` refuses
/// an existing record before it is called, so the target is absent.
pub(crate) fn write_atomic_verified(path: &Path, bytes: &[u8]) -> ModelResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| recovery_error("record path has no parent"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let (temporary, mut file) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate =
            path.with_extension(format!("yaml.{}.{}.tmp", std::process::id(), sequence));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    };
    let staged = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    if let Err(error) = rename_durable(&temporary, path, true) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    sync_dir(parent).map_err(io_error)?;
    if fs::read(path).map_err(io_error)? != bytes {
        return Err(recovery_error(
            "merge record bytes failed write verification",
        ));
    }
    Ok(())
}

fn recovery_error(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

fn io_error(error: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}
