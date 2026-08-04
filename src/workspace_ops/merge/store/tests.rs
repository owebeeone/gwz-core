use super::*;
use crate::workspace_ops::merge::{
    MergeParticipantRecord, OperationState, PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::TempDir;

fn temp(name: &str) -> TempDir {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "gwz-merge-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    TempDir { path }
}

fn record(id: &str, state: OperationState) -> MergeOperationRecord {
    let mut record: MergeOperationRecord = serde_yaml::from_str(
            r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_test, operation_id: op_test, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#,
        )
        .unwrap();
    record.merge_id = id.to_owned();
    record.state = state;
    record
}

fn preservation_participant() -> MergeParticipantRecord {
    serde_yaml::from_str(
            r#"{path: app, target_kind: member, target_branch: main, before_commit: '111', source_commit: '222', commit_message: merge, state: aborted, preservation: [{backup_ref: refs/gwz/merge/kept/app/head, backup_commit: '333'}]}"#,
        )
        .unwrap()
}

#[test]
fn write_load_discover_and_unknown_round_trip() {
    let temp = temp("merge-store-roundtrip");
    let store = FileMergeStore;
    let mut expected = record("merge_1", OperationState::Executing);
    expected.publication = Some(PublicationProgress {
        step: PublicationStep::NotStarted,
        candidate_lock_sha256: Some("candidate".to_owned()),
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    store.write_open(&temp.path, &expected).unwrap();
    assert_eq!(store.load(&temp.path, "merge_1").unwrap(), expected);

    let path = open_path(&temp.path, "merge_1");
    let mut raw: Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    raw["publication"].as_mapping_mut().unwrap().insert(
        Value::String("future_publication".to_owned()),
        Value::String("retained".to_owned()),
    );
    fs::write(&path, serde_yaml::to_string(&raw).unwrap()).unwrap();
    expected = store.discover_open(&temp.path).unwrap().unwrap();
    expected.state = OperationState::Halted;
    expected.publication.as_mut().unwrap().candidate_lock_sha256 = None;
    store.write_open(&temp.path, &expected).unwrap();
    let rewritten = fs::read_to_string(path).unwrap();
    assert!(rewritten.contains("future_publication: retained"));
    let rewritten_value: Value = serde_yaml::from_str(&rewritten).unwrap();
    assert!(rewritten_value["publication"]["candidate_lock_sha256"].is_null());
}

#[test]
fn corrupt_open_records_fail_closed() {
    let temp = temp("merge-store-corrupt");
    let directory = temp.path.join(MERGE_DIR);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("merge_bad.yaml"), "not: [valid").unwrap();
    assert_eq!(
        FileMergeStore.discover_open(&temp.path).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );
}

#[test]
fn archive_retention_keeps_preservation_owners() {
    let temp = temp("merge-store-archive");
    let store = FileMergeStore;
    let open = record("merge_open", OperationState::Executing);
    store.write_open(&temp.path, &open).unwrap();
    assert_eq!(
        store.archive(&temp.path, "merge_open").unwrap_err().code,
        ErrorCode::MergeRecoveryRequired
    );
    fs::remove_file(open_path(&temp.path, "merge_open")).unwrap();
    for index in 0..22 {
        let closed = record(&format!("merge_{index:02}"), OperationState::Completed);
        store.write_open(&temp.path, &closed).unwrap();
        store.archive(&temp.path, &closed.merge_id).unwrap();
    }
    let mut kept = record("merge_kept", OperationState::Aborted);
    kept.selected_targets.push("mem_app".to_owned());
    kept.participants
        .insert("mem_app".to_owned(), preservation_participant());
    store.write_open(&temp.path, &kept).unwrap();
    store.archive(&temp.path, &kept.merge_id).unwrap();

    assert_eq!(record_files(&temp.path.join(DONE_DIR)).unwrap().len(), 21);
    assert!(done_path(&temp.path, "merge_kept").is_file());
    assert!(store.discover_open(&temp.path).unwrap().is_none());
    assert_eq!(store.load(&temp.path, "merge_kept").unwrap(), kept);
}

#[test]
fn archive_retry_accepts_destination_only_after_publish() {
    let temp = temp("merge-store-archive-destination-only");
    let store = FileMergeStore;
    let closed = record("merge_closed", OperationState::Aborted);
    store.write_open(&temp.path, &closed).unwrap();
    fs::create_dir_all(temp.path.join(DONE_DIR)).unwrap();
    fs::rename(
        open_path(&temp.path, &closed.merge_id),
        done_path(&temp.path, &closed.merge_id),
    )
    .unwrap();

    store.archive(&temp.path, &closed.merge_id).unwrap();

    assert!(store.discover_open(&temp.path).unwrap().is_none());
    assert_eq!(store.load(&temp.path, &closed.merge_id).unwrap(), closed);
}

#[test]
fn archive_retry_removes_matching_open_copy_after_destination_publish() {
    let temp = temp("merge-store-archive-both-copies");
    let store = FileMergeStore;
    let closed = record("merge_closed", OperationState::Aborted);
    store.write_open(&temp.path, &closed).unwrap();
    fs::create_dir_all(temp.path.join(DONE_DIR)).unwrap();
    fs::copy(
        open_path(&temp.path, &closed.merge_id),
        done_path(&temp.path, &closed.merge_id),
    )
    .unwrap();

    store.archive(&temp.path, &closed.merge_id).unwrap();

    assert!(!open_path(&temp.path, &closed.merge_id).exists());
    assert_eq!(store.load(&temp.path, &closed.merge_id).unwrap(), closed);
}

#[test]
fn archive_retry_rejects_nonterminal_destination_only_record() {
    let temp = temp("merge-store-archive-open-destination");
    let store = FileMergeStore;
    let open = record("merge_open", OperationState::Executing);
    store.write_open(&temp.path, &open).unwrap();
    fs::create_dir_all(temp.path.join(DONE_DIR)).unwrap();
    fs::rename(
        open_path(&temp.path, &open.merge_id),
        done_path(&temp.path, &open.merge_id),
    )
    .unwrap();

    assert_eq!(
        store.archive(&temp.path, &open.merge_id).unwrap_err().code,
        ErrorCode::MergeRecoveryRequired
    );
    assert!(done_path(&temp.path, &open.merge_id).exists());
}

#[test]
fn failed_atomic_publish_removes_its_temporary_file() {
    let temp = temp("merge-store-atomic-fault");
    let target = temp.path.join("record.yaml");
    fs::create_dir(&target).unwrap();
    assert_eq!(
        write_atomic_verified(&target, b"record").unwrap_err().code,
        ErrorCode::IoError
    );
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
    FileMergeStore.gc(&temp.path, None).unwrap();
}
