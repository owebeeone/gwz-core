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

fn write_raw_open(temp: &TempDir, merge_id: &str, yaml: &str) -> PathBuf {
    let path = open_path(&temp.path, merge_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, yaml).unwrap();
    path
}

fn write_raw_archived(temp: &TempDir, merge_id: &str, yaml: &str) -> PathBuf {
    let path = done_path(&temp.path, merge_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, yaml).unwrap();
    path
}

fn assert_record_context(
    error: &ModelError,
    merge_id: &str,
    schema: &str,
    version: i64,
    required_wave: Option<crate::MergeRecordRequiredWave>,
) {
    let context = error.record_context.as_ref().unwrap();
    assert_eq!(context.merge_id, merge_id);
    assert_eq!(context.schema.as_deref(), Some(schema));
    assert_eq!(context.record_schema_version, Some(version));
    assert_eq!(context.required_wave, required_wave);
    assert_eq!(context.legacy_mode, None);
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
fn installed_but_disabled_v1_reports_a1_compatibility_context() {
    let temp = temp("merge-store-v1-disabled");
    write_raw_open(
        &temp,
        "merge_v1",
        "schema: gwz.merge-operation/v1\nrecord_schema_version: 1\n",
    );

    let error = FileMergeStore.discover_open(&temp.path).unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    assert_eq!(
        error.message,
        "merge record 'merge_v1' uses schema 'gwz.merge-operation/v1' version 1, which requires A1 (v1 integration/acceptance/no-ff); use a compatible newer GWZ"
    );
    assert_record_context(
        &error,
        "merge_v1",
        "gwz.merge-operation/v1",
        1,
        Some(crate::MergeRecordRequiredWave::A1),
    );
}

#[test]
fn allocated_archived_v2_reports_a2_compatibility_context() {
    let temp = temp("merge-store-v2-archived");
    write_raw_archived(
        &temp,
        "merge_v2",
        "schema: gwz.merge-operation/v2\nrecord_schema_version: 2\n",
    );

    let error = FileMergeStore
        .load_archived(&temp.path, "merge_v2")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    assert_record_context(
        &error,
        "merge_v2",
        "gwz.merge-operation/v2",
        2,
        Some(crate::MergeRecordRequiredWave::A2),
    );
}

#[test]
fn unknown_schema_reports_compatibility_context_without_a_wave() {
    let temp = temp("merge-store-unknown-schema");
    write_raw_open(
        &temp,
        "merge_future",
        "schema: example.merge-operation/future\nrecord_schema_version: 42\n",
    );

    let error = FileMergeStore.discover_open(&temp.path).unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    assert_record_context(
        &error,
        "merge_future",
        "example.merge-operation/future",
        42,
        None,
    );
}

#[test]
fn recognized_schema_version_mismatch_is_unreadable_with_header_context() {
    let temp = temp("merge-store-version-mismatch");
    write_raw_open(
        &temp,
        "merge_mismatch",
        "schema: gwz.merge-operation/v1\nrecord_schema_version: 2\n",
    );

    let error = FileMergeStore.discover_open(&temp.path).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
    assert_record_context(&error, "merge_mismatch", "gwz.merge-operation/v1", 2, None);
}

#[test]
fn malformed_envelopes_are_unreadable_without_invented_header_context() {
    for (name, yaml) in [
        (
            "anchor",
            "schema: gwz.merge-operation/v0\nrecord_schema_version: &version 0\n",
        ),
        (
            "duplicate",
            "schema: gwz.merge-operation/v0\nschema: gwz.merge-operation/v0\nrecord_schema_version: 0\n",
        ),
    ] {
        let temp = temp(&format!("merge-store-malformed-{name}"));
        write_raw_open(&temp, &format!("merge_{name}"), yaml);

        let error = FileMergeStore.discover_open(&temp.path).unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
        assert_eq!(error.record_context, None);
    }
}

#[test]
fn multiple_open_records_are_validated_before_the_multiple_record_error() {
    let temp = temp("merge-store-multiple-with-unsupported");
    write_raw_open(
        &temp,
        "merge_a",
        &serde_yaml::to_string(&record("merge_a", OperationState::Executing)).unwrap(),
    );
    write_raw_open(
        &temp,
        "merge_z",
        "schema: gwz.merge-operation/v1\nrecord_schema_version: 1\n",
    );

    let error = FileMergeStore.discover_open(&temp.path).unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    assert_record_context(
        &error,
        "merge_z",
        "gwz.merge-operation/v1",
        1,
        Some(crate::MergeRecordRequiredWave::A1),
    );
}

#[test]
fn targeted_gc_retains_an_unreadable_archived_record() {
    let temp = temp("merge-store-gc-unreadable-archive");
    let path = write_raw_archived(&temp, "merge_bad", "not: [valid");

    let error = FileMergeStore
        .gc(&temp.path, Some("merge_bad"))
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(path.is_file());
}

#[test]
fn archived_post_decode_contradictions_use_the_archive_error_contract() {
    let temp = temp("merge-store-archive-post-decode");
    let mut invalid_id = record("../invalid", OperationState::Completed);
    invalid_id.writer_version = "test-invalid-id".to_owned();
    let path = write_raw_archived(
        &temp,
        "merge_expected",
        &serde_yaml::to_string(&invalid_id).unwrap(),
    );

    let error = FileMergeStore
        .load_archived(&temp.path, "merge_expected")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert_record_context(&error, "merge_expected", "gwz.merge-operation/v0", 0, None);
    assert!(path.is_file());
}

#[test]
fn archived_non_file_record_path_is_unreadable_and_retained() {
    let temp = temp("merge-store-archive-non-file");
    let path = done_path(&temp.path, "merge_directory");
    fs::create_dir_all(&path).unwrap();

    let error = FileMergeStore
        .load_archived(&temp.path, "merge_directory")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert_eq!(error.record_context, None);
    assert!(path.is_dir());

    FileMergeStore.gc(&temp.path, None).unwrap();
    assert!(path.is_dir());
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

    let error = store.archive(&temp.path, &open.merge_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert_record_context(&error, "merge_open", "gwz.merge-operation/v0", 0, None);
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

#[test]
fn retention_treats_stash_only_v0_and_no_ref_v1_as_ordinary() {
    let temp = temp("merge-store-r3-retention-ordinary");
    let (v1_bytes, v1_id) = crate::workspace_ops::merge::record_wire::archived_fixture_for_test(
        crate::workspace_ops::merge::model::v1::RecordVersion::V1,
    );
    write_raw_archived(&temp, v1_id, std::str::from_utf8(&v1_bytes).unwrap());

    let mut stash_only = record("merge_stash_only", OperationState::Aborted);
    let mut participant = preservation_participant();
    participant.preservation[0].backup_ref = None;
    participant.preservation[0].backup_commit = None;
    participant.preservation[0].stash_id = Some("stash_merge_stash_only".to_owned());
    participant.preservation[0].stash_object_id = Some("d".repeat(40));
    stash_only.selected_targets.push("mem_app".to_owned());
    stash_only
        .participants
        .insert("mem_app".to_owned(), participant);
    write_raw_archived(
        &temp,
        &stash_only.merge_id,
        &serde_yaml::to_string(&stash_only).unwrap(),
    );

    for index in 0..20 {
        let ordinary = record(&format!("merge_z{index:02}"), OperationState::Completed);
        write_raw_archived(
            &temp,
            &ordinary.merge_id,
            &serde_yaml::to_string(&ordinary).unwrap(),
        );
    }
    FileMergeStore.gc(&temp.path, None).unwrap();

    assert_eq!(record_files(&temp.path.join(DONE_DIR)).unwrap().len(), 20);
    assert!(!done_path(&temp.path, v1_id).exists());
    assert!(!done_path(&temp.path, &stash_only.merge_id).exists());
}

#[test]
fn retention_exempts_a_valid_v1_archive_that_owns_a_backup_ref() {
    let temp = temp("merge-store-r3-retention-owned");
    let (bytes, merge_id) = crate::workspace_ops::merge::record_wire::archived_fixture_for_test(
        crate::workspace_ops::merge::model::v1::RecordVersion::V1,
    );
    let mut raw: Value = serde_yaml::from_slice(&bytes).unwrap();
    raw["participants"]["mem_a"]["preservation"] =
        serde_yaml::to_value(vec![crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(format!("refs/gwz/merge/{merge_id}/mem_a/head")),
            backup_commit: Some("d".repeat(40)),
            stash_id: None,
            stash_object_id: None,
        }])
        .unwrap();
    write_raw_archived(&temp, merge_id, &serde_yaml::to_string(&raw).unwrap());
    for index in 0..21 {
        let ordinary = record(&format!("merge_z{index:02}"), OperationState::Completed);
        write_raw_archived(
            &temp,
            &ordinary.merge_id,
            &serde_yaml::to_string(&ordinary).unwrap(),
        );
    }

    FileMergeStore.gc(&temp.path, None).unwrap();

    assert!(done_path(&temp.path, merge_id).is_file());
    assert_eq!(record_files(&temp.path.join(DONE_DIR)).unwrap().len(), 21);
}
