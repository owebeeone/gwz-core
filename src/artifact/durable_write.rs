//! Durable publication: staging bytes, fsyncing them, then publishing the
//! manifest and lock together.

use super::*;

/// F14: write the manifest and lock together. True cross-file atomicity isn't possible on
/// a POSIX filesystem, so stage BOTH durably first, then publish back-to-back with the
/// LOCK LAST. A crash can then leave at worst a stale lock (rebuildable from the manifest
/// and git state), never a lock referencing a member the manifest doesn't have. This is
/// the single seam for the consistency-critical pair so the safe ordering can't reverse.
pub fn write_manifest_and_lock(
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
) -> ModelResult<()> {
    write_manifest_and_lock_in(&make_filesystem(), root, manifest, lock)
}

/// Publish a manifest/lock pair through the caller's filesystem world.
pub(crate) fn write_manifest_and_lock_in(
    filesystem: &dyn FileSystem,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
) -> ModelResult<()> {
    let manifest_path = root.join(WORKSPACE_MANIFEST);
    let lock_path = root.join(LOCK_PATH);
    let manifest_staged =
        stage_durably(filesystem, &manifest_path, manifest.to_yaml()?.as_bytes())?;
    let lock_staged = stage_durably(filesystem, &lock_path, lock.to_yaml()?.as_bytes())?;
    publish_staged(filesystem, &manifest_staged, &manifest_path)?;
    publish_staged(filesystem, &lock_staged, &lock_path)?;
    // One refresh after both are published: the marker never records a half-written pair.
    conf_integrity::refresh_conf_integrity_marker_in(filesystem, root)
}

/// Write `contents` to a unique temp beside `path` and fsync it, returning the staged temp
/// path. On success the bytes are durably on disk, ready for `publish_staged`.
pub(super) fn stage_durably(
    filesystem: &dyn FileSystem,
    path: &Path,
    contents: &[u8],
) -> ModelResult<PathBuf> {
    if let Some(parent) = path.parent() {
        filesystem.create_directories(parent).map_err(io_error)?;
    }
    let tmp_path = temp_path(path)?;
    let write = || -> ModelResult<()> {
        // F12: fsync the bytes to disk before the rename publishes them. Sync the SAME
        // writable handle we wrote through — do NOT reopen read-only, because Windows
        // rejects FlushFileBuffers on a read-only handle with ERROR_ACCESS_DENIED.
        let file = filesystem.create_file(&tmp_path).map_err(io_error)?;
        filesystem.write_all(&file, contents).map_err(io_error)?;
        filesystem.sync_file(&file).map_err(io_error)
    };
    if let Err(err) = write() {
        let _ = filesystem.remove_file(&tmp_path);
        return Err(err);
    }
    Ok(tmp_path)
}

/// Publish a staged temp to `path` (atomic rename) and best-effort fsync the directory so
/// the rename entry itself survives a crash.
pub(super) fn publish_staged(
    filesystem: &dyn FileSystem,
    tmp_path: &Path,
    path: &Path,
) -> ModelResult<()> {
    if let Err(err) = filesystem
        .rename(tmp_path, path, RenameMode::Replace)
        .map_err(io_error)
    {
        let _ = filesystem.remove_file(tmp_path);
        return Err(err);
    }
    if let Some(parent) = path.parent() {
        let _ = filesystem.sync_directory(parent);
    }
    Ok(())
}
