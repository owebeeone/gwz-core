use crate::git::Git2Backend;
use crate::workspace_ops::tests::TempDir;
use std::cell::RefCell;
use std::rc::Rc;

use super::*;

pub(super) fn file_snapshot(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

pub(super) fn remove_loose_object(repo: &Path, oid: &str) {
    let object = git2::Oid::from_str(oid).unwrap().to_string();
    let path = repo
        .join(".git/objects")
        .join(&object[..2])
        .join(&object[2..]);
    assert!(
        path.is_file(),
        "test object must be loose: {}",
        path.display()
    );
    std::fs::remove_file(path).unwrap();
}

fn assert_tree_unavailable(repo: &Path, oid: &str) {
    let repository = git2::Repository::open(repo).unwrap();
    assert!(
        repository
            .find_tree(git2::Oid::from_str(oid).unwrap())
            .is_err()
    );
}

#[test]
fn checked_resolution_invalid_evidence_rejects_without_mutation() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingTree,
        MalformedTree,
        InvalidAuthor,
        InvalidCommitter,
        InvalidTimezone,
    }

    for case in [
        Case::MissingTree,
        Case::MalformedTree,
        Case::InvalidAuthor,
        Case::InvalidCommitter,
        Case::InvalidTimezone,
    ] {
        let root = TempDir::new(&format!("merge-resolution-evidence-{case:?}"));
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
        let app = root.path().join("app");
        let conflict = backend
            .prepare_merge_upstream_checked(
                &app,
                &plan.participants[0].target_branch,
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                None,
            )
            .unwrap();
        backend
            .execute_prepared_merge_upstream_checked(
                &app,
                &plan.participants[0].target_branch,
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                &plan.participants[0].commit_message,
                &conflict,
            )
            .unwrap();
        std::fs::write(app.join("README.md"), "frozen resolution evidence\n").unwrap();
        backend.stage_paths(&app, &["README.md"]).unwrap();
        let mut prepared = backend
            .prepare_merge_resolution_checked(
                &app,
                &plan.participants[0].target_branch,
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                attributed_context().attribution.as_ref(),
            )
            .unwrap();
        backend
            .validate_prepared_merge_resolution_state(
                &app,
                &plan.participants[0].target_branch,
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                &prepared,
            )
            .unwrap();
        let recorded_tree = prepared.tree_oid.clone();

        match case {
            Case::MissingTree => remove_loose_object(&app, &recorded_tree),
            Case::MalformedTree => prepared.tree_oid = "not-an-object-id".to_owned(),
            Case::InvalidAuthor => prepared.author.name = "<invalid>".to_owned(),
            Case::InvalidCommitter => {
                prepared.committer.email = "invalid\n@example.invalid".to_owned();
            }
            Case::InvalidTimezone => prepared.committer.timezone_offset_minutes = 1_441,
        }

        let repository_before = file_snapshot(&app);
        let head_before = backend.head(&app).unwrap();
        let native_before = backend.merge_state(&app).unwrap();
        Git2Backend::reset_preparation_call_count();

        assert_eq!(
            backend
                .validate_prepared_merge_resolution_state(
                    &app,
                    &plan.participants[0].target_branch,
                    &plan.participants[0].before_commit,
                    &plan.participants[0].source_commit,
                    &prepared,
                )
                .unwrap_err()
                .code,
            ErrorCode::MergeRecoveryRequired,
            "case={case:?}"
        );
        assert_eq!(
            backend
                .commit_prepared_merge_resolution_checked(
                    &app,
                    &plan.participants[0].target_branch,
                    &plan.participants[0].before_commit,
                    &plan.participants[0].source_commit,
                    &plan.participants[0].commit_message,
                    &prepared,
                )
                .unwrap_err()
                .code,
            ErrorCode::MergeRecoveryRequired,
            "case={case:?}"
        );

        assert_eq!(Git2Backend::preparation_call_count(), 0, "case={case:?}");
        assert_eq!(backend.head(&app).unwrap(), head_before, "case={case:?}");
        assert_eq!(
            backend.merge_state(&app).unwrap(),
            native_before,
            "case={case:?}"
        );
        assert_eq!(file_snapshot(&app), repository_before, "case={case:?}");
        if matches!(case, Case::MissingTree) {
            assert_tree_unavailable(&app, &recorded_tree);
        }
    }
}

