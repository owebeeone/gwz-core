use std::fs;

use super::*;
use crate::git::Git2Backend;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::model::v1::{RecordVersion, test_record};
use crate::workspace_ops::merge::record_wire::archived_fixture_for_test;
use crate::workspace_ops::tests::TempDir;

#[test]
fn terminal_archive_restarts_from_source_both_and_destination_only() {
    let backend = Git2Backend::new();
    let store = CheckedV1Store::default();
    let context = context();
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);

    let source_only = TempDir::new("merge-v1-archive-source-only");
    write_open(&source_only, merge_id, &bytes);
    let result = archive_terminal(&backend, &store, &source_only.path, merge_id, &context).unwrap();
    assert_eq!(result.destination_bytes(), bytes);
    assert!(!open_path(&source_only, merge_id).exists());
    assert_eq!(fs::read(done_path(&source_only, merge_id)).unwrap(), bytes);

    let both = TempDir::new("merge-v1-archive-both");
    write_open(&both, merge_id, &bytes);
    write_done(&both, merge_id, &bytes);
    let result = archive_terminal(&backend, &store, &both.path, merge_id, &context).unwrap();
    assert_eq!(result.destination_bytes(), bytes);
    assert!(!open_path(&both, merge_id).exists());

    for version in [RecordVersion::V0, RecordVersion::V1] {
        let (bytes, merge_id) = archived_fixture_for_test(version);
        let destination_only = TempDir::new(&format!("merge-archive-destination-{version:?}"));
        write_done(&destination_only, merge_id, &bytes);
        let result =
            archive_terminal(&backend, &store, &destination_only.path, merge_id, &context).unwrap();
        assert_eq!(result.source_version(), version);
        assert_eq!(result.destination_bytes(), bytes);
    }
}

#[test]
fn archive_rejects_mismatch_malformed_and_nonterminal_without_deletion() {
    let backend = Git2Backend::new();
    let store = CheckedV1Store::default();
    let context = context();
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);

    let mismatch = TempDir::new("merge-v1-archive-mismatch");
    write_open(&mismatch, merge_id, &bytes);
    let mut different = bytes.clone();
    different.extend_from_slice(b"# same model, different authority bytes\n");
    write_done(&mismatch, merge_id, &different);
    let error = expect_error(archive_terminal(
        &backend,
        &store,
        &mismatch.path,
        merge_id,
        &context,
    ));
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(open_path(&mismatch, merge_id).is_file());
    assert_eq!(fs::read(done_path(&mismatch, merge_id)).unwrap(), different);

    let malformed = TempDir::new("merge-v1-archive-malformed");
    write_open(&malformed, merge_id, &bytes);
    write_done(&malformed, merge_id, b"not: [valid");
    let error = expect_error(archive_terminal(
        &backend,
        &store,
        &malformed.path,
        merge_id,
        &context,
    ));
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(open_path(&malformed, merge_id).is_file());
    assert!(done_path(&malformed, merge_id).is_file());

    let nonterminal = TempDir::new("merge-v1-archive-nonterminal");
    let model = test_record();
    write_open(
        &nonterminal,
        &model.merge_id,
        serde_yaml::to_string(&model).unwrap().as_bytes(),
    );
    let error = expect_error(archive_terminal(
        &backend,
        &store,
        &nonterminal.path,
        &model.merge_id,
        &context,
    ));
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(open_path(&nonterminal, &model.merge_id).is_file());
}

#[cfg(unix)]
#[test]
fn archive_rejects_symlinked_destination_parent_and_leaf() {
    use std::os::unix::fs::symlink;

    let backend = Git2Backend::new();
    let store = CheckedV1Store::default();
    let context = context();
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);

    let parent = TempDir::new("merge-v1-archive-symlink-parent");
    let outside = TempDir::new("merge-v1-archive-symlink-outside");
    write_open(&parent, merge_id, &bytes);
    fs::create_dir_all(parent.path.join(".gwz/merge")).unwrap();
    symlink(&outside.path, parent.path.join(".gwz/merge/done")).unwrap();
    let error = expect_error(archive_terminal(
        &backend,
        &store,
        &parent.path,
        merge_id,
        &context,
    ));
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(open_path(&parent, merge_id).is_file());
    assert!(!outside.path.join(format!("{merge_id}.yaml")).exists());

    let leaf = TempDir::new("merge-v1-archive-symlink-leaf");
    let outside_file = outside.path.join("archive.yaml");
    fs::write(&outside_file, &bytes).unwrap();
    fs::create_dir_all(leaf.path.join(".gwz/merge/done")).unwrap();
    symlink(&outside_file, done_path(&leaf, merge_id)).unwrap();
    let error = expect_error(archive_terminal(
        &backend, &store, &leaf.path, merge_id, &context,
    ));
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert_eq!(fs::read(outside_file).unwrap(), bytes);
}

fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_archive".into(),
        request_id: "req_archive".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

fn expect_error<T>(result: ModelResult<T>) -> ModelError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("operation unexpectedly succeeded"),
    }
}

fn open_path(root: &TempDir, merge_id: &str) -> std::path::PathBuf {
    root.path
        .join(".gwz/merge")
        .join(format!("{merge_id}.yaml"))
}

fn done_path(root: &TempDir, merge_id: &str) -> std::path::PathBuf {
    root.path
        .join(".gwz/merge/done")
        .join(format!("{merge_id}.yaml"))
}

fn write_open(root: &TempDir, merge_id: &str, bytes: &[u8]) {
    let path = open_path(root, merge_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn write_done(root: &TempDir, merge_id: &str, bytes: &[u8]) {
    let path = done_path(root, merge_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
