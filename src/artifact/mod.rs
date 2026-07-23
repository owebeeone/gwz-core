use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::{MemberPath, WORKSPACE_MANIFEST};

mod merge_marker;

pub use merge_marker::{
    MarkerMergeArtifact, MarkerMergeParticipantArtifact, MarkerMergeTargetKind,
};

pub const WORKSPACE_SCHEMA: &str = "gwz.workspace/v0";
pub const LOCK_SCHEMA: &str = "gwz.lock/v0";
pub const SNAPSHOT_SCHEMA: &str = "gwz.snapshot/v0";
pub const MARKER_SCHEMA: &str = "gwz.marker/v0";
pub const LOCK_PATH: &str = "gwz.conf/gwz.lock.yml";
pub const SNAPSHOT_DIR: &str = "gwz.conf/snapshots";
pub const MARKER_DIR: &str = "gwz.conf/markers";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestArtifact {
    pub schema: String,
    pub workspace: WorkspaceHeader,
    pub members: Vec<ManifestMember>,
}

impl ManifestArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, WORKSPACE_SCHEMA)?;
        parse_id("workspace.id", "ws_", &self.workspace.id)?;

        let mut member_ids = BTreeSet::new();
        let mut active_paths = Vec::with_capacity(self.members.len());
        for member in &self.members {
            member.validate()?;
            if !member_ids.insert(member.id.as_str()) {
                return Err(invalid(format!("duplicate member id '{}'", member.id)));
            }
            if member.active {
                active_paths.push(MemberPath::parse(&member.path)?);
            }
        }
        crate::workspace::validate_member_path_set(&active_paths)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceHeader {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestMember {
    pub id: String,
    pub path: String,
    #[serde(rename = "type")]
    pub source_kind: ArtifactSourceKind,
    pub source_id: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired: Option<DesiredRefArtifact>,
    pub remotes: Vec<RemoteArtifact>,
}

impl ManifestMember {
    fn validate(&self) -> ModelResult<()> {
        parse_id("member.id", "mem_", &self.id)?;
        parse_id("member.source_id", "src_", &self.source_id)?;
        MemberPath::parse(&self.path)?;
        if let Some(desired) = &self.desired {
            desired.validate()?;
        }
        reject_duplicate_remote_names(&self.remotes)?;
        for remote in &self.remotes {
            remote.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredRefArtifact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_only: Option<bool>,
}

impl DesiredRefArtifact {
    fn validate(&self) -> ModelResult<()> {
        if self.local_only == Some(false) {
            return Err(invalid("desired.local_only must be true when present"));
        }

        let mut targets = 0;
        targets += optional_text_target("desired.branch", &self.branch)?;
        targets += optional_text_target("desired.commit", &self.commit)?;
        targets += optional_text_target("desired.git_tag", &self.git_tag)?;
        if self.local_only == Some(true) {
            targets += 1;
        }

        if targets == 1 {
            Ok(())
        } else {
            Err(invalid("desired ref must specify exactly one target"))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteArtifact {
    pub name: String,
    pub url: String,
    pub fetch: bool,
    pub push: bool,
}

impl RemoteArtifact {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("remote.name", &self.name)?;
        require_non_empty("remote.url", &self.url)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockArtifact {
    pub schema: String,
    pub workspace_id: String,
    pub manifest_schema: String,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
}

impl LockArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, LOCK_SCHEMA)?;
        require_schema(&self.manifest_schema, WORKSPACE_SCHEMA)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        for (member_id, member) in &self.members {
            parse_id("member id", "mem_", member_id)?;
            member.validate(true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedMemberArtifact {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub source_kind: ArtifactSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialized: Option<bool>,
}

impl ResolvedMemberArtifact {
    fn validate(&self, require_source_id: bool) -> ModelResult<()> {
        MemberPath::parse(&self.path)?;
        if require_source_id {
            let source_id = self
                .source_id
                .as_ref()
                .ok_or_else(|| invalid("resolved member source_id is required"))?;
            parse_id("member.source_id", "src_", source_id)?;
        } else if let Some(source_id) = &self.source_id {
            parse_id("member.source_id", "src_", source_id)?;
        }
        validate_optional_text("commit", &self.commit)?;
        validate_optional_text("branch", &self.branch)?;
        validate_optional_text("upstream", &self.upstream)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotArtifact {
    pub schema: String,
    pub workspace_id: String,
    pub snapshot_id: String,
    pub created_at: String,
    pub created_by: CreatedByArtifact,
    pub selected_members: Vec<String>,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
}

impl SnapshotArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, SNAPSHOT_SCHEMA)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        require_slug("snapshot_id", &self.snapshot_id)?;
        validate_member_record(
            &self.created_at,
            &self.created_by,
            &self.selected_members,
            &self.members,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerArtifact {
    pub schema: String,
    pub gwz_commit_id: String,
    pub workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_url_hash: Option<String>,
    pub created_at: String,
    pub created_by: CreatedByArtifact,
    pub root: MarkerRootArtifact,
    pub selected_targets: Vec<String>,
    pub committed_targets: Vec<String>,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<MarkerMergeArtifact>,
}

impl MarkerArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, MARKER_SCHEMA)?;
        require_uuid_v7("gwz_commit_id", &self.gwz_commit_id)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        if let Some(hash) = &self.origin_url_hash {
            validate_origin_url_hash(hash)?;
        }
        self.root.validate()?;
        for target in &self.selected_targets {
            validate_target_ref("selected target", target)?;
        }
        for target in &self.committed_targets {
            validate_target_ref("committed target", target)?;
        }
        if let Some(merge) = &self.merge {
            if merge.selected_targets != self.selected_targets {
                return Err(invalid(
                    "marker selected_targets must match merge selected_targets",
                ));
            }
            merge.validate()?;
        }
        validate_member_record(&self.created_at, &self.created_by, &[], &self.members)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerRootArtifact {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl MarkerRootArtifact {
    fn validate(&self) -> ModelResult<()> {
        if self.path != "." {
            return Err(invalid("marker root.path must be ."));
        }
        validate_optional_text("root.before_commit", &self.before_commit)?;
        validate_optional_text("root.branch", &self.branch)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedByArtifact {
    pub actor_id: String,
}

impl CreatedByArtifact {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("created_by.actor_id", &self.actor_id)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSourceKind {
    #[default]
    Git,
    Archive,
    Package,
    Local,
    Generated,
}

pub fn read_manifest(root: &Path) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(&read_to_string(root.join(WORKSPACE_MANIFEST))?)
}

pub fn write_manifest(root: &Path, artifact: &ManifestArtifact) -> ModelResult<()> {
    write_atomic(&root.join(WORKSPACE_MANIFEST), artifact.to_yaml()?)
}

pub fn read_lock(root: &Path) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(&read_to_string(root.join(LOCK_PATH))?)
}

pub fn write_lock(root: &Path, artifact: &LockArtifact) -> ModelResult<()> {
    write_atomic(&root.join(LOCK_PATH), artifact.to_yaml()?)
}

pub fn read_snapshot(root: &Path, snapshot_id: &str) -> ModelResult<SnapshotArtifact> {
    SnapshotArtifact::from_yaml(&read_to_string(snapshot_path(root, snapshot_id))?)
}

pub fn write_snapshot(root: &Path, artifact: &SnapshotArtifact) -> ModelResult<()> {
    write_atomic(
        &snapshot_path(root, &artifact.snapshot_id),
        artifact.to_yaml()?,
    )
}

/// All snapshots in the workspace, sorted by file name. A missing dir is an empty list.
pub fn list_snapshots(root: &Path) -> ModelResult<Vec<SnapshotArtifact>> {
    list_artifacts(root.join(SNAPSHOT_DIR), SnapshotArtifact::from_yaml)
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
fn list_artifacts<T>(dir: PathBuf, parse: impl Fn(&str) -> ModelResult<T>) -> ModelResult<Vec<T>> {
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

pub fn write_atomic(path: &Path, contents: impl AsRef<str>) -> ModelResult<()> {
    let staged = stage_durably(path, contents.as_ref())?;
    publish_staged(&staged, path)
}

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
    let manifest_path = root.join(WORKSPACE_MANIFEST);
    let lock_path = root.join(LOCK_PATH);
    let manifest_staged = stage_durably(&manifest_path, &manifest.to_yaml()?)?;
    let lock_staged = stage_durably(&lock_path, &lock.to_yaml()?)?;
    publish_staged(&manifest_staged, &manifest_path)?;
    publish_staged(&lock_staged, &lock_path)?;
    Ok(())
}

/// Write `contents` to a unique temp beside `path` and fsync it, returning the staged temp
/// path. On success the bytes are durably on disk, ready for `publish_staged`.
fn stage_durably(path: &Path, contents: &str) -> ModelResult<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let tmp_path = temp_path(path)?;
    let write = || -> ModelResult<()> {
        // F12: fsync the bytes to disk before the rename publishes them. Sync the SAME
        // writable handle we wrote through — do NOT reopen read-only, because Windows
        // rejects FlushFileBuffers on a read-only handle with ERROR_ACCESS_DENIED.
        let mut file = fs::File::create(&tmp_path).map_err(io_error)?;
        file.write_all(contents.as_bytes()).map_err(io_error)?;
        file.sync_all().map_err(io_error)
    };
    if let Err(err) = write() {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(tmp_path)
}

/// Publish a staged temp to `path` (atomic rename) and best-effort fsync the directory so
/// the rename entry itself survives a crash.
fn publish_staged(tmp_path: &Path, path: &Path) -> ModelResult<()> {
    if let Err(err) = fs::rename(tmp_path, path).map_err(io_error) {
        let _ = fs::remove_file(tmp_path);
        return Err(err);
    }
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn parse_yaml<T>(text: &str) -> ModelResult<T>
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

fn emit_yaml<T>(value: &T) -> ModelResult<String>
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

fn read_to_string(path: PathBuf) -> ModelResult<String> {
    fs::read_to_string(path).map_err(io_error)
}

pub(crate) fn snapshot_path(root: &Path, snapshot_id: &str) -> PathBuf {
    root.join(SNAPSHOT_DIR).join(format!("{snapshot_id}.yaml"))
}

pub fn marker_path(root: &Path, gwz_commit_id: &str) -> PathBuf {
    root.join(MARKER_DIR).join(format!("{gwz_commit_id}.yaml"))
}

fn temp_path(path: &Path) -> ModelResult<PathBuf> {
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

static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn require_schema(actual: &str, expected: &str) -> ModelResult<()> {
    let expected_major =
        schema_major(expected).ok_or_else(|| invalid("invalid expected schema"))?;
    match schema_major(actual) {
        Some(actual_major) if actual == expected && actual_major == expected_major => Ok(()),
        Some(_) => Err(ModelError::new(
            ErrorCode::SchemaUnsupported,
            format!("unsupported schema {actual}; expected {expected}"),
        )),
        None => Err(ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("invalid schema {actual}"),
        )),
    }
}

fn schema_major(schema: &str) -> Option<u32> {
    let (_, major) = schema.rsplit_once("/v")?;
    major.parse().ok()
}

fn validate_member_record(
    created_at: &str,
    created_by: &CreatedByArtifact,
    selected_members: &[String],
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> ModelResult<()> {
    require_non_empty("created_at", created_at)?;
    created_by.validate()?;
    for member_id in selected_members {
        parse_id("selected member", "mem_", member_id)?;
    }
    for (member_id, member) in members {
        parse_id("member id", "mem_", member_id)?;
        member.validate(false)?;
    }
    Ok(())
}

fn validate_target_ref(field: &str, value: &str) -> ModelResult<()> {
    if value == "@root" || value == "@default" {
        return Ok(());
    }
    if value.starts_with('@') {
        return require_non_empty(field, value);
    }
    parse_id(field, "mem_", value)
}

fn require_uuid_v7(field: &str, value: &str) -> ModelResult<()> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[14] == b'7'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes.iter().enumerate().all(|(idx, byte)| {
            [8, 13, 18, 23].contains(&idx) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
        });
    if valid {
        Ok(())
    } else {
        Err(invalid(format!("{field} must be a canonical UUIDv7")))
    }
}

fn validate_origin_url_hash(value: &str) -> ModelResult<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(invalid("origin_url_hash must start with sha256:"));
    };
    let valid = hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "origin_url_hash must be sha256:<64 lowercase hex chars>",
        ))
    }
}

fn reject_duplicate_remote_names(remotes: &[RemoteArtifact]) -> ModelResult<()> {
    let mut names = BTreeSet::new();
    for remote in remotes {
        if !names.insert(remote.name.as_str()) {
            return Err(invalid(format!("duplicate remote name '{}'", remote.name)));
        }
    }
    Ok(())
}

fn parse_id(field: &str, prefix: &str, value: &str) -> ModelResult<()> {
    let valid = value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must start with {prefix} and contain only portable characters"
        )))
    }
}

fn require_slug(field: &str, value: &str) -> ModelResult<()> {
    require_non_empty(field, value)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must contain only portable slug characters"
        )))
    }
}

