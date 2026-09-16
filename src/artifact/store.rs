//! Reading, writing and listing artifacts on a workspace path.

use super::*;

/// Load the workspace manifest.
///
/// This deliberately does NOT gate on conf integrity: the merge lane reads the manifest
/// through here while git is mid-rewrite of the conf files, and a refusal at this seam
/// displaces the merge lane's own errors. The gate lives at the command sites instead --
/// see [`assert_conf_unmodified_for`].
pub fn read_manifest(root: &Path) -> ModelResult<ManifestArtifact> {
    read_manifest_in(&make_filesystem(), root)
}

/// Load the workspace manifest through the caller's filesystem world.
pub(crate) fn read_manifest_in(
    filesystem: &dyn FileSystem,
    root: &Path,
) -> ModelResult<ManifestArtifact> {
    let result = (|| {
        let text = filesystem
            .read(&root.join(WORKSPACE_MANIFEST))
            .map_err(manifest_io_error)?;
        let text = String::from_utf8(text).map_err(|error| {
            manifest_io_error(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
        ManifestArtifact::from_yaml(&text)
    })();
    result.map_err(|error| workspace_path_hint(root, error))
}

pub(crate) fn workspace_path_hint(root: &Path, mut error: ModelError) -> ModelError {
    if let Some(name @ ("@root" | "@all")) = root.file_name().and_then(|name| name.to_str()) {
        error.message.push_str(&format!(
            "; workspace directory is {}. --root expects a directory path; to select repositories use --target {name}",
            root.display()
        ));
    }
    error
}

pub fn write_manifest(root: &Path, artifact: &ManifestArtifact) -> ModelResult<()> {
    write_manifest_in(&make_filesystem(), root, artifact)
}

/// Write the manifest through the caller's filesystem world.
///
/// The marker is refreshed through the same filesystem after the manifest publishes.
pub(crate) fn write_manifest_in(
    filesystem: &dyn FileSystem,
    root: &Path,
    artifact: &ManifestArtifact,
) -> ModelResult<()> {
    write_atomic_in(
        filesystem,
        &root.join(WORKSPACE_MANIFEST),
        artifact.to_yaml()?,
    )?;
    conf_integrity::refresh_conf_integrity_marker_in(filesystem, root)
}

pub fn read_lock(root: &Path) -> ModelResult<LockArtifact> {
    read_lock_in(&make_filesystem(), root)
}

/// Load the lock through the caller's filesystem world.
pub(crate) fn read_lock_in(filesystem: &dyn FileSystem, root: &Path) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(&read_to_string_in(filesystem, root.join(LOCK_PATH))?)
}

pub fn write_lock(root: &Path, artifact: &LockArtifact) -> ModelResult<()> {
    write_atomic(&root.join(LOCK_PATH), artifact.to_yaml()?)?;
    conf_integrity::refresh_conf_integrity_marker(root)
}

pub fn read_snapshot(root: &Path, snapshot_id: &str) -> ModelResult<SnapshotArtifact> {
    read_snapshot_with(root, snapshot_id, read_to_string)
}

pub(super) fn read_snapshot_with(
    root: &Path,
    snapshot_id: &str,
    read: impl FnOnce(PathBuf) -> ModelResult<String>,
) -> ModelResult<SnapshotArtifact> {
    let path = snapshot_path(root, snapshot_id)?;
    let artifact = SnapshotArtifact::from_yaml(&read(path)?)?;
    if artifact.snapshot_id != snapshot_id {
        return Err(invalid(format!(
            "snapshot filename id '{snapshot_id}' does not match embedded snapshot_id '{}'",
            artifact.snapshot_id
        )));
    }
    Ok(artifact)
}

pub fn write_snapshot(root: &Path, artifact: &SnapshotArtifact) -> ModelResult<()> {
    let yaml = artifact.to_yaml()?;
    write_atomic(&snapshot_path(root, &artifact.snapshot_id)?, yaml)
}

/// All snapshots in the workspace, sorted by file name. A missing dir is an empty list.
pub fn list_snapshots(root: &Path) -> ModelResult<Vec<SnapshotArtifact>> {
    list_artifacts(root.join(SNAPSHOT_DIR), SnapshotArtifact::from_yaml)
}

/// Snapshot ids present on disk, without opening artifact bodies.
///
/// Operand lowering uses this filename-only view so an exact legacy v0 id wins
/// over range punctuation before either diff or log reads the referenced body.
pub(crate) fn snapshot_ids_for_operand_parsing(root: &Path) -> ModelResult<Vec<String>> {
    let dir = root.join(SNAPSHOT_DIR);
    let mut ids = match fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?
            .into_iter()
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"))
            .filter_map(|path| {
                let id = path.file_stem()?.to_str()?.to_owned();
                validate_snapshot_id_for_read(&id).is_ok().then_some(id)
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(io_error(error)),
    };
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn read_marker(root: &Path, gwz_commit_id: &str) -> ModelResult<MarkerArtifact> {
    MarkerArtifact::from_yaml(&read_to_string(marker_path(root, gwz_commit_id))?)
}

pub fn write_marker(root: &Path, artifact: &MarkerArtifact) -> ModelResult<()> {
    write_atomic(
        &marker_path(root, &artifact.gwz_commit_id),
        artifact.to_yaml()?,
    )
}

/// All commit markers in the workspace, sorted by file name. A missing dir is an empty list.
pub fn list_markers(root: &Path) -> ModelResult<Vec<MarkerArtifact>> {
    list_artifacts(root.join(MARKER_DIR), MarkerArtifact::from_yaml)
}

/// Read + parse every `*.yaml` in `dir`, path-sorted. A missing dir yields an empty list.
pub(super) fn list_artifacts<T>(
    dir: PathBuf,
    parse: impl Fn(&str) -> ModelResult<T>,
) -> ModelResult<Vec<T>> {
    let mut paths: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(read) => read
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error)?
            .into_iter()
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|path| read_to_string(path).and_then(|yaml| parse(&yaml)))
        .collect()
}
