use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::PreparedV1Rewrite;
use super::{CommitFault, unknown};

pub(super) fn load_open(root: &Path, merge_id: &str) -> ModelResult<StoredV1Record> {
    validate_merge_id(merge_id)?;
    let root = root.canonicalize().map_err(io_error)?;
    let path = root.join(".gwz/merge").join(format!("{merge_id}.yaml"));
    let bytes = read_regular(&path)?;
    StoredV1Record::from_open_bytes(&root, &path, &bytes)
}

/// Create the durable open record for one accepted start, at the version the
/// contract-§2 writer floor selected.
///
/// A1's writer floor needs a creation owner: `commit` rewrites a record that
/// already exists (it requires a lease-covered base digest), and before the
/// activation nothing in this tree could bring a v1 record into being. This
/// is that owner.
///
/// **R2-E Step E4.2 — the converted creation path (O13's substantive half,
/// ConsumerCheckpoint §10 row `:280`).** The publication is a CHECKED ARTIFACT
/// ACTION now, not this module's own raw `durable_fs` staged/rename/fsync:
/// `entry::create_merge_store_record` replaces an expected-`Missing` leaf,
/// staging, publishing and flushing the managed parent inside the boundary. So
/// this path no longer creates its parent either — `create_temporary`'s
/// `create_dir_all` was the raw when-missing bootstrap of `.gwz/merge`, and row
/// `:273` gives that to the provider, reached through `acquire_for_merge_start`
/// BEFORE this call; an unbootstrapped parent is refused, not papered over. The
/// single-open-record invariant survives: the pre-flight existence check stays
/// and the checked replacement publishes no-replace onto an absent leaf.
/// `commit`'s raw writers are E4.3's half of O13, converted at that step; with
/// it this module names `durable_fs` nowhere at all.
pub(super) fn create_open(
    lease: &V1MutationLease,
    root: &Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) -> ModelResult<StoredV1Record> {
    validate_merge_id(&record.merge_id)?;
    let root = root.canonicalize().map_err(io_error)?;
    let relative = PathBuf::from(".gwz/merge").join(format!("{}.yaml", record.merge_id));
    let path = root.join(&relative);
    if path_exists(&path)? {
        return Err(recovery(format!(
            "merge record '{}' already exists",
            record.merge_id
        )));
    }

    let raw = serde_yaml::to_value(record).map_err(encode_error)?;
    let encoded = serde_yaml::to_string(&raw)
        .map(String::into_bytes)
        .map_err(encode_error)?;
    crate::checked_artifact::entry::create_merge_store_record(&root, &relative, &encoded)?;

    let published = StoredV1Record::from_open_bytes(&root, &path, &read_regular(&path)?)?;
    if published.record() != record {
        return Err(recovery(
            "checked v1 published record differs from the created record",
        ));
    }
    if !lease.covers(published.location()) {
        return Err(recovery("checked v1 creation is outside its lease"));
    }
    Ok(published)
}

