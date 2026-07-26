use crate::git::Git2Backend;
use crate::workspace_ops::tests::{TempDir, commit_file, request_meta, test_member_state};
use std::cell::RefCell;

use super::*;

pub(super) struct DriftOnInspection {
    pub(super) backend: Git2Backend,
    pub(super) drift_path: std::path::PathBuf,
    pub(super) inspected: RefCell<Vec<String>>,
}

pub(super) fn real_three_member_plan(
    root: &Path,
    backend: &Git2Backend,
) -> (super::super::MergePlan, Vec<String>, Vec<String>) {
    backend.create_repo(root).unwrap();
    crate::workspace_ops::tests::set_identity(root);
    let mut lock = artifact::LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: "ws_ops".to_owned(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members: Default::default(),
    };
    let mut bases = Vec::new();
    let mut sources = Vec::new();
    for path in ["app", "lib", "tool"] {
        let repo = root.join(path);
        backend.create_repo(&repo).unwrap();
        let before = commit_file(&repo, "README.md", "base\n", "base", &[]).unwrap();
        backend
            .branch_create(&repo, "feature/source", "HEAD")
            .unwrap();
        backend.switch_branch(&repo, "feature/source").unwrap();
        let source = commit_file(
            &repo,
            "source.txt",
            "source\n",
            "source",
            &[git2::Oid::from_str(&before).unwrap()],
        )
        .unwrap();
        backend.switch_branch(&repo, "main").unwrap();
        let mut state = test_member_state(path, Some(before.clone()), false);
        state.source_id = Some(format!("src_{path}"));
        lock.members.insert(format!("mem_{path}"), state);
        bases.push(before);
        sources.push(source);
    }
    artifact::write_manifest(
        root,
        &artifact::ManifestArtifact {
            schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: artifact::WorkspaceHeader {
                id: "ws_ops".to_owned(),
            },
            members: ["app", "lib", "tool"]
                .into_iter()
                .map(|path| artifact::ManifestMember {
                    id: format!("mem_{path}"),
                    path: path.to_owned(),
                    source_kind: artifact::ArtifactSourceKind::Git,
                    source_id: format!("src_{path}"),
                    active: true,
                    desired: None,
                    remotes: Vec::new(),
                })
                .collect(),
        },
    )
    .unwrap();
    artifact::write_lock(root, &lock).unwrap();
    let request = crate::MergeRequest {
        meta: request_meta(),
        op: crate::MergeOp::Start,
        source_ref: Some("feature/source".to_owned()),
        ..Default::default()
    };
    let plan = plan_merge(backend, root, &request).unwrap();
    (plan, bases, sources)
}

#[derive(Clone, Copy)]
pub(super) enum ActionFixture {
    FastForward,
    TrueMerge,
    Conflict,
}

