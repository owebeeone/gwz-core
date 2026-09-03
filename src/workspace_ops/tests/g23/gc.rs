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
        crate::workspace_ops::merge::read_archived_record(temp.path(), &merge_id)
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
            noop_commit: None,
            reset_commit: None,
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
            noop_commit: None,
            reset_commit: None,
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

fn archived_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/done/{merge_id}.yaml"))
}

/// A real, production-written v1 archive owning one canonical backup ref.
/// `--no-ff` is the shipped writer floor's v1 door, so the record is v1 from
/// creation; the conflicting member leaves it open and `--abort` archives it.
/// The preservation row is injected as the v0 rows above inject theirs.
fn v1_archive_owning_a_backup_ref(label: &str) -> (TempDir, crate::git::Git2Backend, String) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        crate::MergeRequest {
            mode: Some(crate::MergeMode::NoFf),
            ..request(false)
        },
        format!("op_{label}_start"),
    )
    .unwrap();
    let merge_id = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        format!("op_{label}_abort"),
    )
    .unwrap()
    .merge_id
    .unwrap();
    let lib = temp.path().join("lib");
    let name = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    let commit = backend.head(&lib).unwrap().commit.unwrap();
    backend.create_backup_ref(&lib, &name, &commit).unwrap();
    let archive = archived_path(temp.path(), &merge_id);
    let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&archive).unwrap()).unwrap();
    // These rows only stand while --no-ff archives under the v1 envelope.
    assert_eq!(yaml["schema"].as_str(), Some("gwz.merge-operation/v1"));
    yaml["participants"]["mem_lib"]["preservation"] =
        serde_yaml::to_value(vec![crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(name),
            backup_commit: Some(commit),
            stash_id: None,
            stash_object_id: None,
            noop_commit: None,
            reset_commit: None,
        }])
        .unwrap();
    fs::write(&archive, serde_yaml::to_string(&yaml).unwrap()).unwrap();
    (temp, backend, merge_id)
}

/// **The shipped defect, driven end to end through the live `--gc` dispatch.**
/// A `--no-ff` merge archives as v1, and BOTH reads the live path makes of those
/// bytes — the retention decode and the store's last read before unlink — were
/// v0-only, so no v1 archive could be collected. Now the ref goes under the
/// checked rule and the archive with it.
#[test]
fn explicit_gc_collects_a_v1_archive_and_deletes_its_backup_ref() {
    let (temp, backend, merge_id) = v1_archive_owning_a_backup_ref("merge-gc-v1");
    let lib = temp.path().join("lib");
    let name = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id.clone()));
    gc.preserve = None;

    let collected = handle_merge(&backend, temp.path(), gc, "op_gc_v1_collect").unwrap();

    assert_eq!(collected.merge_id.as_deref(), Some(merge_id.as_str()));
    assert_eq!(collected.state, crate::MergeOperationState::Aborted);
    assert!(!collected.open);
    assert_eq!(
        collected.record.as_ref().map(|row| row.source_version),
        Some(crate::MergeRecordVersion::V1)
    );
    assert!(backend.read_ref(&lib, &name).unwrap().is_none());
    assert!(!archived_path(temp.path(), &merge_id).exists());
}

/// **The refusal, kept.** The dispatch widens which envelopes decode, not which
/// archives are collectable: a v1 envelope whose body is not a v1 record still
/// refuses typed before any ref is deleted, and both are retained.
#[test]
fn explicit_gc_refuses_an_unreadable_v1_archive_and_retains_every_ref() {
    let (temp, backend, merge_id) = v1_archive_owning_a_backup_ref("merge-gc-v1-corrupt");
    let lib = temp.path().join("lib");
    let name = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    let archive = archived_path(temp.path(), &merge_id);
    let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&archive).unwrap()).unwrap();
    yaml.as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("participants".to_owned()))
        .unwrap();
    fs::write(&archive, serde_yaml::to_string(&yaml).unwrap()).unwrap();
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id));
    gc.preserve = None;

    let error = handle_merge(&backend, temp.path(), gc, "op_gc_v1_corrupt_collect").unwrap_err();

    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(backend.read_ref(&lib, &name).unwrap().is_some());
    assert!(archive.is_file());
}

/// The shape every COMPLETED `--no-ff` merge has archived since 0.11.0: a v1
/// envelope whose body carries the publication candidate AND the persisted
/// `accepted_workspace` the v1 acceptance model defines.
fn completed_v1_archive(label: &str) -> (TempDir, crate::git::Git2Backend, String) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, label);
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let completed = handle_merge(
        &backend,
        temp.path(),
        crate::MergeRequest {
            mode: Some(crate::MergeMode::NoFf),
            ..request(false)
        },
        format!("op_{label}_start"),
    )
    .unwrap();
    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    let merge_id = completed.merge_id.unwrap();
    let yaml: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(archived_path(temp.path(), &merge_id)).unwrap()).unwrap();
    assert_eq!(yaml["schema"].as_str(), Some("gwz.merge-operation/v1"));
    assert!(yaml["publication"]["candidate"].is_mapping());
    assert!(yaml["accepted_workspace"].is_mapping());
    (temp, backend, merge_id)
}

