use crate::git::{Git2Backend, GitBackend};
use crate::workspace_ops::merge::PendingMergeActionKind;

use super::*;

fn test_root(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("gwz-status-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn missing_expected_and_resulting_commits_are_member_scoped_object_drift() {
    let root = test_root("missing-object");
    let repo = root.join("repos/app");
    Git2Backend::new().create_repo(&repo).unwrap();
    let head = commit(&repo, "tracked.txt", "one\n", "initial");
    let missing = "0000000000000000000000000000000000000000";
    let mut expected = participant(ParticipantState::Unattempted);
    expected.before_commit = missing.to_owned();
    expected.source_commit = head.clone();
    let mut resulting = participant(ParticipantState::Merged);
    resulting.before_commit = head.clone();
    resulting.source_commit = head;
    resulting.resulting_commit = Some(missing.to_owned());

    for record in [expected, resulting] {
        let observed = observe_participant(&Git2Backend::new(), &root, "mem_app", &record).unwrap();
        assert!(
            observed
                .drift
                .iter()
                .any(|drift| drift.kind == ParticipantDriftKind::ObjectMissing)
        );
        assert!(!observed.continue_eligibility.eligible);
    }
    assert!(repo.join("tracked.txt").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_foreign_sequencer_state_is_not_optimistically_accepted() {
    let root = test_root("foreign-state");
    let repo = root.join("repos/app");
    Git2Backend::new().create_repo(&repo).unwrap();
    let head = commit(&repo, "tracked.txt", "one\n", "initial");
    fs::write(repo.join(".git/CHERRY_PICK_HEAD"), format!("{head}\n")).unwrap();
    let mut record = participant(ParticipantState::Merged);
    record.before_commit = head.clone();
    record.source_commit = head.clone();
    record.resulting_commit = Some(head);

    let observed = observe_participant(&Git2Backend::new(), &root, "mem_app", &record).unwrap();

    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::ForeignIntegrationState)
    );
    assert!(!observed.abort_eligibility.eligible);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_true_merge_completion_is_exact_and_read_only() {
    let root = test_root("reconcile-complete");
    let repo = root.join("repos/app");
    let (before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    let message = "frozen message";
    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let result = backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, message, &prepared,
        )
        .unwrap();
    let mut record = pending_record(
        ParticipantState::Planned,
        &before,
        &source,
        message,
        PendingMergeActionKind::TrueMerge,
    );
    let crate::git::GitPreparedMerge::Commit(prepared) = prepared else {
        panic!("fixture must produce a clean merge")
    };
    set_prepared_commit(&mut record, &prepared);
    let index_before = fs::read(repo.join(".git/index")).unwrap();
    let head_before = backend.head(&repo).unwrap();

    let reconciled = reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap();

    assert_eq!(
        reconciled,
        PendingActionReconciliation::Completed {
            resulting_commit: result.commit.unwrap()
        }
    );
    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
    assert_eq!(
        observed
            .pending_action
            .as_ref()
            .map(|pending| pending.state),
        Some(super::super::super::PendingActionObservationState::CompletedExactly)
    );
    assert!(observed.drift.is_empty());
    assert!(observed.continue_eligibility.eligible);
    assert!(observed.abort_eligibility.eligible);
    assert_eq!(backend.head(&repo).unwrap(), head_before);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(backend.status(&repo).unwrap(), GitStatus::clean());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn different_tree_or_signature_commit_is_ambiguous_and_status_is_read_only() {
    for (case, difference) in [
        ("tree", CandidateDifference::Tree),
        ("author", CandidateDifference::AuthorTime),
        ("committer", CandidateDifference::CommitterTime),
    ] {
        let root = test_root(&format!("reconcile-different-{case}"));
        let repo = root.join("repos/app");
        let (before, source) = seed_divergence(&repo);
        let backend = Git2Backend::new();
        let message = "frozen message";
        let prepared = backend
            .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
            .unwrap();
        let crate::git::GitPreparedMerge::Commit(prepared) = prepared else {
            panic!("fixture must prepare a commit")
        };
        let candidate =
            alternate_merge_commit(&repo, &before, &source, message, &prepared, difference);
        run_git(&repo, &["reset", "--hard", &candidate]);
        let mut record = pending_record(
            ParticipantState::Planned,
            &before,
            &source,
            message,
            PendingMergeActionKind::TrueMerge,
        );
        set_prepared_commit(&mut record, &prepared);
        let index_before = fs::read(repo.join(".git/index")).unwrap();
        let status_before = backend.status(&repo).unwrap();

        let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

        assert_eq!(
            observed.pending_action.unwrap().state,
            super::super::super::PendingActionObservationState::Ambiguous,
            "case={case}"
        );
        assert!(!observed.continue_eligibility.eligible, "case={case}");
        assert!(!observed.abort_eligibility.eligible, "case={case}");
        assert_eq!(
            backend.head(&repo).unwrap().commit.as_deref(),
            Some(candidate.as_str())
        );
        assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
        assert_eq!(backend.status(&repo).unwrap(), status_before);
        if matches!(difference, CandidateDifference::Tree) {
            assert!(repo.join("post-intent.txt").is_file());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn old_commit_producing_pending_record_is_ambiguous_but_old_fast_forward_is_classifiable() {
    let root = test_root("reconcile-old-record");
    let repo = root.join("repos/app");
    let (before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    let old_merge = pending_record(
        ParticipantState::Planned,
        &before,
        &source,
        "old merge",
        PendingMergeActionKind::TrueMerge,
    );
    let observed = observe_participant(&backend, &root, "mem_app", &old_merge).unwrap();
    assert_eq!(
        observed.pending_action.unwrap().state,
        super::super::super::PendingActionObservationState::Ambiguous
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);

    let mut old_fast_forward = pending_record(
        ParticipantState::Planned,
        &before,
        &source,
        "old fast-forward",
        PendingMergeActionKind::FastForward,
    );
    old_fast_forward.source_commit = before.clone();
    old_fast_forward
        .pending_action
        .as_mut()
        .unwrap()
        .source_commit = before;
    assert_eq!(
        reconcile_pending_action(&backend, &root, "mem_app", &old_fast_forward).unwrap(),
        PendingActionReconciliation::Completed {
            resulting_commit: old_fast_forward.source_commit.clone()
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_conflict_and_resolved_native_state_are_distinguished() {
    let root = test_root("reconcile-conflict");
    let repo = root.join("repos/app");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    commit(&repo, "conflict.txt", "base\n", "base");
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source = commit(&repo, "conflict.txt", "source\n", "source");
    run_git(&repo, &["checkout", "main"]);
    let before = commit(&repo, "conflict.txt", "main\n", "main");
    let message = "frozen conflict message";
    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    assert_eq!(prepared, crate::git::GitPreparedMerge::ExpectedConflict);
    let result = backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, message, &prepared,
        )
        .unwrap();
    assert_eq!(result.conflicts, vec!["conflict.txt"]);
    let mut record = pending_record(
        ParticipantState::Planned,
        &before,
        &source,
        message,
        PendingMergeActionKind::TrueMerge,
    );
    set_expected_conflict(&mut record);

    assert_eq!(
        reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
        PendingActionReconciliation::ExpectedConflict {
            conflict_paths: vec!["conflict.txt".to_owned()]
        }
    );
    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
    assert_eq!(
        observed
            .pending_action
            .as_ref()
            .map(|pending| pending.state),
        Some(super::super::super::PendingActionObservationState::ExpectedConflict)
    );
    assert!(observed.abort_eligibility.eligible);
    assert_eq!(observed.conflict_paths, vec!["conflict.txt"]);

    fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
    run_git(&repo, &["add", "conflict.txt"]);
    record.state = ParticipantState::Conflicted;
    record.expected_merge_head = Some(source.clone());
    record.pending_action.as_mut().unwrap().kind = PendingMergeActionKind::ResolveConflict;
    let prepared = backend
        .prepare_merge_resolution_checked(&repo, "main", &before, &source, None)
        .unwrap();
    set_prepared_commit(&mut record, &prepared);
    assert_eq!(
        reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
        PendingActionReconciliation::NotStarted
    );
    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();
    assert_eq!(
        observed
            .pending_action
            .as_ref()
            .map(|pending| pending.state),
        Some(super::super::super::PendingActionObservationState::NotStarted)
    );
    assert!(observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);

    let committed = backend
        .commit_prepared_merge_resolution_checked(
            &repo, "main", &before, &source, message, &prepared,
        )
        .unwrap();
    assert_eq!(
        reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap(),
        PendingActionReconciliation::Completed {
            resulting_commit: committed.commit
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_resolution_tree_change_is_ambiguous_without_mutation() {
    let root = test_root("pending-resolution-tree-change");
    let repo = root.join("repos/app");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    commit(&repo, "conflict.txt", "base\n", "base");
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source = commit(&repo, "conflict.txt", "source\n", "source");
    run_git(&repo, &["checkout", "main"]);
    let before = commit(&repo, "conflict.txt", "main\n", "main");
    let conflict = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    assert_eq!(conflict, crate::git::GitPreparedMerge::ExpectedConflict);
    backend
        .execute_prepared_merge_upstream_checked(
            &repo,
            "main",
            &before,
            &source,
            "frozen resolution",
            &conflict,
        )
        .unwrap();
    fs::write(repo.join("conflict.txt"), "resolution A\n").unwrap();
    run_git(&repo, &["add", "conflict.txt"]);
    let prepared = backend
        .prepare_merge_resolution_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let mut record = pending_record(
        ParticipantState::Conflicted,
        &before,
        &source,
        "frozen resolution",
        PendingMergeActionKind::ResolveConflict,
    );
    record.expected_merge_head = Some(source);
    set_prepared_commit(&mut record, &prepared);

    fs::write(repo.join("conflict.txt"), "resolution B\n").unwrap();
    run_git(&repo, &["add", "conflict.txt"]);
    let durable_before = record.clone();
    let head_before = backend.head(&repo).unwrap();
    let index_before = fs::read(repo.join(".git/index")).unwrap();
    let worktree_before = fs::read(repo.join("conflict.txt")).unwrap();
    let native_before = backend.merge_state(&repo).unwrap();
    let status_before = backend.status(&repo).unwrap();

    let reconciliation = reconcile_pending_action(&backend, &root, "mem_app", &record).unwrap();
    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

    assert!(matches!(
        reconciliation,
        PendingActionReconciliation::Ambiguous { .. }
    ));
    assert_eq!(
        observed.pending_action.unwrap().state,
        super::super::super::PendingActionObservationState::Ambiguous
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);
    assert_eq!(record, durable_before);
    assert_eq!(backend.head(&repo).unwrap(), head_before);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(
        fs::read(repo.join("conflict.txt")).unwrap(),
        worktree_before
    );
    assert_eq!(backend.merge_state(&repo).unwrap(), native_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolution_candidate_with_different_tree_is_never_adopted_or_rollback_eligible() {
    let root = test_root("reconcile-resolution-different-tree");
    let repo = root.join("repos/app");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    commit(&repo, "conflict.txt", "base\n", "base");
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source = commit(&repo, "conflict.txt", "source\n", "source");
    run_git(&repo, &["checkout", "main"]);
    let before = commit(&repo, "conflict.txt", "main\n", "main");
    let conflict = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &repo,
            "main",
            &before,
            &source,
            "resolution message",
            &conflict,
        )
        .unwrap();
    fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
    run_git(&repo, &["add", "conflict.txt"]);
    let prepared = backend
        .prepare_merge_resolution_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let candidate = alternate_merge_commit(
        &repo,
        &before,
        &source,
        "resolution message",
        &prepared,
        CandidateDifference::Tree,
    );
    run_git(&repo, &["reset", "--hard", &candidate]);
    let mut record = pending_record(
        ParticipantState::Conflicted,
        &before,
        &source,
        "resolution message",
        PendingMergeActionKind::ResolveConflict,
    );
    record.expected_merge_head = Some(source);
    set_prepared_commit(&mut record, &prepared);
    let index_before = fs::read(repo.join(".git/index")).unwrap();

    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

    assert_eq!(
        observed.pending_action.unwrap().state,
        super::super::super::PendingActionObservationState::Ambiguous
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(candidate));
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert!(repo.join("post-intent.txt").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ambiguous_pending_inputs_are_structured_and_block_recovery() {
    let root = test_root("reconcile-ambiguous");
    let repo = root.join("repos/app");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    let before = commit(&repo, "tracked.txt", "one\n", "initial");
    let mut record = pending_record(
        ParticipantState::Planned,
        &before,
        &before,
        "frozen",
        PendingMergeActionKind::FastForward,
    );
    record.pending_action.as_mut().unwrap().target_branch = "other".to_owned();

    let observed = observe_participant(&backend, &root, "mem_app", &record).unwrap();

    let pending = observed.pending_action.unwrap();
    assert_eq!(
        pending.state,
        super::super::super::PendingActionObservationState::Ambiguous
    );
    assert!(pending.message.unwrap().contains("do not match"));
    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::PendingActionAmbiguous)
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);
    fs::remove_dir_all(root).unwrap();
}