#[test]
fn pending_resolution_missing_tree_race_rejects_without_mutation() {
    let root = TempDir::new("merge-pending-resolution-missing-tree-race");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
    let app = root.path().join("app");
    let conflict = backend
        .prepare_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            None,
        )
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &plan.participants[0].commit_message,
            &conflict,
        )
        .unwrap();
    std::fs::write(app.join("README.md"), "frozen resolution race\n").unwrap();
    backend.stage_paths(&app, &["README.md"]).unwrap();
    let frozen_commit = backend
        .prepare_merge_resolution_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            attributed_context().attribution.as_ref(),
        )
        .unwrap();

    let store = Rc::new(MemoryStore::default());
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::AwaitingResolution;
    let participant = record.participants.get_mut("mem_app").unwrap();
    participant.state = ParticipantState::Conflicted;
    participant.expected_merge_head = Some(plan.participants[0].source_commit.clone());
    participant.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::ResolveConflict,
        target_branch: plan.participants[0].target_branch.clone(),
        before_commit: plan.participants[0].before_commit.clone(),
        source_commit: plan.participants[0].source_commit.clone(),
        commit_message: plan.participants[0].commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: pending_commit_spec(&GitPreparedMerge::Commit(frozen_commit.clone())),
        extensions: BTreeMap::new(),
    });
    let frozen_pending = participant.pending_action.clone().unwrap();
    store.write_open(root.path(), &record).unwrap();

    let head_before = backend.head(&app).unwrap();
    let native_before = backend.merge_state(&app).unwrap();
    let lock_before = std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap();
    let manifest_before =
        std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let boundary = Rc::new(RefCell::new(
        None::<(
            BTreeMap<std::path::PathBuf, Vec<u8>>,
            Vec<MergeOperationRecord>,
            usize,
        )>,
    ));
    let hook_boundary = Rc::clone(&boundary);
    let hook_store = Rc::clone(&store);
    let hook_app = app.clone();
    let recorded_tree = frozen_commit.tree_oid.clone();
    let hook_tree = recorded_tree.clone();
    Git2Backend::before_next_prepared_execution(move || {
        remove_loose_object(&hook_app, &hook_tree);
        assert_tree_unavailable(&hook_app, &hook_tree);
        *hook_boundary.borrow_mut() = Some((
            file_snapshot(&hook_app),
            hook_store.records.lock().unwrap().clone(),
            *hook_store.writes.lock().unwrap(),
        ));
    });
    Git2Backend::reset_preparation_call_count();

    let error = resume(&backend, store.as_ref(), root.path()).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(Git2Backend::preparation_call_count(), 0);
    assert_tree_unavailable(&app, &recorded_tree);
    assert_eq!(backend.head(&app).unwrap(), head_before);
    assert_eq!(backend.merge_state(&app).unwrap(), native_before);
    let boundary = boundary.borrow();
    let (repository_at_boundary, records_at_boundary, writes_at_boundary) =
        boundary.as_ref().expect("execution hook must run");
    assert_eq!(&file_snapshot(&app), repository_at_boundary);
    assert_eq!(
        store.records.lock().unwrap().as_slice(),
        records_at_boundary
    );
    assert_eq!(*store.writes.lock().unwrap(), *writes_at_boundary);
    assert!(store.events.lock().unwrap().is_empty());
    assert!(records_at_boundary.iter().all(|record| {
        let participant = &record.participants["mem_app"];
        participant.pending_action.as_ref() == Some(&frozen_pending)
            && participant.state == ParticipantState::Conflicted
            && participant.error.is_none()
    }));
    assert_eq!(
        std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        manifest_before
    );
}