pub(super) fn single_real_plan(
    root: &Path,
    backend: &Git2Backend,
    fixture: ActionFixture,
) -> super::super::MergePlan {
    let (mut plan, bases, sources) = real_three_member_plan(root, backend);
    let app = root.join("app");
    match fixture {
        ActionFixture::FastForward => {}
        ActionFixture::TrueMerge => {
            commit_file(
                &app,
                "local.txt",
                "local\n",
                "local",
                &[git2::Oid::from_str(&bases[0]).unwrap()],
            )
            .unwrap();
        }
        ActionFixture::Conflict => {
            backend.switch_branch(&app, "feature/source").unwrap();
            commit_file(
                &app,
                "README.md",
                "source\n",
                "source conflict",
                &[git2::Oid::from_str(&sources[0]).unwrap()],
            )
            .unwrap();
            backend.switch_branch(&app, "main").unwrap();
            commit_file(
                &app,
                "README.md",
                "local\n",
                "local conflict",
                &[git2::Oid::from_str(&bases[0]).unwrap()],
            )
            .unwrap();
        }
    }
    if !matches!(fixture, ActionFixture::FastForward) {
        plan = plan_merge(
            backend,
            root,
            &crate::MergeRequest {
                meta: request_meta(),
                op: crate::MergeOp::Start,
                source_ref: Some("feature/source".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    plan.participants.truncate(1);
    plan
}

#[test]
fn clean_start_store_failures_adopt_exact_results_without_duplicate_git() {
    for (name, fixture, expected) in [
        (
            "ff",
            ActionFixture::FastForward,
            crate::MergeParticipantState::FastForwarded,
        ),
        (
            "true",
            ActionFixture::TrueMerge,
            crate::MergeParticipantState::Merged,
        ),
    ] {
        let root = TempDir::new(&format!("merge-{name}-action-recovery"));
        let backend = Git2Backend::new();
        let plan = single_real_plan(root.path(), &backend, fixture);
        let store = start_with_outcome_fault(&backend, root.path(), &plan);
        let app = root.path().join("app");
        let result = backend.head(&app).unwrap().commit.unwrap();
        {
            let records = store.records.lock().unwrap();
            let pending = records.last().unwrap().participants["mem_app"]
                .pending_action
                .as_ref()
                .unwrap();
            match fixture {
                ActionFixture::FastForward => {
                    assert_eq!(
                        pending.expected_result,
                        Some(PendingMergeExpectedResult::FastForward)
                    );
                    assert!(pending.commit_spec.is_none());
                }
                ActionFixture::TrueMerge => {
                    assert_eq!(
                        pending.expected_result,
                        Some(PendingMergeExpectedResult::Commit)
                    );
                    assert!(pending.commit_spec.is_some());
                }
                ActionFixture::Conflict => unreachable!(),
            }
        }
        let response = resume(&backend, &store, root.path()).unwrap();

        assert_eq!(response.repos[0].state, expected);
        assert_eq!(
            response.repos[0].resulting_commit.as_deref(),
            Some(&*result)
        );
        assert_eq!(backend.head(&app).unwrap().commit, Some(result));
        assert!(
            store.records.lock().unwrap().last().unwrap().participants["mem_app"]
                .pending_action
                .is_none()
        );
    }
}

#[test]
fn conflict_and_resolution_store_failures_reconcile_after_reload() {
    let root = TempDir::new("merge-conflict-action-recovery");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
    let app = root.path().join("app");
    let store = start_with_outcome_fault(&backend, root.path(), &plan);
    assert!(backend.merge_state(&app).unwrap().is_some());

    let unresolved = resume(&backend, &store, root.path()).unwrap_err();
    assert_eq!(unresolved.code, ErrorCode::MergeDrift);
    assert_eq!(
        store.records.lock().unwrap().last().unwrap().participants["mem_app"].state,
        ParticipantState::Conflicted
    );
    assert!(
        store.records.lock().unwrap().last().unwrap().participants["mem_app"]
            .pending_action
            .is_none()
    );

    std::fs::write(app.join("README.md"), "resolved\n").unwrap();
    backend.stage_paths(&app, &["README.md"]).unwrap();
    let fail_resolution_outcome = *store.writes.lock().unwrap() + 3;
    *store.fail_write_at.lock().unwrap() = Some(fail_resolution_outcome);
    let attribution = attributed_context();
    resume_with_context(&backend, &store, root.path(), &attribution).unwrap_err();
    let resolution_commit = backend.head(&app).unwrap().commit.unwrap();
    assert!(backend.merge_state(&app).unwrap().is_none());
    assert_eq!(
        store.records.lock().unwrap().last().unwrap().participants["mem_app"]
            .pending_action
            .as_ref()
            .unwrap()
            .kind,
        PendingMergeActionKind::ResolveConflict
    );
    {
        let records = store.records.lock().unwrap();
        let pending = records.last().unwrap().participants["mem_app"]
            .pending_action
            .as_ref()
            .unwrap();
        assert_eq!(
            pending.expected_result,
            Some(PendingMergeExpectedResult::Commit)
        );
        assert!(pending.commit_spec.is_some());
    }

    *store.fail_write_at.lock().unwrap() = None;
    let response = resume(&backend, &store, root.path()).unwrap();
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Continued
    );
    assert_eq!(
        response.repos[0].resulting_commit.as_deref(),
        Some(resolution_commit.as_str())
    );
    assert_eq!(backend.head(&app).unwrap().commit, Some(resolution_commit));
    let repository = git2::Repository::open(&app).unwrap();
    let commit = repository
        .find_commit(
            git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(commit.author().name(), Ok("Merge Request Author"));
    assert_eq!(commit.committer().name(), Ok("Merge Request Committer"));
}

#[test]
fn real_git_drift_halts_with_durable_rows_and_keeps_baseline_lock() {
    let root = TempDir::new("merge-real-halt");
    let backend = Git2Backend::new();
    let (plan, bases, sources) = real_three_member_plan(root.path(), &backend);
    let drifting = DriftOnInspection {
        backend: backend.clone(),
        drift_path: root.path().join("lib"),
        inspected: RefCell::new(Vec::new()),
    };

    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    store.write_open(root.path(), &record).unwrap();
    let sink = TraceSink(&store);
    let emitter = EventEmitter::new(&context(false), &sink, 0);
    execute_durable(
        &drifting,
        &store,
        root.path(),
        &plan.participants,
        None,
        &mut record,
        &emitter,
    )
    .unwrap();
    super::super::persist_operation_transition(
        &store,
        root.path(),
        &mut record,
        OperationState::Halted,
        &emitter,
    )
    .unwrap();
    assert_eq!(*drifting.inspected.borrow(), ["app", "lib"]);
    let response = start_response(&record, &plan.participants, &context(false)).unwrap();

    assert_eq!(response.state, OpState::Halted);
    assert_eq!(
        response.response.meta.aggregate_status,
        AggregateStatus::Failed
    );
    assert_eq!(response.participant_counts.fast_forwarded, 1);
    assert_eq!(response.participant_counts.failed, 1);
    assert_eq!(response.participant_counts.unattempted, 1);
    let ids = response
        .repos
        .iter()
        .map(|repo| repo.target_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["mem_app", "mem_lib", "mem_tool"]);
    assert_eq!(response.repos[0].state, PState::FastForwarded);
    assert_eq!(response.repos[1].state, PState::Failed);
    assert_eq!(response.repos[2].state, PState::Unattempted);
    assert_eq!(
        response.repos[0].live_commit.as_deref(),
        Some(sources[0].as_str())
    );
    assert_eq!(response.repos[1].live_commit, None);
    assert_eq!(response.repos[2].live_commit, None);
    let error = response.repos[1].error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::MergeDrift.into());
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));

    let live = |path| {
        backend
            .head(&root.path().join(path))
            .unwrap()
            .commit
            .unwrap()
    };
    let moved_lib = live("lib");
    assert_ne!(moved_lib, bases[1]);
    assert_eq!(
        (live("app"), live("tool")),
        (sources[0].clone(), bases[2].clone())
    );
    let lock = artifact::read_lock(root.path()).unwrap();
    assert_eq!(
        ["mem_app", "mem_lib", "mem_tool"].map(|id| lock.members[id].commit.as_deref()),
        [
            Some(bases[0].as_str()),
            Some(bases[1].as_str()),
            Some(bases[2].as_str())
        ]
    );
    assert_eq!(record.state, OperationState::Halted);
    assert_eq!(
        record.participants["mem_app"].state,
        ParticipantState::FastForwarded
    );
    assert_eq!(
        record.participants["mem_lib"].state,
        ParticipantState::Failed
    );
    assert_eq!(
        record.participants["mem_tool"].state,
        ParticipantState::Unattempted
    );
    assert!(["app", "lib", "tool"].into_iter().all(|path| {
        backend
            .merge_state(&root.path().join(path))
            .unwrap()
            .is_none()
    }));
}
