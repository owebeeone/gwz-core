use super::*;

mod preserving_abort_gate;
mod review_remediation;
mod root_retry_safety;

struct FailingPreservationStore {
    fail_at_write: usize,
    writes: Cell<usize>,
    fired: Cell<bool>,
}

impl MergeStore for FailingPreservationStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        let write = self.writes.get() + 1;
        self.writes.set(write);
        if !self.fired.get() && write == self.fail_at_write {
            self.fired.set(true);
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("injected preservation write {write} failure"),
            ));
        }
        FileMergeStore.write_open(root, record)
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        FileMergeStore.archive(root, merge_id)
    }
}

fn invoke_preservation_store(
    backend: &crate::git::Git2Backend,
    store: &FailingPreservationStore,
    root: &Path,
    request: crate::MergeRequest,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request,
        operation_id,
    )
}

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
fn preserve_abort_handles_post_composition_root_work_with_root_bundle_identity() {
    let temp = TempDir::new("merge-preserve-root");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-preserve-root");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before =
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(&backend, temp.path(), "root-feature.txt", "root feature\n");
    let mut start = request(false);
    start.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        start,
        "op_preserve_root_start",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let composition = record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_ref())
        .unwrap()
        .clone();
    let post_composition = commit_file(
        temp.path(),
        "after-composition.txt",
        "keep me\n",
        "post composition work",
        &[git2::Oid::from_str(&composition).unwrap()],
    )
    .unwrap();
    fs::write(temp.path().join("root-untracked.txt"), "keep me too\n").unwrap();

    let mut abort = recovery_request(crate::MergeOp::Abort, Some(record.merge_id));
    abort.preserve = Some(true);
    let aborted = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        abort,
        "op_preserve_root_abort",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    let evidence = aborted
        .preservation
        .as_ref()
        .unwrap()
        .iter()
        .find(|entry| entry.target_id == "@root")
        .unwrap();
    assert_eq!(
        evidence.backup_commit.as_deref(),
        Some(post_composition.as_str())
    );
    assert!(
        evidence
            .backup_ref
            .as_deref()
            .unwrap()
            .ends_with("/root/head")
    );
    let stash_id = evidence.stash_id.clone().unwrap();
    let stash_object_id = evidence.stash_object_id.clone().unwrap();
    let bundle = crate::stash::read_bundle(temp.path(), &stash_id).unwrap();
    assert!(
        bundle
            .members
            .iter()
            .any(|member| member.member_id == "@root" && member.path == ".")
    );
    handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: request_meta(),
            op: crate::StashOp::Apply,
            stash_id: Some(stash_id),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: None,
        },
        "op_preserve_root_restore",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("root-untracked.txt"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["keep me too"]
    );
    handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: {
                let mut meta = request_meta();
                meta.selection = Some(crate::Selection {
                    targets: vec!["@root".to_owned()],
                    ..Default::default()
                });
                meta
            },
            op: crate::StashOp::Drop,
            stash_id: Some(format!("stash_{}", aborted.merge_id.unwrap())),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: None,
        },
        "op_preserve_root_drop",
    )
    .unwrap();
    assert!(
        backend
            .stash_list(temp.path())
            .unwrap()
            .iter()
            .all(|entry| entry.object_id != stash_object_id)
    );
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

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert!(
        record
            .participants
            .values()
            .all(|row| row.preservation.is_empty())
    );
    assert!(
        backend
            .read_ref(
                &lib,
                &format!("refs/gwz/merge/{}/mem_lib/head", record.merge_id),
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
    let mut record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    record.state = OperationState::Preserving;
    record.participants.get_mut("mem_lib").unwrap().preservation =
        vec![crate::workspace_ops::merge::PreservationEvidence {
            backup_ref: Some(backup_ref),
            backup_commit: Some(extra.clone()),
            stash_id: Some(format!("stash_{merge_id}")),
            stash_object_id: Some(stash.object_id.clone()),
        }];
    FileMergeStore.write_open(temp.path(), &record).unwrap();
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

#[test]
fn preserve_abort_failure_windows_never_begin_rollback_and_retry_converges() {
    for fail_at_write in 1..=3 {
        let temp = TempDir::new(&format!("merge-preserve-fault-{fail_at_write}"));
        let backend = crate::git::Git2Backend::new();
        let fixture = init_mixed_merge_workspace(temp.path(), &backend);
        let started = handle_merge(
            &backend,
            temp.path(),
            request(false),
            format!("op_preserve_fault_start_{fail_at_write}"),
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
            "fault-window.txt",
            "preserve me\n",
            "fault window work",
            &[git2::Oid::from_str(&result).unwrap()],
        )
        .unwrap();
        fs::write(lib.join("fault-untracked.txt"), "stash me\n").unwrap();
        let store = FailingPreservationStore {
            fail_at_write,
            writes: Cell::new(0),
            fired: Cell::new(false),
        };
        let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id.clone()));
        abort.preserve = Some(true);

        let error = invoke_preservation_store(
            &backend,
            &store,
            temp.path(),
            abort.clone(),
            "op_preserve_fault",
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
        assert_eq!(
            backend.head(&lib).unwrap().commit.as_deref(),
            Some(extra.as_str()),
            "failure window {fail_at_write} rolled back the successful participant"
        );
        assert_eq!(
            backend.repository_state(&temp.path().join("docs")).unwrap(),
            crate::git::GitRepositoryState::Merge,
            "failure window {fail_at_write} aborted the conflicted participant"
        );
        let backup_ref = format!("refs/gwz/merge/{merge_id}/mem_lib/head");
        assert_eq!(
            backend.read_ref(&lib, &backup_ref).unwrap().is_some(),
            fail_at_write >= 2
        );
        assert_eq!(
            backend
                .stash_list(&lib)
                .unwrap()
                .iter()
                .any(|entry| entry.message.contains(&format!("gwz:stash_{merge_id}:"))),
            fail_at_write >= 3
        );

        let aborted = invoke_preservation_store(
            &backend,
            &store,
            temp.path(),
            abort,
            "op_preserve_fault_retry",
        )
        .unwrap();
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
        assert!(evidence.stash_object_id.is_some());
        let bundle = crate::stash::read_bundle(temp.path(), &format!("stash_{merge_id}")).unwrap();
        assert!(bundle.members.iter().any(|member| {
            member.member_id == "mem_lib"
                && member.native_stash_object_id == evidence.stash_object_id
        }));
    }
}