#[test]
fn pending_resolution_same_commit_branch_switch_race_rejects_without_mutation() {
    let root = TempDir::new("merge-pending-resolution-branch-switch-race");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
    let app = root.path().join("app");
    let conflict = backend
        .prepare_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            None,
        )
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &plan.participants[0].commit_message,
            &conflict,
        )
        .unwrap();
    std::fs::write(app.join("README.md"), "frozen branch resolution\n").unwrap();
    backend.stage_paths(&app, &["README.md"]).unwrap();
    let frozen_commit = backend
        .prepare_merge_resolution_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            attributed_context().attribution.as_ref(),
        )
        .unwrap();
    let repository = git2::Repository::open(&app).unwrap();
    repository
        .reference(
            "refs/heads/other",
            git2::Oid::from_str(&plan.participants[0].before_commit).unwrap(),
            true,
            "same-commit branch-switch test",
        )
        .unwrap();
    drop(repository);

    let repository_before_wrong_prepare = file_snapshot(&app);
    assert_eq!(
        backend
            .prepare_merge_resolution_checked(
                &app,
                "other",
                &plan.participants[0].before_commit,
                &plan.participants[0].source_commit,
                None,
            )
            .unwrap_err()
            .code,
        ErrorCode::MergeDrift
    );
    assert_eq!(file_snapshot(&app), repository_before_wrong_prepare);

    let store = Rc::new(MemoryStore::default());
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::AwaitingResolution;
    let participant = record.participants.get_mut("mem_app").unwrap();
    participant.state = ParticipantState::Conflicted;
    participant.expected_merge_head = Some(plan.participants[0].source_commit.clone());
    participant.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::ResolveConflict,
        target_branch: plan.participants[0].target_branch.clone(),
        before_commit: plan.participants[0].before_commit.clone(),
        source_commit: plan.participants[0].source_commit.clone(),
        commit_message: plan.participants[0].commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: pending_commit_spec(&GitPreparedMerge::Commit(frozen_commit)),
        extensions: BTreeMap::new(),
    });
    let frozen_pending = participant.pending_action.clone().unwrap();
    store.write_open(root.path(), &record).unwrap();

    let native_before = backend.merge_state(&app).unwrap();
    let lock_before = std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap();
    let manifest_before =
        std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let boundary = Rc::new(RefCell::new(
        None::<(
            BTreeMap<std::path::PathBuf, Vec<u8>>,
            Vec<MergeOperationRecord>,
            usize,
        )>,
    ));
    let hook_boundary = Rc::clone(&boundary);
    let hook_store = Rc::clone(&store);
    let hook_app = app.clone();
    Git2Backend::before_next_prepared_execution(move || {
        std::fs::write(hook_app.join(".git/HEAD"), "ref: refs/heads/other\n").unwrap();
        *hook_boundary.borrow_mut() = Some((
            file_snapshot(&hook_app),
            hook_store.records.lock().unwrap().clone(),
            *hook_store.writes.lock().unwrap(),
        ));
    });
    Git2Backend::reset_preparation_call_count();

    let error = resume(&backend, store.as_ref(), root.path()).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(Git2Backend::preparation_call_count(), 0);
    let head = backend.head(&app).unwrap();
    assert_eq!(head.branch.as_deref(), Some("other"));
    assert_eq!(
        head.commit.as_deref(),
        Some(plan.participants[0].before_commit.as_str())
    );
    assert_eq!(backend.merge_state(&app).unwrap(), native_before);
    let boundary = boundary.borrow();
    let (repository_at_boundary, records_at_boundary, writes_at_boundary) =
        boundary.as_ref().expect("execution hook must run");
    assert_eq!(&file_snapshot(&app), repository_at_boundary);
    assert_eq!(
        store.records.lock().unwrap().as_slice(),
        records_at_boundary
    );
    assert_eq!(*store.writes.lock().unwrap(), *writes_at_boundary);
    assert!(store.events.lock().unwrap().is_empty());
    assert!(records_at_boundary.iter().all(|record| {
        let participant = &record.participants["mem_app"];
        participant.pending_action.as_ref() == Some(&frozen_pending)
            && participant.state == ParticipantState::Conflicted
            && participant.error.is_none()
    }));
    assert_eq!(
        std::fs::read(root.path().join(artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        std::fs::read(root.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        manifest_before
    );
}
