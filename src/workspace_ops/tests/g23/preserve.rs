use super::*;

mod review_remediation;

#[test]
fn preserve_abort_saves_committed_staged_and_untracked_work_before_rollback() {
    let temp = TempDir::new("merge-preserve-abort");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_preserve_start").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "after-merge.txt",
        "committed after merge\n",
        "post-merge work",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    fs::write(lib.join("staged.txt"), "staged after merge\n").unwrap();
    backend.stage_paths(&lib, &["staged.txt"]).unwrap();
    fs::write(lib.join("untracked.txt"), "untracked after merge\n").unwrap();

    let mut request = recovery_request(crate::MergeOp::Abort, started.merge_id);
    request.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), request, "op_preserve_abort").unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(fixture.lib_before.as_str())
    );
    let evidence = aborted
        .preservation
        .as_ref()
        .unwrap()
        .iter()
        .find(|entry| entry.target_id == "mem_lib")
        .unwrap();
    assert_eq!(evidence.backup_commit.as_deref(), Some(extra.as_str()));
    assert_eq!(
        backend
            .read_ref(&lib, evidence.backup_ref.as_deref().unwrap())
            .unwrap()
            .as_deref(),
        Some(extra.as_str())
    );
    assert!(
        backend
            .stash_list(&lib)
            .unwrap()
            .iter()
            .any(|stash| Some(stash.object_id.as_str()) == evidence.stash_object_id.as_deref())
    );
    let bundle =
        crate::stash::read_bundle(temp.path(), evidence.stash_id.as_deref().unwrap()).unwrap();
    assert!(bundle.members.iter().any(|member| {
        member.member_id == "mem_lib"
            && member.native_stash_object_id.as_deref() == evidence.stash_object_id.as_deref()
    }));
}

#[test]
fn preserve_abort_rejects_diverged_successful_member_before_creating_artifacts() {
    let temp = TempDir::new("merge-preserve-diverged");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_preserve_diverged",
    )
    .unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    backend
        .set_branch_target_checked(&lib, "main", &result, &fixture.lib_before)
        .unwrap();
    let divergent = commit_file(
        &lib,
        "diverged.txt",
        "not a descendant\n",
        "diverged",
        &[git2::Oid::from_str(&fixture.lib_before).unwrap()],
    )
    .unwrap();
    assert_ne!(divergent, result);
    let mut abort = recovery_request(crate::MergeOp::Abort, started.merge_id);
    abort.preserve = Some(true);

    let error =
        handle_merge(&backend, temp.path(), abort, "op_preserve_diverged_abort").unwrap_err();

    assert_eq!(error.code, ErrorCode::PreservationEvidenceMismatch);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    let record = open_record(temp.path()).unwrap();
    let record = record.view();
    assert!(
        record
.participants()
            .values()
            .all(|row| row.preservation.is_empty())
    );
    assert!(
        backend
            .read_ref(
                &lib,
                &format!("refs/gwz/merge/{}/mem_lib/head", record
.merge_id()),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn preserve_abort_resumes_from_recorded_ref_and_native_stash_without_duplicates() {
    let temp = TempDir::new("merge-preserve-retry");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_preserve_retry_start",
    )
    .unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "retry-commit.txt",
        "retry\n",
        "retry work",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    fs::write(lib.join("retry-untracked.txt"), "retry stash\n").unwrap();
    let backup_ref = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
    backend
        .create_backup_ref(&lib, &backup_ref, &extra)
        .unwrap();
    let stash = backend
        .stash_for_merge_preservation(&lib, &merge_id, true)
        .unwrap();
    patch_open_record(temp.path(), |value| {
        value["state"] = serde_yaml::to_value(OperationState::Preserving).unwrap();
        // `preservation` is a SHARED field, but `state: Preserving` is a v1
        // journal state, and v1 requires the durable publication handoff that
        // entering preservation writes: `validate_durable_handoff`
        // (model/v1/validate/preservation.rs:28-51) makes
        // `preservation_publication_handoff` REQUIRED for `Preserving`, and
        // without it decode refuses the whole record with
        // PreservationEvidenceMismatch before the abort is even dispatched.
        // `no_candidate` is what production would have written here:
        // `install_preservation_handoff`
        // (v1_lifecycle/transition/reduce/mod.rs:176-187) stores
        // `model_handoff(...)` only if `preservation_handoff_is_compatible`
        // accepts it, and with no `publication` on the record yet that
        // predicate admits exactly `NoCandidate`
        // (model/v1/validate/publication.rs:99-101).
        // Written as literal YAML because `merge::model` is private to
        // `merge`, so the enum cannot be named from this test module.
        value["preservation_publication_handoff"] =
            serde_yaml::from_str("kind: no_candidate").unwrap();
        value["participants"]["mem_lib"]["preservation"] = serde_yaml::to_value(vec![
            crate::workspace_ops::merge::PreservationEvidence {
                backup_ref: Some(backup_ref),
                backup_commit: Some(extra.clone()),
                stash_id: Some(format!("stash_{merge_id}")),
                stash_object_id: Some(stash.object_id.clone()),
                noop_commit: None,
                reset_commit: None,
            },
        ])
        .unwrap();
    });
    // The durable evidence above is only half of `mem_lib`'s stash step. A
    // member's owner step writes its `PreservationEvidence` and THEN its bundle
    // entry (`PreservationStashPhaseV1::WriteBundle`), so evidence without a
    // bundle is a state no interrupted run can leave behind, and the entry
    // preflight says so: `v1_bundle_cursor_is_exact`
    // (preserve/checked_bundle.rs:48) derives the expected bundle from every
    // owner whose evidence carries a `stash_object_id`, finds nothing on disk,
    // and `cursor.rs:185` refuses with "preservation bundle does not match the
    // exact durable cursor prefix". Running the production writer completes the
    // interrupted step exactly as production would have.
    crate::workspace_ops::merge::v1_write_preservation_bundle_for_test(
        &backend,
        temp.path(),
        "mem_lib",
    )
    .unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id));
    abort.preserve = Some(true);

    let aborted = handle_merge(&backend, temp.path(), abort, "op_preserve_retry_abort").unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(fixture.lib_before.as_str())
    );
    assert_eq!(
        backend
            .stash_list(&lib)
            .unwrap()
            .iter()
            .filter(|entry| entry.object_id == stash.object_id)
            .count(),
        1
    );
    assert_eq!(
        aborted
            .preservation
            .as_ref()
            .unwrap()
            .iter()
            .filter(|entry| entry.target_id == "mem_lib")
            .count(),
        1
    );
}