/// **The order's own case: a COMPLETED `--no-ff` archive, collected end to end
/// through the live `--gc` dispatch.** Only the two v0-only read sites stood in
/// its way; the archive projection needs no change, because a completed
/// `--no-ff` merge DOES persist `accepted_workspace`, so the projection takes
/// its `SupportedPersisted` arm over `source_version: v1`. This row pins that
/// whole answer, so no future reader has to re-derive it from a record dump.
#[test]
fn explicit_gc_collects_a_completed_no_ff_archive() {
    let (temp, backend, merge_id) = completed_v1_archive("merge-gc-v1-done");
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id.clone()));
    gc.preserve = None;

    let collected = handle_merge(&backend, temp.path(), gc, "op_gc_v1_done_collect").unwrap();

    assert_eq!(collected.state, crate::MergeOperationState::Completed);
    assert!(!collected.open);
    let record = collected.record.as_ref().unwrap();
    assert_eq!(record.source_version, crate::MergeRecordVersion::V1);
    assert_eq!(
        record.terminal_outcome,
        Some(crate::MergeTerminalOutcome::Completed)
    );
    assert_eq!(
        record.acceptance.as_ref().map(|value| value.kind),
        Some(crate::MergeAcceptanceKind::SupportedPersisted)
    );
    assert!(!archived_path(temp.path(), &merge_id).exists());
    assert_eq!(
        crate::workspace_ops::merge::read_archived_record(temp.path(), &merge_id)
            .unwrap_err()
            .code,
        ErrorCode::OperationNotFound
    );
}

/// **The id-less sweep, pinned.** `retention::enforce` classifies each archive
/// so the ordinary cap can retire the oldest; under the v0-only reader a v1
/// archive fell to the "unknown archive: retain it" arm and was never
/// classified, so it outlived the cap forever. Twenty v0 fillers fill
/// `ORDINARY_RETENTION` exactly, so the sweep retires exactly one row — and it
/// can only be the v1 archive, which is both the oldest and the first by path.
///
/// The row drives the arm that SHIPS: `validated_future_cleanup`'s `cfg(test)`
/// twin, which classified through `decode_archived` and masked this site, was
/// test-only and is gone. The archive here also carries an UNKNOWN FIELD, the
/// one shape where the two decoders genuinely differ — the shared decoder
/// extracts an unknown-field manifest that `decode_archived` does not — so the
/// row pins that the shipped read classifies it rather than retaining it.
#[test]
fn ordinary_retention_sweeps_a_v1_archive_once_it_can_be_classified() {
    let (temp, backend, merge_id) = completed_v1_archive("merge-gc-v1-sweep");
    let done = temp.path().join(".gwz/merge/done");
    let archive = archived_path(temp.path(), &merge_id);
    let mut yaml: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&archive).unwrap()).unwrap();
    yaml.as_mapping_mut().unwrap().insert(
        serde_yaml::Value::from("unknown_rider"),
        serde_yaml::Value::from("from a newer writer"),
    );
    fs::write(&archive, serde_yaml::to_string(&yaml).unwrap()).unwrap();
    for index in 0..20 {
        let filler_id = format!("merge_z{index:02}");
        let mut filler = yaml.clone();
        let map = filler.as_mapping_mut().unwrap();
        map.insert(
            serde_yaml::Value::from("schema"),
            serde_yaml::Value::from("gwz.merge-operation/v0"),
        );
        map.insert(
            serde_yaml::Value::from("record_schema_version"),
            serde_yaml::Value::from(0),
        );
        map.insert(
            serde_yaml::Value::from("merge_id"),
            serde_yaml::Value::from(filler_id.as_str()),
        );
        map.remove(serde_yaml::Value::from("accepted_workspace"));
        fs::write(
            done.join(format!("{filler_id}.yaml")),
            serde_yaml::to_string(&filler).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(fs::read_dir(&done).unwrap().count(), 21);
    let mut sweep = recovery_request(crate::MergeOp::Gc, None);
    sweep.preserve = None;

    handle_merge(&backend, temp.path(), sweep, "op_gc_v1_sweep").unwrap();

    assert!(
        !archived_path(temp.path(), &merge_id).exists(),
        "the v1 archive must join the ordinary set and be retired by the cap"
    );
    assert_eq!(fs::read_dir(&done).unwrap().count(), 20);
}