/// Rewrite the exact existing open record.
///
/// **R2-E Step E4.3 — the converted rewrite path (O13's substantive half,
/// ConsumerCheckpoint §10 row `:280`).** The publication is a CHECKED ARTIFACT
/// ACTION now, not this module's own staged/rename/fsync:
/// `entry::rewrite_merge_store_record` replaces an expected-`Bytes` leaf, so the
/// boundary re-proves the exact existing source under its own retained parent
/// before publishing and barriers both areas afterwards. With `create_open`'s
/// half (E4.2) this module names `durable_fs` nowhere, which is O13's
/// "no legacy raw writer" clause discharged for the v1 store.
///
/// The frozen row's three clauses, and where each lives:
///
/// * **exact existing `MergeStore`** — driven twice over. Here, the record is
///   re-read and `same_source_as` the caller's `current` before anything is
///   serialized; then the boundary is handed those same bytes as its expected
///   fact, so a leaf that moved between the two refuses inside `replace_exact`
///   rather than being clobbered.
/// * **no parent creation** — `create_temporary`'s `create_dir_all` was this
///   path's only parent-creating primitive and it is gone with the staging it
///   served. Nothing replaces it: the boundary never creates a parent, and no
///   managed-parent bootstrap runs on a rewrite (see the door's doc).
/// * **unknown fields and exact reread preserved** — the overlay and the
///   `require_expected` pair are UNCHANGED; what moved is only the writer under
///   them. The pre-publication proof no longer needs a temporary of ours: the
///   bytes about to be published are decoded and checked in memory, and the
///   post-publication reread proves the durable leaf decodes to the same
///   record, the same unknown manifest and the same digest.
///
/// **TWO DISCLOSED CONSEQUENCES of riding this boundary, both measured, neither
/// owned by this step (2026-09-01, E4.3; the builder delivery's flags 1 and 2).**
///
/// 1. *The detach window.* The boundary replaces an existing leaf by moving it
///    into the private area and then publishing the goal no-replace; between
///    those two durable edges the open record does not exist, where
///    `rename_durable(temp, path, replace=true)` was atomic. Nothing is lost —
///    the prior and intended bytes are both durable in the private area — but
///    the lifecycle cannot reopen the merge from them and
///    `classify_open_record` enumerates `.gwz/merge/*.yaml`, so a workspace
///    interrupted there reports NO OPEN MERGE. The merge record is the one §10
///    leaf with no outer artifact to reconcile it from: every other converted
///    leaf is reconciled FROM this record. Driven at
///    `tests::store::an_interrupted_checked_rewrite_detaches_the_record_beyond_the_lifecycles_reach`.
/// 2. *Capability reach.* `CheckedArtifact::acquire` takes a durable object
///    identity, so every commit — including the reverse arms' on the
///    capability-free plain lease — now needs an admitted filesystem. Post-
///    publication aborts already did (`abort/evidence.rs`'s `artifact_facts`
///    calls); pre-publication ones did not. `capability.rs`'s shipped remedy
///    sentence still promises `gwz merge --abort` "needs no such filesystem".
pub(super) fn commit(
    lease: &V1MutationLease,
    current: &StoredV1Record,
    rewrite: PreparedV1Rewrite,
    fault: Option<CommitFault>,
) -> ModelResult<StoredV1Record> {
    if !lease.covers(current.location()) || rewrite.base_digest() != current.source_digest() {
        return Err(recovery(
            "checked v1 rewrite does not match its lease or source digest",
        ));
    }
    let root = current.location().root();
    let path = current.location().path();
    let source_bytes = read_regular(path)?;
    let reopened = StoredV1Record::from_open_bytes(root, path, &source_bytes)?;
    if !current.same_source_as(&reopened) {
        return Err(recovery("checked v1 source bytes changed before commit"));
    }

    rewrite
        .effect()
        .verify_known_diff(current.record(), rewrite.next())?;
    let mut raw = serde_yaml::to_value(rewrite.next()).map_err(encode_error)?;
    let expected_unknown = unknown::overlay(current, rewrite.effect(), &mut raw)?;
    let encoded = serde_yaml::to_string(&raw)
        .map(String::into_bytes)
        .map_err(encode_error)?;
    let intended = StoredV1Record::from_open_bytes(root, path, &encoded)?;
    require_expected(&intended, rewrite.next(), &expected_unknown)?;

    // The boundary is addressed by workspace-relative path, so the leaf it acts
    // on is proved to be the leaf just validated rather than assumed to be.
    let relative = PathBuf::from(".gwz/merge").join(format!("{}.yaml", current.record().merge_id));
    if root.join(&relative) != path {
        return Err(recovery(
            "checked v1 rewrite target is not its canonical open path",
        ));
    }
    if fault == Some(CommitFault::BeforePublication) {
        return Err(recovery("injected checked-store fault before publication"));
    }
    crate::checked_artifact::entry::rewrite_merge_store_record(
        root,
        &relative,
        &source_bytes,
        &encoded,
    )?;
    if fault == Some(CommitFault::AfterPublish) {
        return Err(recovery("injected checked-store fault after publication"));
    }

    let published_bytes = read_regular(path)?;
    let published = StoredV1Record::from_open_bytes(root, path, &published_bytes)?;
    require_expected(&published, rewrite.next(), &expected_unknown)?;
    if !intended.same_source_as(&published) {
        return Err(recovery(
            "checked v1 published bytes differ from the encoded rewrite",
        ));
    }
    Ok(published)
}

fn require_expected(
    actual: &StoredV1Record,
    expected_record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    expected_unknown: &crate::workspace_ops::merge::record_wire::UnknownFieldManifest,
) -> ModelResult<()> {
    if actual.record() == expected_record && actual.unknown_fields() == expected_unknown {
        Ok(())
    } else {
        Err(recovery(
            "checked v1 canonical model or unknown manifest changed during commit",
        ))
    }
}

pub(super) fn read_regular(path: &Path) -> ModelResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file() {
        return Err(unreadable(format!(
            "record path '{}' is not a regular file",
            path.display()
        )));
    }
    if path.canonicalize().map_err(io_error)? != path {
        return Err(unreadable(format!(
            "record path '{}' traverses a symbolic link",
            path.display()
        )));
    }
    fs::read(path).map_err(io_error)
}

pub(super) fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
    if merge_id.is_empty()
        || matches!(merge_id, "." | "..")
        || !merge_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(recovery(format!("invalid merge record id '{merge_id}'")));
    }
    Ok(())
}

pub(super) fn path_exists(path: &Path) -> ModelResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn io_error(error: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn encode_error(error: serde_yaml::Error) -> ModelError {
    ModelError::new(ErrorCode::InternalError, error.to_string())
}

pub(super) fn recovery(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail)
}

fn unreadable(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, detail)
}
