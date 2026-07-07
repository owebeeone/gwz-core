use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::artifact::write_atomic;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::{MemberPath, RUNTIME_DIR};

pub const STASH_BUNDLE_SCHEMA: &str = "gwz.stash-bundle/v0";
pub const STASH_BUNDLE_DIR: &str = ".gwz/stash/bundles";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashBundle {
    pub schema: String,
    pub workspace_id: String,
    pub stash_id: String,
    pub created_at: String,
    pub message_suffix: String,
    pub include_untracked: bool,
    pub include_ignored: bool,
    pub selected_members: Vec<String>,
    pub members: Vec<StashBundleMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<StashWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<StashDrift>,
}

impl StashBundle {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = serde_yaml::from_str(text).map_err(|err| {
            ModelError::new(
                ErrorCode::ManifestInvalid,
                format!("failed to parse stash bundle YAML: {err}"),
            )
        })?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        serde_yaml::to_string(self).map_err(|err| {
            ModelError::new(
                ErrorCode::InternalError,
                format!("failed to serialize stash bundle YAML: {err}"),
            )
        })
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, STASH_BUNDLE_SCHEMA)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        parse_id("stash_id", "stash_", &self.stash_id)?;
        require_non_empty("created_at", &self.created_at)?;
        require_non_empty("message_suffix", &self.message_suffix)?;

        let mut selected = BTreeSet::new();
        for member_id in &self.selected_members {
            parse_id("selected_members", "mem_", member_id)?;
            if !selected.insert(member_id.as_str()) {
                return Err(invalid(format!(
                    "duplicate selected stash member '{member_id}'"
                )));
            }
        }

        let mut seen = BTreeSet::new();
        for member in &self.members {
            member.validate()?;
            if !seen.insert(member.member_id.as_str()) {
                return Err(invalid(format!(
                    "duplicate stash member record '{}'",
                    member.member_id
                )));
            }
            if !selected.contains(member.member_id.as_str()) {
                return Err(invalid(format!(
                    "stash member '{}' was not part of selected_members",
                    member.member_id
                )));
            }
        }
        for member_id in &self.selected_members {
            if !seen.contains(member_id.as_str()) {
                return Err(invalid(format!(
                    "selected stash member '{member_id}' has no member record"
                )));
            }
        }
        for warning in &self.warnings {
            warning.validate()?;
        }
        for drift in &self.drift {
            drift.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashBundleMember {
    pub member_id: String,
    pub path: String,
    pub participation: StashParticipation,
    pub push_lifecycle: StashPushLifecycle,
    pub restore_state: StashRestoreState,
    pub branch_before: Option<String>,
    pub head_before: Option<String>,
    pub full_stash_message: String,
    pub dirty_summary: StashDirtySummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_stash_object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_stash_display_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<StashErrorDetail>,
}

impl StashBundleMember {
    fn validate(&self) -> ModelResult<()> {
        parse_id("member_id", "mem_", &self.member_id)?;
        MemberPath::parse(&self.path)?;
        if self.participation == StashParticipation::Skipped {
            return Err(invalid("persisted stash members must not be skipped"));
        }
        validate_optional_text("branch_before", &self.branch_before)?;
        validate_optional_object_id("head_before", &self.head_before)?;
        require_non_empty("full_stash_message", &self.full_stash_message)?;
        if let Some(object_id) = &self.native_stash_object_id {
            validate_object_id("native_stash_object_id", object_id)?;
        }
        validate_optional_text("native_stash_display_ref", &self.native_stash_display_ref)?;
        self.dirty_summary.validate()?;
        if let Some(error) = &self.error {
            error.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StashParticipation {
    Stashed,
    Empty,
    Skipped,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StashRestoreState {
    Pending,
    Applied,
    Popped,
    Dropped,
    Noop,
    Missing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StashPushLifecycle {
    Unattempted,
    Saving,
    Saved,
    Empty,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashDirtySummary {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub ignored: bool,
}

impl StashDirtySummary {
    fn validate(&self) -> ModelResult<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
}

impl StashWarning {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("warning.code", &self.code)?;
        require_non_empty("warning.message", &self.message)?;
        if let Some(member_id) = &self.member_id {
            parse_id("warning.member_id", "mem_", member_id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashDrift {
    pub code: String,
    pub message: String,
    pub member_id: String,
}

impl StashDrift {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("drift.code", &self.code)?;
        require_non_empty("drift.message", &self.message)?;
        parse_id("drift.member_id", "mem_", &self.member_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StashErrorDetail {
    pub code: String,
    pub message: String,
}

impl StashErrorDetail {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("error.code", &self.code)?;
        require_non_empty("error.message", &self.message)
    }
}

pub fn read_bundle(root: &Path, stash_id: &str) -> ModelResult<StashBundle> {
    parse_id("stash_id", "stash_", stash_id)?;
    let bundle = StashBundle::from_yaml(&read_to_string(bundle_path(root, stash_id))?)?;
    if bundle.stash_id == stash_id {
        Ok(bundle)
    } else {
        Err(invalid(format!(
            "stash bundle id '{}' does not match requested id '{stash_id}'",
            bundle.stash_id
        )))
    }
}

pub fn write_bundle(root: &Path, bundle: &StashBundle) -> ModelResult<()> {
    write_atomic(&bundle_path(root, &bundle.stash_id), bundle.to_yaml()?)
}

/// Returns all local stash bundle records, newest first by `created_at`.
pub fn list_bundles(root: &Path) -> ModelResult<Vec<StashBundle>> {
    let dir = root.join(STASH_BUNDLE_DIR);
    let mut paths: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(read) => read
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    paths.sort();

    let mut bundles = paths
        .into_iter()
        .map(|path| read_to_string(path).and_then(|yaml| StashBundle::from_yaml(&yaml)))
        .collect::<ModelResult<Vec<_>>>()?;
    bundles.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.stash_id.cmp(&left.stash_id))
    });
    Ok(bundles)
}

pub fn bundle_path(root: &Path, stash_id: &str) -> PathBuf {
    root.join(RUNTIME_DIR)
        .join("stash")
        .join("bundles")
        .join(format!("{stash_id}.yaml"))
}

fn read_to_string(path: PathBuf) -> ModelResult<String> {
    fs::read_to_string(path).map_err(io_error)
}

fn require_schema(actual: &str, expected: &str) -> ModelResult<()> {
    match schema_major(actual) {
        Some(_) if actual == expected => Ok(()),
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

fn validate_optional_text(field: &str, value: &Option<String>) -> ModelResult<()> {
    match value {
        Some(value) => require_non_empty(field, value),
        None => Ok(()),
    }
}

fn validate_optional_object_id(field: &str, value: &Option<String>) -> ModelResult<()> {
    match value {
        Some(value) => validate_object_id(field, value),
        None => Ok(()),
    }
}

fn validate_object_id(field: &str, value: &str) -> ModelResult<()> {
    let valid_len = matches!(value.len(), 40 | 64);
    let valid_hex = value.chars().all(|ch| ch.is_ascii_hexdigit());
    if valid_len && valid_hex {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must be a 40 or 64 character Git object id"
        )))
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

    use crate::artifact::{ArtifactSourceKind, LockArtifact};
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    use crate::workspace_ops::ensure_workspace_exclude;

    use super::*;

    #[test]
    fn pinned_member_state_tuples_round_trip() {
        let mut bundle = sample_bundle();
        bundle.members = vec![
            member(
                "mem_clean",
                StashParticipation::Empty,
                StashPushLifecycle::Empty,
                StashRestoreState::Noop,
            ),
            member(
                "mem_saved",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Pending,
            ),
            member(
                "mem_failed",
                StashParticipation::Stashed,
                StashPushLifecycle::Failed,
                StashRestoreState::Missing,
            ),
            member(
                "mem_unattempted",
                StashParticipation::Stashed,
                StashPushLifecycle::Unattempted,
                StashRestoreState::Missing,
            ),
            member(
                "mem_applied",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Applied,
            ),
            member(
                "mem_popped",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Popped,
            ),
            member(
                "mem_dropped",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Dropped,
            ),
            member(
                "mem_missing",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Missing,
            ),
        ];
        bundle.selected_members = bundle
            .members
            .iter()
            .map(|member| member.member_id.clone())
            .collect();

        let yaml = bundle.to_yaml().unwrap();
        assert_eq!(StashBundle::from_yaml(&yaml).unwrap(), bundle);
    }

    #[test]
    fn failed_push_lifecycle_round_trips_error_detail() {
        let mut bundle = sample_bundle();
        bundle.members[0].push_lifecycle = StashPushLifecycle::Failed;
        bundle.members[0].restore_state = StashRestoreState::Missing;
        bundle.members[0].error = Some(StashErrorDetail {
            code: "git_stash_failed".to_owned(),
            message: "native stash failed".to_owned(),
        });

        let parsed = StashBundle::from_yaml(&bundle.to_yaml().unwrap()).unwrap();
        assert_eq!(
            parsed.members[0].error.as_ref().unwrap().code,
            "git_stash_failed"
        );
    }

    #[test]
    fn rejects_invalid_schema_ids_paths_object_ids_duplicates_and_skipped() {
        let mut unsupported = sample_bundle();
        unsupported.schema = "gwz.stash-bundle/v1".to_owned();
        assert_eq!(
            unsupported.to_yaml().unwrap_err().code,
            ErrorCode::SchemaUnsupported
        );

        let mut invalid_workspace = sample_bundle();
        invalid_workspace.workspace_id = "bad".to_owned();
        assert_eq!(
            invalid_workspace.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut invalid_stash = sample_bundle();
        invalid_stash.stash_id = "bad".to_owned();
        assert_eq!(
            invalid_stash.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut invalid_member = sample_bundle();
        invalid_member.members[0].member_id = "bad".to_owned();
        assert_eq!(
            invalid_member.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut invalid_path = sample_bundle();
        invalid_path.members[0].path = "../escape".to_owned();
        assert_eq!(
            invalid_path.to_yaml().unwrap_err().code,
            ErrorCode::PathEscape
        );

        let mut invalid_oid = sample_bundle();
        invalid_oid.members[0].native_stash_object_id = Some("not-an-oid".to_owned());
        assert_eq!(
            invalid_oid.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut duplicate = sample_bundle();
        duplicate.members.push(duplicate.members[0].clone());
        assert_eq!(
            duplicate.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut duplicate_selected = sample_bundle();
        duplicate_selected
            .selected_members
            .push("mem_app".to_owned());
        assert_eq!(
            duplicate_selected.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut unselected_member = sample_bundle();
        unselected_member.selected_members = vec!["mem_other".to_owned()];
        assert_eq!(
            unselected_member.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let mut skipped = sample_bundle();
        skipped.members[0].participation = StashParticipation::Skipped;
        assert_eq!(
            skipped.to_yaml().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        let bad_enum = sample_bundle()
            .to_yaml()
            .unwrap()
            .replace("push_lifecycle: saved", "push_lifecycle: unknown");
        assert_eq!(
            StashBundle::from_yaml(&bad_enum).unwrap_err().code,
            ErrorCode::ManifestInvalid
        );
    }

    #[test]
    fn bundle_file_io_uses_runtime_registry_path_and_replaces_atomically() {
        let temp = TempDir::new("stash-io");
        let mut first = sample_bundle();
        first.message_suffix = "first".to_owned();
        write_bundle(temp.path(), &first).unwrap();

        let mut second = sample_bundle();
        second.message_suffix = "second".to_owned();
        write_bundle(temp.path(), &second).unwrap();

        assert_eq!(read_bundle(temp.path(), "stash_demo").unwrap(), second);
        assert!(
            temp.path()
                .join(".gwz/stash/bundles/stash_demo.yaml")
                .is_file()
        );
        let leftovers = fs::read_dir(temp.path().join(".gwz/stash/bundles"))
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
    fn list_bundles_returns_newest_first() {
        let temp = TempDir::new("stash-list");
        assert!(list_bundles(temp.path()).unwrap().is_empty());

        let mut older = sample_bundle();
        older.stash_id = "stash_older".to_owned();
        older.created_at = "2026-06-25T01:00:00Z".to_owned();
        let mut newer = sample_bundle();
        newer.stash_id = "stash_newer".to_owned();
        newer.created_at = "2026-06-25T02:00:00Z".to_owned();

        write_bundle(temp.path(), &older).unwrap();
        write_bundle(temp.path(), &newer).unwrap();

        let ids = list_bundles(temp.path())
            .unwrap()
            .into_iter()
            .map(|bundle| bundle.stash_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["stash_newer", "stash_older"]);
    }

    #[test]
    fn stash_runtime_write_does_not_dirty_root_repository_when_boundary_synced() {
        let temp = TempDir::new("stash-root-boundary");
        let backend = Git2Backend::default();
        backend.create_repo(temp.path()).unwrap();
        ensure_workspace_exclude(temp.path(), &empty_lock()).unwrap();

        write_bundle(temp.path(), &sample_bundle()).unwrap();

        let exclude = fs::read_to_string(temp.path().join(".git/info/exclude")).unwrap();
        assert!(exclude.contains("/.gwz/"));
        assert!(
            !backend.status(temp.path()).unwrap().is_dirty,
            "writing .gwz stash registry must not dirty root repo"
        );
    }

    fn sample_bundle() -> StashBundle {
        StashBundle {
            schema: STASH_BUNDLE_SCHEMA.to_owned(),
            workspace_id: "ws_01".to_owned(),
            stash_id: "stash_demo".to_owned(),
            created_at: "2026-06-25T00:00:00Z".to_owned(),
            message_suffix: "demo".to_owned(),
            include_untracked: true,
            include_ignored: false,
            selected_members: vec!["mem_app".to_owned()],
            members: vec![member(
                "mem_app",
                StashParticipation::Stashed,
                StashPushLifecycle::Saved,
                StashRestoreState::Pending,
            )],
            warnings: vec![StashWarning {
                code: "orphan_native_stash".to_owned(),
                message: "native stash has no bundle".to_owned(),
                member_id: Some("mem_app".to_owned()),
            }],
            drift: vec![StashDrift {
                code: "missing_native_stash".to_owned(),
                message: "registry entry has no native stash".to_owned(),
                member_id: "mem_app".to_owned(),
            }],
        }
    }

    fn member(
        member_id: &str,
        participation: StashParticipation,
        push_lifecycle: StashPushLifecycle,
        restore_state: StashRestoreState,
    ) -> StashBundleMember {
        StashBundleMember {
            member_id: member_id.to_owned(),
            path: format!("repos/{member_id}"),
            participation,
            push_lifecycle,
            restore_state,
            branch_before: Some("main".to_owned()),
            head_before: Some("0123456789012345678901234567890123456789".to_owned()),
            full_stash_message: format!("gwz:stash_demo:{member_id}: demo"),
            dirty_summary: StashDirtySummary {
                staged: true,
                unstaged: true,
                untracked: true,
                ignored: false,
            },
            native_stash_object_id: Some("1111111111111111111111111111111111111111".to_owned()),
            native_stash_display_ref: Some("stash@{0}".to_owned()),
            error: None,
        }
    }

    fn empty_lock() -> LockArtifact {
        LockArtifact {
            schema: crate::artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_01".to_owned(),
            manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            members: [(
                "mem_app".to_owned(),
                crate::artifact::ResolvedMemberArtifact {
                    path: "repos/mem_app".to_owned(),
                    source_kind: ArtifactSourceKind::Git,
                    ..crate::artifact::ResolvedMemberArtifact::default()
                },
            )]
            .into(),
        }
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
