use super::*;

#[test]
fn status_by_id_reads_a_closed_merge_without_reopening_it() {
    let temp = TempDir::new("merge-archived-status");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-archived-status");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let completed = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_archived_status_start",
    )
    .unwrap();
    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    let merge_id = completed.merge_id.clone().unwrap();
    let mut status = recovery_request(crate::MergeOp::Status, Some(merge_id.clone()));

    let archived = handle_merge(
        &backend,
        temp.path(),
        status.clone(),
        "op_archived_status_read",
    )
    .unwrap();

    assert_eq!(archived.merge_id.as_deref(), Some(merge_id.as_str()));
    assert_eq!(archived.state, crate::MergeOperationState::Completed);
    assert!(!archived.open);
    status.merge_id = Some("merge_missing".to_owned());
    assert_eq!(
        handle_merge(&backend, temp.path(), status, "op_archived_status_missing",)
            .unwrap_err()
            .code,
        ErrorCode::OperationNotFound
    );
}

#[test]
fn explicit_gc_checked_deletes_only_backup_refs_and_archive_record() {
    let temp = TempDir::new("merge-preservation-gc");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_gc_preserve_start",
    )
    .unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    commit_file(
        &lib,
        "gc-commit.txt",
        "keep through ref\n",
        "gc preservation work",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    fs::write(lib.join("gc-untracked.txt"), "keep through stash\n").unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, started.merge_id);
    abort.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), abort, "op_gc_preserve_abort").unwrap();
    let merge_id = aborted.merge_id.clone().unwrap();
    let evidence = aborted
        .preservation
        .as_ref()
        .unwrap()
        .iter()
        .find(|entry| entry.target_id == "mem_lib")
        .unwrap()
        .clone();
    let stash_id = evidence.stash_id.clone().unwrap();
    let stash_object_id = evidence.stash_object_id.clone().unwrap();
    assert!(crate::stash::bundle_path(temp.path(), &stash_id).is_file());
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id.clone()));
    gc.preserve = None;

    let collected = handle_merge(&backend, temp.path(), gc, "op_gc_preserve_collect").unwrap();

    assert_eq!(collected.merge_id.as_deref(), Some(merge_id.as_str()));
    assert!(
        backend
            .read_ref(&lib, evidence.backup_ref.as_deref().unwrap())
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .stash_list(&lib)
            .unwrap()
            .iter()
            .any(|entry| entry.object_id == stash_object_id)
    );
    assert!(crate::stash::bundle_path(temp.path(), &stash_id).is_file());
    assert_eq!(
        FileMergeStore
            .load(temp.path(), &merge_id)
            .unwrap_err()
            .code,
        ErrorCode::OperationNotFound
    );
    assert!(
        collected
            .preservation
            .as_ref()
            .unwrap()
            .iter()
            .all(|evidence| evidence.backup_ref.is_none() && evidence.backup_commit.is_none())
    );
}

#[test]
fn explicit_gc_rejects_cross_merge_ref_ownership_before_deleting_any_ref() {
    let temp = TempDir::new("merge-preservation-gc-owner");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_gc_owner_start").unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "gc-owner.txt",
        "owner\n",
        "gc owner",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, started.merge_id);
    abort.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), abort, "op_gc_owner_abort").unwrap();
    let merge_id = aborted.merge_id.unwrap();
    let original = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    let foreign = "refs/gwz/merge/merge_foreign/mem_lib/head";
    backend.create_backup_ref(&lib, foreign, &extra).unwrap();
    let app = temp.path().join("app");
    let app_commit = backend.head(&app).unwrap().commit.unwrap();
    let earlier = format!("refs/gwz/merge/{merge_id}/mem_app/head");
    backend
        .create_backup_ref(&app, &earlier, &app_commit)
        .unwrap();
    let archive = temp.path().join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&archive).unwrap()).unwrap();
    yaml["participants"]["mem_app"]["preservation"] =
        serde_yaml::to_value(vec![crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(earlier.clone()),
            backup_commit: Some(app_commit.clone()),
            stash_id: None,
            stash_object_id: None,
        }])
        .unwrap();
    yaml["participants"]["mem_lib"]["preservation"][0]["backup_ref"] =
        serde_yaml::Value::String(foreign.to_owned());
    fs::write(&archive, serde_yaml::to_string(&yaml).unwrap()).unwrap();
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id.clone()));
    gc.preserve = None;

    let error = handle_merge(&backend, temp.path(), gc, "op_gc_owner_collect").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
    assert_eq!(
        backend.read_ref(&lib, &original).unwrap().as_deref(),
        Some(extra.as_str())
    );
    assert_eq!(
        backend.read_ref(&lib, foreign).unwrap().as_deref(),
        Some(extra.as_str())
    );
    assert_eq!(
        backend.read_ref(&app, &earlier).unwrap().as_deref(),
        Some(app_commit.as_str())
    );
    assert!(archive.is_file());
}

#[test]
fn explicit_gc_rejects_malformed_later_target_before_deleting_earlier_ref() {
    assert_gc_rejects_later_target_before_deleting_earlier_ref("not-an-oid");
}

#[test]
fn explicit_gc_rejects_sha256_length_target_before_deleting_earlier_ref() {
    assert_gc_rejects_later_target_before_deleting_earlier_ref(&"a".repeat(64));
}

fn assert_gc_rejects_later_target_before_deleting_earlier_ref(invalid_commit: &str) {
    let temp = TempDir::new("merge-preservation-gc-malformed-target");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_gc_malformed_start",
    )
    .unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "gc-malformed.txt",
        "owner\n",
        "gc malformed target",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, started.merge_id);
    abort.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), abort, "op_gc_malformed_abort").unwrap();
    let merge_id = aborted.merge_id.unwrap();
    let lib_ref = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    backend
        .delete_backup_ref_checked(&lib, &lib_ref, &extra)
        .unwrap();

    let app = temp.path().join("app");
    let app_commit = backend.head(&app).unwrap().commit.unwrap();
    let app_ref = format!("refs/gwz/merge/{merge_id}/mem_app/head");
    backend
        .create_backup_ref(&app, &app_ref, &app_commit)
        .unwrap();
    let archive = temp.path().join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&archive).unwrap()).unwrap();
    yaml["participants"]["mem_app"]["preservation"] =
        serde_yaml::to_value(vec![crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(app_ref.clone()),
            backup_commit: Some(app_commit.clone()),
            stash_id: None,
            stash_object_id: None,
        }])
        .unwrap();
    yaml["participants"]["mem_lib"]["preservation"][0]["backup_commit"] =
        serde_yaml::Value::String(invalid_commit.to_owned());
    fs::write(&archive, serde_yaml::to_string(&yaml).unwrap()).unwrap();
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id));
    gc.preserve = None;

    let error = handle_merge(&backend, temp.path(), gc, "op_gc_malformed_collect").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
    assert_eq!(
        backend.read_ref(&app, &app_ref).unwrap().as_deref(),
        Some(app_commit.as_str())
    );
    assert!(archive.is_file());
}
