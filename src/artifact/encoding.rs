//! YAML round-tripping, artifact path construction and temp-file naming.

use super::*;

pub(super) fn parse_yaml<T>(text: &str) -> ModelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_yaml::from_str(text).map_err(|err| {
        ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("failed to parse artifact YAML: {err}"),
        )
    })
}

pub(super) fn emit_yaml<T>(value: &T) -> ModelResult<String>
where
    T: Serialize,
{
    serde_yaml::to_string(value).map_err(|err| {
        ModelError::new(
            ErrorCode::InternalError,
            format!("failed to serialize artifact YAML: {err}"),
        )
    })
}

pub(super) fn read_to_string(path: PathBuf) -> ModelResult<String> {
    read_to_string_in(&make_filesystem(), path)
}

pub(crate) fn read_to_string_in(filesystem: &dyn FileSystem, path: PathBuf) -> ModelResult<String> {
    String::from_utf8(filesystem.read(&path).map_err(io_error)?)
        .map_err(|error| io_error(io::Error::new(io::ErrorKind::InvalidData, error)))
}

pub(crate) fn snapshot_path(root: &Path, snapshot_id: &str) -> ModelResult<PathBuf> {
    validate_snapshot_id_for_read(snapshot_id)?;
    Ok(root.join(SNAPSHOT_DIR).join(format!("{snapshot_id}.yaml")))
}

pub fn marker_path(root: &Path, gwz_commit_id: &str) -> PathBuf {
    root.join(MARKER_DIR).join(format!("{gwz_commit_id}.yaml"))
}

pub(super) fn temp_path(path: &Path) -> ModelResult<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("atomic write target must have a file name"))?;
    // F12: a unique temp name per process + per call so concurrent writers (or a stale
    // temp left by a crashed prior write) never collide; the rename publishes atomically.
    let pid = std::process::id();
    let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(path.with_file_name(format!("{file_name}.{pid}.{seq}.tmp")))
}

pub(super) static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