fn optional_text_target(field: &str, value: &Option<String>) -> ModelResult<usize> {
    match value {
        Some(value) => {
            require_non_empty(field, value)?;
            Ok(1)
        }
        None => Ok(0),
    }
}

fn validate_optional_text(field: &str, value: &Option<String>) -> ModelResult<()> {
    match value {
        Some(value) => require_non_empty(field, value),
        None => Ok(()),
    }
}

fn require_non_empty(field: &str, value: &str) -> ModelResult<()> {
    if value.trim().is_empty() {
        Err(invalid(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

fn io_error(err: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, err.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::model::ErrorCode;

    use super::*;

    const MANIFEST_GOLDEN: &str = "schema: gwz.workspace/v0\nworkspace:\n  id: ws_01\nmembers:\n- id: mem_01\n  path: repos/example\n  type: git\n  source_id: src_01\n  active: true\n  desired:\n    branch: main\n  remotes:\n  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n";

    const LOCK_GOLDEN: &str = "schema: gwz.lock/v0\nworkspace_id: ws_01\nmanifest_schema: gwz.workspace/v0\nmembers:\n  mem_01:\n    path: repos/example\n    source_id: src_01\n    source_kind: git\n    commit: abc123\n    branch: main\n    detached: false\n    upstream: origin/main\n    dirty: false\n    materialized: true\n";

    const SNAPSHOT_GOLDEN: &str = "schema: gwz.snapshot/v0\nworkspace_id: ws_01\nsnapshot_id: snap_demo\ncreated_at: 2026-06-15T00:00:00Z\ncreated_by:\n  actor_id: agent_01\nselected_members:\n- mem_01\nmembers:\n  mem_01:\n    path: repos/example\n    source_kind: git\n    commit: abc123\n";

    #[test]
    fn manifest_round_trips_and_matches_golden_yaml() {
        let manifest = sample_manifest();

        assert_eq!(manifest.to_yaml().unwrap(), MANIFEST_GOLDEN);
        assert_eq!(
            ManifestArtifact::from_yaml(MANIFEST_GOLDEN).unwrap(),
            manifest
        );
    }

    #[test]
    fn lock_and_snapshot_round_trip_and_match_golden_yaml() {
        assert_eq!(sample_lock().to_yaml().unwrap(), LOCK_GOLDEN);
        assert_eq!(LockArtifact::from_yaml(LOCK_GOLDEN).unwrap(), sample_lock());

        assert_eq!(sample_snapshot().to_yaml().unwrap(), SNAPSHOT_GOLDEN);
        assert_eq!(
            SnapshotArtifact::from_yaml(SNAPSHOT_GOLDEN).unwrap(),
            sample_snapshot()
        );
    }

    #[test]
    fn marker_round_trips() {
        let marker = sample_marker();
        let yaml = marker.to_yaml().unwrap();

        assert_eq!(MarkerArtifact::from_yaml(&yaml).unwrap(), marker);
    }

    #[test]
    fn marker_merge_targets_must_match_outer_marker_targets() {
        let mut marker = sample_marker();
        marker.merge = Some(MarkerMergeArtifact {
            merge_id: "merge_1".to_owned(),
            operation_id: "op_1".to_owned(),
            source_ref: "feature/x".to_owned(),
            selected_targets: vec!["mem_01".to_owned()],
            participants: [(
                "mem_01".to_owned(),
                MarkerMergeParticipantArtifact {
                    target_kind: MarkerMergeTargetKind::Member,
                    target_branch: "main".to_owned(),
                    before_commit: "before".to_owned(),
                    source_commit: "source".to_owned(),
                    resulting_commit: "result".to_owned(),
                },
            )]
            .into(),
            root_merge_commit: None,
        });

        assert_eq!(
            marker.validate().unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn unsupported_major_schema_versions_fail_with_typed_error() {
        let manifest = MANIFEST_GOLDEN.replace("gwz.workspace/v0", "gwz.workspace/v1");
        let lock = LOCK_GOLDEN.replacen("gwz.lock/v0", "gwz.lock/v1", 1);
        let snapshot = SNAPSHOT_GOLDEN.replace("gwz.snapshot/v0", "gwz.snapshot/v1");
        let marker = sample_marker()
            .to_yaml()
            .unwrap()
            .replace("gwz.marker/v0", "gwz.marker/v1");

        assert_eq!(
            ManifestArtifact::from_yaml(&manifest).unwrap_err().code,
            ErrorCode::SchemaUnsupported
        );
        assert_eq!(
            LockArtifact::from_yaml(&lock).unwrap_err().code,
            ErrorCode::SchemaUnsupported
        );
        assert_eq!(
            SnapshotArtifact::from_yaml(&snapshot).unwrap_err().code,
            ErrorCode::SchemaUnsupported
        );
        assert_eq!(
            MarkerArtifact::from_yaml(&marker).unwrap_err().code,
            ErrorCode::SchemaUnsupported
        );
    }

    #[test]
    fn manifest_reader_rejects_duplicate_remote_names() {
        let yaml = MANIFEST_GOLDEN.replace(
            "  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n",
            "  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n  - name: origin\n    url: git@example.invalid:example-2.git\n    fetch: true\n    push: false\n",
        );

        assert_eq!(
            ManifestArtifact::from_yaml(&yaml).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn manifest_rejects_duplicate_member_ids_across_active_and_inactive_rows() {
        let mut manifest = sample_manifest();
        let mut historical = manifest.members[0].clone();
        historical.path = "repos/historical".to_owned();
        historical.active = false;
        manifest.members.push(historical);

        let error = manifest.validate().unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.message.contains("duplicate member id 'mem_01'"));
    }

    #[test]
    fn manifest_allows_shared_source_ids_and_inactive_path_overlap() {
        let mut manifest = sample_manifest();
        let mut replacement = manifest.members[0].clone();
        replacement.id = "mem_02".to_owned();
        replacement.source_id = manifest.members[0].source_id.clone();
        manifest.members[0].active = false;
        manifest.members.push(replacement);

        manifest.validate().unwrap();
    }

    #[test]
    fn manifest_rejects_overlap_between_active_rows_only() {
        let mut manifest = sample_manifest();
        let mut nested = manifest.members[0].clone();
        nested.id = "mem_02".to_owned();
        nested.path = "repos/example/tools".to_owned();
        nested.source_id = "src_02".to_owned();
        manifest.members.push(nested);

        assert_eq!(
            manifest.validate().unwrap_err().code,
            ErrorCode::PathCollision
        );

        manifest.members[1].active = false;
        manifest.validate().unwrap();
    }

    #[test]
    fn artifact_file_io_uses_workspace_paths() {
        let temp = TempDir::new("artifact-io");
        write_manifest(temp.path(), &sample_manifest()).unwrap();
        write_lock(temp.path(), &sample_lock()).unwrap();
        write_snapshot(temp.path(), &sample_snapshot()).unwrap();
        write_marker(temp.path(), &sample_marker()).unwrap();

        assert_eq!(read_manifest(temp.path()).unwrap(), sample_manifest());
        assert_eq!(read_lock(temp.path()).unwrap(), sample_lock());
        assert_eq!(
            read_snapshot(temp.path(), "snap_demo").unwrap(),
            sample_snapshot()
        );
        assert_eq!(
            read_marker(temp.path(), &sample_marker().gwz_commit_id).unwrap(),
            sample_marker()
        );

        assert!(
            temp.path()
                .join("gwz.conf/snapshots/snap_demo.yaml")
                .is_file()
        );
        assert!(
            temp.path()
                .join("gwz.conf/markers/01987b0c-2f75-7c4a-9a32-8fd22f7d7c91.yaml")
                .is_file()
        );
    }

    #[test]
    fn list_snapshots_reads_sorted_entries() {
        let temp = TempDir::new("artifact-list");
        // No dir yet → empty, not an error.
        assert!(list_snapshots(temp.path()).unwrap().is_empty());

        write_snapshot(temp.path(), &sample_snapshot()).unwrap(); // "snap_demo"
        let mut alpha = sample_snapshot();
        alpha.snapshot_id = "snap_alpha".to_owned();
        write_snapshot(temp.path(), &alpha).unwrap();
        let snapshots = list_snapshots(temp.path()).unwrap();
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .collect::<Vec<_>>(),
            vec!["snap_alpha", "snap_demo"]
        );
    }

    #[test]
    fn list_markers_reads_sorted_entries() {
        let temp = TempDir::new("marker-list");
        // No dir yet -> empty, not an error.
        assert!(list_markers(temp.path()).unwrap().is_empty());

        let marker = sample_marker();
        write_marker(temp.path(), &marker).unwrap();
        let mut alpha = marker.clone();
        alpha.gwz_commit_id = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c92".to_owned();
        write_marker(temp.path(), &alpha).unwrap();
        let markers = list_markers(temp.path()).unwrap();
        assert_eq!(
            markers
                .iter()
                .map(|marker| marker.gwz_commit_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91",
                "01987b0c-2f75-7c4a-9a32-8fd22f7d7c92"
            ]
        );
    }

    #[test]
    fn atomic_write_replaces_existing_file_without_leftover_temp() {
        let temp = TempDir::new("atomic");
        let target = temp.path().join("nested/file.txt");

        write_atomic(&target, "old").unwrap();
        write_atomic(&target, "new").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(!temp.path().join("nested/file.txt.tmp").exists());
    }

    fn sample_manifest() -> ManifestArtifact {
        ManifestArtifact {
            schema: WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: "ws_01".to_owned(),
            },
            members: vec![ManifestMember {
                id: "mem_01".to_owned(),
                path: "repos/example".to_owned(),
                source_kind: ArtifactSourceKind::Git,
                source_id: "src_01".to_owned(),
                active: true,
                desired: Some(DesiredRefArtifact {
                    branch: Some("main".to_owned()),
                    ..DesiredRefArtifact::default()
                }),
                remotes: vec![RemoteArtifact {
                    name: "origin".to_owned(),
                    url: "git@example.invalid:example.git".to_owned(),
                    fetch: true,
                    push: true,
                }],
            }],
        }
    }

    fn sample_lock() -> LockArtifact {
        LockArtifact {
            schema: LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_01".to_owned(),
            manifest_schema: WORKSPACE_SCHEMA.to_owned(),
            members: [("mem_01".to_owned(), sample_resolved_member())].into(),
        }
    }

    fn sample_snapshot() -> SnapshotArtifact {
        SnapshotArtifact {
            schema: SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: "ws_01".to_owned(),
            snapshot_id: "snap_demo".to_owned(),
            created_at: "2026-06-15T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "agent_01".to_owned(),
            },
            selected_members: vec!["mem_01".to_owned()],
            members: [("mem_01".to_owned(), sample_short_member())].into(),
        }
    }

    fn sample_marker() -> MarkerArtifact {
        MarkerArtifact {
            schema: MARKER_SCHEMA.to_owned(),
            gwz_commit_id: "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91".to_owned(),
            workspace_id: "ws_01".to_owned(),
            origin_url_hash: Some(format!("sha256:{}", "0".repeat(64))),
            created_at: "2026-06-15T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "agent_01".to_owned(),
            },
            root: MarkerRootArtifact {
                path: ".".to_owned(),
                before_commit: Some("abc123".to_owned()),
                branch: Some("main".to_owned()),
            },
            selected_targets: vec!["@root".to_owned(), "mem_01".to_owned()],
            committed_targets: vec!["mem_01".to_owned(), "@root".to_owned()],
            members: [("mem_01".to_owned(), sample_short_member())].into(),
            merge: None,
        }
    }

    fn sample_resolved_member() -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: "repos/example".to_owned(),
            source_id: Some("src_01".to_owned()),
            source_kind: ArtifactSourceKind::Git,
            commit: Some("abc123".to_owned()),
            branch: Some("main".to_owned()),
            detached: Some(false),
            upstream: Some("origin/main".to_owned()),
            dirty: Some(false),
            materialized: Some(true),
        }
    }

    fn sample_short_member() -> ResolvedMemberArtifact {
        ResolvedMemberArtifact {
            path: "repos/example".to_owned(),
            source_kind: ArtifactSourceKind::Git,
            commit: Some("abc123".to_owned()),
            ..ResolvedMemberArtifact::default()
        }
    }

    #[test]
    fn write_atomic_publishes_content_and_leaves_no_temp_file() {
        // F12: the durable write lands the exact bytes and cleans up after itself — no
        // fixed-name `.tmp` lingering to race a concurrent writer.
        let temp = TempDir::new("write-atomic");
        let target = temp.path().join("lock.yaml");

        write_atomic(&target, "first\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first\n");
        write_atomic(&target, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "second\n");

        let leftovers = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn write_manifest_and_lock_publishes_both_consistently() {
        // F14: the consistency-critical pair is published together (lock last), durably,
        // leaving no temp litter.
        let temp = TempDir::new("manifest-lock");
        let manifest = sample_manifest();
        let lock = sample_lock();

        write_manifest_and_lock(temp.path(), &manifest, &lock).unwrap();

        assert_eq!(read_manifest(temp.path()).unwrap(), manifest);
        assert_eq!(read_lock(temp.path()).unwrap(), lock);
        let leftovers = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gwz-core-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
