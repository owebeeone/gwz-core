use std::path::{Path, PathBuf};

use super::{MergeOperationRecord, MergeStore};
use crate::model::ModelResult;
use crate::workspace::WORKSPACE_MANIFEST;

/// An open recovery record found without consulting live workspace metadata.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenMergeRecovery {
    pub root: PathBuf,
    pub record: MergeOperationRecord,
}

/// Search ancestors for merge runtime state before parsing the manifest or lock.
///
/// This keeps recovery reachable when an explicitly merged workspace root has
/// conflicted or temporarily invalid GWZ-owned metadata.
pub(crate) fn discover_open_before_manifest<S: MergeStore>(
    store: &S,
    start: &Path,
) -> ModelResult<Option<OpenMergeRecovery>> {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if current.join(".gwz/merge").try_exists().unwrap_or(true)
            && let Some(record) = store.discover_open(&current)?
        {
            return Ok(Some(OpenMergeRecovery {
                root: current,
                record,
            }));
        }
        // Recovery state belongs to the nearest workspace boundary. Inspect
        // runtime state first so an invalid/conflicted manifest cannot hide an
        // operation, then stop instead of capturing this nested workspace with
        // an enclosing workspace's open merge.
        if current
            .join(WORKSPACE_MANIFEST)
            .try_exists()
            .unwrap_or(true)
        {
            return Ok(None);
        }
        if !current.pop() {
            return Ok(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::workspace_ops::merge::store::FileMergeStore;
    use crate::workspace_ops::tests::TempDir;

    fn temp(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("gwz-merge-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    #[test]
    fn recovery_is_discovered_before_invalid_manifest_is_read() {
        let temp = temp("merge-recovery-discovery");
        let nested = temp.path.join("repos/app/src");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(temp.path.join("gwz.conf")).unwrap();
        fs::write(temp.path.join("gwz.conf/gwz.yml"), "invalid: [").unwrap();
        let yaml = r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_1, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#;
        let directory = temp.path.join(".gwz/merge");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("merge_1.yaml"), yaml).unwrap();

        let found = discover_open_before_manifest(&FileMergeStore, &nested)
            .unwrap()
            .unwrap();
        assert_eq!(found.root, temp.path);
        assert_eq!(found.record.merge_id, "merge_1");
    }

    #[test]
    fn recovery_discovery_stops_at_nearest_nested_workspace_boundary() {
        let temp = temp("merge-recovery-nested-boundary");
        let inner = temp.path.join("repos/inner");
        let start = inner.join("src");
        fs::create_dir_all(&start).unwrap();
        fs::create_dir_all(temp.path.join("gwz.conf")).unwrap();
        fs::write(
            temp.path.join(crate::workspace::WORKSPACE_MANIFEST),
            "schema: gwz.workspace/v0\n",
        )
        .unwrap();
        fs::create_dir_all(inner.join("gwz.conf")).unwrap();
        fs::write(
            inner.join(crate::workspace::WORKSPACE_MANIFEST),
            "schema: gwz.workspace/v0\n",
        )
        .unwrap();

        let yaml = r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_outer, operation_id: op_1, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#;
        let directory = temp.path.join(".gwz/merge");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("merge_outer.yaml"), yaml).unwrap();

        assert!(
            discover_open_before_manifest(&FileMergeStore, &start)
                .unwrap()
                .is_none()
        );
    }
}
