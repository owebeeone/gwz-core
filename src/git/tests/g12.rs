use std::fs;
use std::path::Path;

use crate::model::{ErrorCode, GitObjectIdentity, OperationAttribution};
use crate::runtime::clock::TimestampMs;

use super::*;

fn seed_divergence(path: &Path) -> (String, String, String) {
    let backend = Git2Backend::new();
    backend.create_repo(path).unwrap();
    let base = commit_file(path, "base.txt", "base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    run_git(path, &["branch", "feature"]);
    run_git(path, &["checkout", "feature"]);
    let source = commit_file(path, "feature.txt", "source\n", "source", &[base_oid]).unwrap();
    run_git(path, &["checkout", "main"]);
    let target = commit_file(path, "main.txt", "target\n", "target", &[base_oid]).unwrap();
    (base, target, source)
}

fn create_orphan_ref(path: &Path, ref_name: &str, content: &str) -> String {
    let repo = git2::Repository::open(path).unwrap();
    let blob = repo.blob(content.as_bytes()).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("unrelated.txt", blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    let oid = repo
        .commit(None, &signature, &signature, "unrelated", &tree, &[])
        .unwrap();
    repo.reference(ref_name, oid, true, "test unrelated history")
        .unwrap();
    oid.to_string()
}

fn seed_conflict(path: &Path) -> (String, String) {
    let backend = Git2Backend::new();
    backend.create_repo(path).unwrap();
    let base = commit_file(path, "conflict.txt", "base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    commit_file(path, "stable.txt", "stable\n", "stable", &[base_oid]).unwrap();
    run_git(path, &["branch", "feature"]);
    run_git(path, &["checkout", "feature"]);
    fs::write(path.join("conflict.txt"), "feature\n").unwrap();
    run_git(path, &["commit", "-am", "feature conflict"]);
    let source = rev_parse(path, "HEAD");
    run_git(path, &["checkout", "main"]);
    fs::write(path.join("conflict.txt"), "main\n").unwrap();
    run_git(path, &["commit", "-am", "main conflict"]);
    (rev_parse(path, "HEAD"), source)
}

#[test]
fn merge_analysis_classifies_without_mutating_the_repository() {
    let temp = TempDir::new("merge-analysis");
    let repo = temp.path().join("repo");
    let (base, target, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    run_git(&repo, &["checkout", "-b", "feature-child", "feature"]);
    let source_oid = git2::Oid::from_str(&source).unwrap();
    let feature_child = commit_file(&repo, "child.txt", "child\n", "child", &[source_oid]).unwrap();
    run_git(&repo, &["checkout", "main"]);
    let before_status = backend.status(&repo).unwrap();

    let up_to_date = backend.merge_analysis(&repo, "main", &base).unwrap();
    assert_eq!(up_to_date.kind, GitMergeAnalysisKind::UpToDate);
    assert_eq!(up_to_date.target_commit, target);
    assert_eq!(up_to_date.source_commit, base);
    assert!(!up_to_date.commit_identity_required);
    assert!(up_to_date.prediction_complete);

    let fast_forward = backend
        .merge_analysis(&repo, "feature", &feature_child)
        .unwrap();
    assert_eq!(fast_forward.kind, GitMergeAnalysisKind::FastForward);
    assert_eq!(fast_forward.target_commit, source);
    assert_eq!(fast_forward.source_commit, feature_child);
    assert!(fast_forward.prediction_complete);

    let true_merge = backend.merge_analysis(&repo, "main", &source).unwrap();
    assert_eq!(true_merge.kind, GitMergeAnalysisKind::TrueMerge);
    assert_eq!(true_merge.target_commit, target);
    assert_eq!(true_merge.source_commit, source);
    assert!(true_merge.commit_identity_required);
    assert!(!true_merge.prediction_complete);

    assert_eq!(backend.head(&repo).unwrap().commit, Some(target));
    assert_eq!(backend.status(&repo).unwrap(), before_status);
    assert_eq!(
        git2::Repository::open(&repo).unwrap().state(),
        git2::RepositoryState::Clean
    );
}

#[test]
fn prepared_clean_merge_freezes_exact_content_without_observable_mutation() {
    let temp = TempDir::new("merge-prepared-clean");
    let repo = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    {
        let repository = git2::Repository::open(&repo).unwrap();
        let mut config = repository.config().unwrap();
        config.set_str("user.name", "Frozen Identity").unwrap();
        config
            .set_str("user.email", "frozen@example.invalid")
            .unwrap();
    }
    let head_before = backend.head(&repo).unwrap();
    let status_before = backend.status(&repo).unwrap();
    let index_before = fs::read(repo.join(".git/index")).unwrap();
    let state_before = git2::Repository::open(&repo).unwrap().state();

    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();

    let GitPreparedMerge::Commit(spec) = &prepared else {
        panic!("divergent non-conflicting fixture must prepare a commit")
    };
    assert!(git2::Oid::from_str(&spec.tree_oid).is_ok());
    assert_eq!(backend.head(&repo).unwrap(), head_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(git2::Repository::open(&repo).unwrap().state(), state_before);

    {
        let repository = git2::Repository::open(&repo).unwrap();
        let mut config = repository.config().unwrap();
        config.set_str("user.name", "Changed Identity").unwrap();
        config
            .set_str("user.email", "changed@example.invalid")
            .unwrap();
    }

    let mut wrong = prepared.clone();
    let GitPreparedMerge::Commit(wrong_spec) = &mut wrong else {
        unreachable!()
    };
    wrong_spec.tree_oid = git2::Repository::open(&repo)
        .unwrap()
        .find_commit(git2::Oid::from_str(&before).unwrap())
        .unwrap()
        .tree_id()
        .to_string();
    let error = backend
        .execute_prepared_merge_upstream_checked(
            &repo,
            "main",
            &before,
            &source,
            "frozen message",
            &wrong,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(backend.head(&repo).unwrap(), head_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(git2::Repository::open(&repo).unwrap().state(), state_before);

    let result = backend
        .execute_prepared_merge_upstream_checked(
            &repo,
            "main",
            &before,
            &source,
            "frozen message",
            &prepared,
        )
        .unwrap();
    let repository = git2::Repository::open(&repo).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(result.commit.as_deref().unwrap()).unwrap())
        .unwrap();
    assert_eq!(commit.author().name(), Ok("Frozen Identity"));
    assert_eq!(commit.committer().name(), Ok("Frozen Identity"));
    assert!(
        backend
            .commit_matches_prepared_merge(
                &repo,
                result.commit.as_deref().unwrap(),
                &before,
                &source,
                "frozen message",
                spec,
            )
            .unwrap()
    );
}

#[test]
fn forced_fast_forward_prepares_and_publishes_an_exact_two_parent_commit() {
    let temp = TempDir::new("merge-prepared-forced-commit");
    let repo = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    let before = commit_file(&repo, "base.txt", "base\n", "base", &[]).unwrap();
    run_git(&repo, &["checkout", "-b", "feature"]);
    let source = commit_file(
        &repo,
        "feature.txt",
        "source\n",
        "source",
        &[git2::Oid::from_str(&before).unwrap()],
    )
    .unwrap();
    run_git(&repo, &["checkout", "main"]);

    let prepared = backend
        .prepare_merge_upstream_mode_checked(
            &repo,
            "main",
            &before,
            &source,
            GitPreparedMergeMode::ForceMergeCommit,
            None,
        )
        .unwrap();
    let GitPreparedMerge::Commit(spec) = &prepared else {
        panic!("forced fast-forward must freeze a merge commit")
    };
    let repository = git2::Repository::open(&repo).unwrap();
    assert_eq!(
        spec.tree_oid,
        repository
            .find_commit(git2::Oid::from_str(&source).unwrap())
            .unwrap()
            .tree_id()
            .to_string()
    );
    backend
        .validate_prepared_merge_upstream_state(&repo, "main", &before, &source, &prepared)
        .unwrap();

    let result = backend
        .execute_prepared_merge_upstream_checked(
            &repo,
            "main",
            &before,
            &source,
            "forced merge",
            &prepared,
        )
        .unwrap();
    let commit_id = result.commit.unwrap();
    assert_ne!(commit_id, source);
    let commit = repository
        .find_commit(git2::Oid::from_str(&commit_id).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
    assert_eq!(commit.tree_id().to_string(), spec.tree_oid);
    assert!(
        backend
            .commit_matches_prepared_merge(
                &repo,
                &commit_id,
                &before,
                &source,
                "forced merge",
                spec,
            )
            .unwrap()
    );
}

#[test]
fn prepared_conflict_prediction_does_not_enter_native_merge_state() {
    let temp = TempDir::new("merge-prepared-conflict");
    let repo = temp.path().join("repo");
    let (before, source) = seed_conflict(&repo);
    let backend = Git2Backend::new();
    let head_before = backend.head(&repo).unwrap();
    let status_before = backend.status(&repo).unwrap();
    let index_before = fs::read(repo.join(".git/index")).unwrap();

    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();

    assert_eq!(prepared, GitPreparedMerge::ExpectedConflict);
    assert_eq!(backend.head(&repo).unwrap(), head_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(
        git2::Repository::open(&repo).unwrap().state(),
        git2::RepositoryState::Clean
    );
    assert!(backend.merge_state(&repo).unwrap().is_none());
}

#[test]
fn merge_analysis_resolves_only_local_commit_objects() {
    let temp = TempDir::new("merge-analysis-source");
    let repo = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    commit_file(&repo, "tracked.txt", "one\n", "seed", &[]).unwrap();

    for source in ["HEAD^{tree}", "HEAD:tracked.txt", "missing-source"] {
        let error = backend.merge_analysis(&repo, "main", source).unwrap_err();
        assert_eq!(error.code, ErrorCode::GitCommandFailed, "source={source}");
    }
    assert_eq!(
        backend
            .read_ref(&repo, "refs/heads/missing-source")
            .unwrap(),
        None
    );
}

#[test]
fn merge_upstream_handles_up_to_date_and_fast_forward_with_exact_results() {
    let temp = TempDir::new("merge-simple-results");
    let backend = Git2Backend::new();

    let up_to_date_repo = temp.path().join("up-to-date");
    let (base, target, _) = seed_divergence(&up_to_date_repo);
    let result = backend
        .merge_upstream(&up_to_date_repo, "main", &base)
        .unwrap();
    assert_eq!(result, GitIntegrateResult::clean(target.clone()));
    assert_eq!(backend.head(&up_to_date_repo).unwrap().commit, Some(target));
    assert_eq!(
        backend.status(&up_to_date_repo).unwrap(),
        GitStatus::clean()
    );

    let fast_forward_repo = temp.path().join("fast-forward");
    backend.create_repo(&fast_forward_repo).unwrap();
    let base = commit_file(&fast_forward_repo, "base.txt", "base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    run_git(&fast_forward_repo, &["branch", "feature"]);
    run_git(&fast_forward_repo, &["checkout", "feature"]);
    let source = commit_file(
        &fast_forward_repo,
        "feature.txt",
        "source\n",
        "source",
        &[base_oid],
    )
    .unwrap();
    run_git(&fast_forward_repo, &["checkout", "main"]);
    let result = backend
        .merge_upstream(&fast_forward_repo, "main", "feature")
        .unwrap();
    assert_eq!(result, GitIntegrateResult::clean(source.clone()));
    assert_eq!(
        backend.head(&fast_forward_repo).unwrap().commit,
        Some(source)
    );
    assert_eq!(
        backend.status(&fast_forward_repo).unwrap(),
        GitStatus::clean()
    );
}

#[test]
fn checked_merge_rejects_target_drift_before_mutation() {
    let temp = TempDir::new("merge-checked-drift");
    let repo = temp.path().join("repo");
    let (_, planned_before, source) = seed_divergence(&repo);
    let planned_oid = git2::Oid::from_str(&planned_before).unwrap();
    let moved = commit_file(
        &repo,
        "drift.txt",
        "external\n",
        "external target move",
        &[planned_oid],
    )
    .unwrap();
    let backend = Git2Backend::new();

    let error = backend
        .merge_upstream_checked(
            &repo,
            "main",
            &planned_before,
            &source,
            "must not be committed",
            None,
        )
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(moved));
    assert_eq!(backend.status(&repo).unwrap(), GitStatus::clean());
    assert!(backend.merge_state(&repo).unwrap().is_none());
    assert!(!repo.join(".git/MERGE_HEAD").exists());
}

#[test]
fn checked_merge_rejects_unrelated_history_without_repository_mutation() {
    let temp = TempDir::new("merge-checked-unrelated");
    let repo = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    let target = commit_file(&repo, "target.txt", "target\n", "target", &[]).unwrap();
    let source = create_orphan_ref(&repo, "refs/heads/unrelated", "source\n");
    let target_ref = backend.read_ref(&repo, "refs/heads/main").unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let worktree = fs::read(repo.join("target.txt")).unwrap();
    let status = backend.status(&repo).unwrap();
    let native_state = backend.merge_state(&repo).unwrap();

    let error = backend
        .merge_upstream_checked(
            &repo,
            "main",
            &target,
            &source,
            "must not be committed",
            None,
        )
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert!(error.message.contains("do not share a merge base"));
    assert_eq!(
        backend.read_ref(&repo, "refs/heads/main").unwrap(),
        target_ref
    );
    assert_eq!(
        backend.head(&repo).unwrap().commit.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(repo.join("target.txt")).unwrap(), worktree);
    assert_eq!(backend.status(&repo).unwrap(), status);
    assert_eq!(backend.merge_state(&repo).unwrap(), native_state);
    assert!(!repo.join(".git/MERGE_HEAD").exists());
}

#[test]
fn checked_true_merge_uses_exact_message_identities_and_parents() {
    let temp = TempDir::new("merge-checked-metadata");
    let repo_path = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo_path);
    let backend = Git2Backend::new();
    let message = "Merge 'feature' into 'main'\n\nGWZ-Operation-ID: op_test";
    let author = GitObjectIdentity {
        name: "Request Author".into(),
        email: "author@example.invalid".into(),
        time_ms: Some(TimestampMs(1_700_000_000_000)),
        timezone_offset_minutes: Some(600),
    };
    let committer = GitObjectIdentity {
        name: "Request Committer".into(),
        email: "committer@example.invalid".into(),
        time_ms: Some(TimestampMs(1_700_000_100_000)),
        timezone_offset_minutes: Some(-300),
    };
    let attribution = OperationAttribution {
        git_author: Some(author),
        git_committer: Some(committer),
        ..OperationAttribution::default()
    };

    let result = backend
        .merge_upstream_checked(
            &repo_path,
            "main",
            &before,
            &source,
            message,
            Some(&attribution),
        )
        .unwrap();
    let merge_oid = git2::Oid::from_str(result.commit.as_deref().unwrap()).unwrap();
    let repo = git2::Repository::open(&repo_path).unwrap();
    let commit = repo.find_commit(merge_oid).unwrap();

    assert_eq!(commit.message(), Ok(message));
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
    assert_eq!(commit.author().name(), Ok("Request Author"));
    assert_eq!(commit.author().email(), Ok("author@example.invalid"));
    assert_eq!(commit.author().when().seconds(), 1_700_000_000);
    assert_eq!(commit.author().when().offset_minutes(), 600);
    assert_eq!(commit.committer().name(), Ok("Request Committer"));
    assert_eq!(commit.committer().email(), Ok("committer@example.invalid"));
    assert_eq!(commit.committer().when().seconds(), 1_700_000_100);
    assert_eq!(commit.committer().when().offset_minutes(), -300);
    assert_eq!(backend.status(&repo_path).unwrap(), GitStatus::clean());
    assert!(backend.merge_state(&repo_path).unwrap().is_none());
}

#[test]
fn complete_repository_state_mapping_covers_every_libgit2_variant() {
    use crate::git::gitbackend::map_repository_state;

    let cases = [
        (git2::RepositoryState::Clean, GitRepositoryState::Clean),
        (git2::RepositoryState::Merge, GitRepositoryState::Merge),
        (git2::RepositoryState::Revert, GitRepositoryState::Revert),
        (
            git2::RepositoryState::RevertSequence,
            GitRepositoryState::RevertSequence,
        ),
        (
            git2::RepositoryState::CherryPick,
            GitRepositoryState::CherryPick,
        ),
        (
            git2::RepositoryState::CherryPickSequence,
            GitRepositoryState::CherryPickSequence,
        ),
        (git2::RepositoryState::Bisect, GitRepositoryState::Bisect),
        (git2::RepositoryState::Rebase, GitRepositoryState::Rebase),
        (
            git2::RepositoryState::RebaseInteractive,
            GitRepositoryState::RebaseInteractive,
        ),
        (
            git2::RepositoryState::RebaseMerge,
            GitRepositoryState::RebaseMerge,
        ),
        (
            git2::RepositoryState::ApplyMailbox,
            GitRepositoryState::ApplyMailbox,
        ),
        (
            git2::RepositoryState::ApplyMailboxOrRebase,
            GitRepositoryState::ApplyMailboxOrRebase,
        ),
    ];

    for (native, expected) in cases {
        assert_eq!(map_repository_state(native), expected);
    }
}

#[test]
fn exact_merge_commit_matcher_checks_ordered_parents_and_message() {
    let temp = TempDir::new("merge-exact-match");
    let repo = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    let message = "frozen merge message";
    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let result = backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, message, &prepared,
        )
        .unwrap();
    let commit = result.commit.unwrap();
    let GitPreparedMerge::Commit(spec) = prepared else {
        panic!("fixture must prepare a commit")
    };

    assert!(
        backend
            .commit_matches_merge(&repo, &commit, &before, &source, message)
            .unwrap()
    );
    assert!(
        backend
            .commit_matches_prepared_merge(&repo, &commit, &before, &source, message, &spec,)
            .unwrap()
    );
    let mut wrong_tree = spec.clone();
    wrong_tree.tree_oid = git2::Repository::open(&repo)
        .unwrap()
        .find_commit(git2::Oid::from_str(&before).unwrap())
        .unwrap()
        .tree_id()
        .to_string();
    assert!(
        !backend
            .commit_matches_prepared_merge(&repo, &commit, &before, &source, message, &wrong_tree,)
            .unwrap()
    );
    let mut wrong_author = spec.clone();
    wrong_author.author.time_seconds += 1;
    assert!(!backend
        .commit_matches_prepared_merge(
            &repo,
            &commit,
            &before,
            &source,
            message,
            &wrong_author,
        )
        .unwrap());
    let mut wrong_committer = spec.clone();
    wrong_committer.committer.timezone_offset_minutes += 1;
    assert!(
        !backend
            .commit_matches_prepared_merge(
                &repo,
                &commit,
                &before,
                &source,
                message,
                &wrong_committer,
            )
            .unwrap()
    );
    assert!(
        !backend
            .commit_matches_merge(&repo, &commit, &source, &before, message)
            .unwrap()
    );
    assert!(
        !backend
            .commit_matches_merge(&repo, &commit, &before, &source, "changed")
            .unwrap()
    );
    assert!(
        !backend
            .commit_matches_merge(
                &repo,
                "0000000000000000000000000000000000000000",
                &before,
                &source,
                message,
            )
            .unwrap()
    );
}

#[test]
fn checked_true_merge_falls_back_each_identity_independently() {
    let temp = TempDir::new("merge-checked-identity-fallback");
    let backend = Git2Backend::new();
    for (case, author, committer, expected_author, expected_committer) in [
        (
            "author-only",
            Some(GitObjectIdentity::new(
                "Request Author",
                "author@example.invalid",
            )),
            None,
            "Request Author",
            "Repository Identity",
        ),
        (
            "committer-only",
            None,
            Some(GitObjectIdentity::new(
                "Request Committer",
                "committer@example.invalid",
            )),
            "Repository Identity",
            "Request Committer",
        ),
    ] {
        let repo_path = temp.path().join(case);
        let (_, before, source) = seed_divergence(&repo_path);
        {
            let repo = git2::Repository::open(&repo_path).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Repository Identity").unwrap();
            config
                .set_str("user.email", "repository@example.invalid")
                .unwrap();
        }
        let attribution = OperationAttribution {
            git_author: author,
            git_committer: committer,
            ..OperationAttribution::default()
        };

        let result = backend
            .merge_upstream_checked(
                &repo_path,
                "main",
                &before,
                &source,
                "checked identity fallback",
                Some(&attribution),
            )
            .unwrap();
        let repo = git2::Repository::open(&repo_path).unwrap();
        let oid = git2::Oid::from_str(result.commit.as_deref().unwrap()).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.author().name(), Ok(expected_author));
        assert_eq!(commit.committer().name(), Ok(expected_committer));
    }
}

#[test]
fn dirty_and_native_merge_state_are_precise_rejection_signals() {
    let temp = TempDir::new("merge-preflight-signals");
    let repo = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();

    fs::write(repo.join("untracked.txt"), "local\n").unwrap();
    let error = backend
        .merge_upstream(&repo, "main", "feature")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));
    fs::remove_file(repo.join("untracked.txt")).unwrap();

    fs::write(repo.join("base.txt"), "main conflict\n").unwrap();
    run_git(&repo, &["add", "base.txt"]);
    run_git(&repo, &["commit", "-m", "main conflict"]);
    run_git(&repo, &["checkout", "feature"]);
    fs::write(repo.join("base.txt"), "feature conflict\n").unwrap();
    run_git(&repo, &["add", "base.txt"]);
    run_git(&repo, &["commit", "-m", "feature conflict"]);
    let merge_head = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "main"]);

    let result = backend.merge_upstream(&repo, "main", "feature").unwrap();
    assert_eq!(result.conflicts, vec!["base.txt"]);
    let status = backend.status(&repo).unwrap();
    assert!(status.is_dirty);
    assert_eq!(status.unresolved, 1);
    let state = backend.merge_state(&repo).unwrap().unwrap();
    assert_eq!(state.merge_head, merge_head);
    assert_eq!(state.conflict_paths, vec!["base.txt"]);
    assert_eq!(state.unresolved_entries, 1);

    let error = backend.merge_analysis(&repo, "main", &source).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitCommandFailed);
}

#[test]
fn checked_native_abort_rejects_drift_and_dirt_then_is_idempotent() {
    let temp = TempDir::new("merge-checked-abort");
    let repo = temp.path().join("repo");
    let (before, source) = seed_conflict(&repo);
    let backend = Git2Backend::new();
    backend.merge_upstream(&repo, "main", "feature").unwrap();
    let error = backend.abort_merge(&repo, &before, &before).unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert!(backend.merge_state(&repo).unwrap().is_some());
    fs::write(repo.join("stable.txt"), "post-merge work\n").unwrap();
    let error = backend.abort_merge(&repo, &before, &source).unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    run_git(&repo, &["checkout", "--", "stable.txt"]);
    backend.abort_merge(&repo, &before, &source).unwrap();
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));
    assert!(backend.merge_state(&repo).unwrap().is_none());
    backend.abort_merge(&repo, &before, &source).unwrap();
}

#[test]
fn checked_clean_rollback_rejects_current_oid_and_dirt_then_is_idempotent() {
    let temp = TempDir::new("merge-checked-rollback");
    let repo = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    let merged = backend
        .merge_upstream_checked(&repo, "main", &before, &source, "merge", None)
        .unwrap()
        .commit
        .unwrap();
    fs::write(repo.join("untracked.txt"), "keep\n").unwrap();
    let error = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(merged.clone()));
    fs::remove_file(repo.join("untracked.txt")).unwrap();
    let error = backend
        .set_branch_target_checked(&repo, "main", &source, &before)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(merged.clone()));

    backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap();
    let repeated = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap();
    assert!(!repeated.updated);
}

#[test]
fn checked_rollback_tolerates_checked_artifact_private_residue() {
    let temp = TempDir::new("merge-checked-rollback-private-residue");
    let repo = temp.path().join("repo");
    let (_, before, source) = seed_divergence(&repo);
    let backend = Git2Backend::new();
    let merged = backend
        .merge_upstream_checked(&repo, "main", &before, &source, "merge", None)
        .unwrap()
        .commit
        .unwrap();
    // The checked-artifact private area retains a durability anchor for the
    // life of the repository on Windows. It is product infrastructure, not
    // user work: rollback preflight and post-verification must stay available
    // over it, while real untracked work keeps rejecting (covered above).
    let private = repo.join(".gwz/checked-artifacts");
    fs::create_dir_all(&private).unwrap();
    let anchor = private.join(".ca1-durability-anchor-deadbeefdeadbeefdeadbeefdeadbeef");
    fs::write(&anchor, b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n").unwrap();
    let result = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap();
    assert!(result.updated);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));
    // The rollback must neither remove nor rewrite the anchor.
    assert_eq!(
        fs::read(&anchor).unwrap(),
        b"GWZ-CHECKED-ARTIFACT-DURABILITY-ANCHOR-V1\n"
    );
}

/// Seed a rollback fixture whose rewrite set crosses an attribute-covered
/// path: `secret.txt` (covered by `attributes`) differs between `before` and
/// the returned merge commit, so a checked rollback must rewrite it.
/// Returns `(before, merged)`.
fn seed_filtered_rollback(repo: &Path, attributes: &str) -> (String, String) {
    let backend = Git2Backend::new();
    backend.create_repo(repo).unwrap();
    let base = commit_file(repo, "secret.txt", "plain-v1\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    let attrs = commit_file(repo, ".gitattributes", attributes, "attrs", &[base_oid]).unwrap();
    let attrs_oid = git2::Oid::from_str(&attrs).unwrap();
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["checkout", "feature"]);
    let source = commit_file(repo, "secret.txt", "plain-v2\n", "feature", &[attrs_oid]).unwrap();
    run_git(repo, &["checkout", "main"]);
    let before = commit_file(repo, "main.txt", "target\n", "target", &[attrs_oid]).unwrap();
    let merged = backend
        .merge_upstream_checked(repo, "main", &before, &source, "merge", None)
        .unwrap()
        .commit
        .unwrap();
    (before, merged)
}

fn configure_filter_driver(repo: &Path, key: &str, value: &str) {
    let repository = git2::Repository::open(repo).unwrap();
    let mut config = repository.config().unwrap();
    config.set_str(key, value).unwrap();
}

#[test]
fn checked_rollback_refuses_configured_foreign_filter_before_any_mutation() {
    // M5-8 A1 Decision Packet, Decision 2 (A′): a recovery-grade checkout
    // whose rewrite set is covered by a CONFIGURED, non-passthrough foreign
    // clean filter (the git-crypt class) must refuse pre-mutation with a
    // typed error naming path and filter — replacing the post-commit
    // `verify_merge_result` wedge with a clean preflight refusal. The driver
    // command never runs under libgit2; its configuration is the hazard.
    let temp = TempDir::new("rollback-foreign-filter");
    let repo = temp.path().join("repo");
    let (before, merged) = seed_filtered_rollback(&repo, "secret.txt filter=crypt\n");
    let backend = Git2Backend::new();
    configure_filter_driver(&repo, "filter.crypt.clean", "crypt-clean %f");

    let error = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert!(
        error.message.contains("secret.txt"),
        "refusal must name the covered path: {}",
        error.message
    );
    assert!(
        error.message.contains("'crypt'"),
        "refusal must name the foreign filter: {}",
        error.message
    );
    // Pre-mutation refusal: ref unmoved, HEAD unmoved, worktree untouched.
    assert_eq!(rev_parse(&repo, "refs/heads/main"), merged);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(merged.clone()));
    assert_eq!(fs::read(repo.join("secret.txt")).unwrap(), b"plain-v2\n");
    // Nothing moved, so a retry re-refuses identically instead of sliding
    // into the idempotent-arm wedge the packet documents.
    let retry = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap_err();
    assert_eq!(retry.code, ErrorCode::DirtyMember);
    assert_eq!(rev_parse(&repo, "refs/heads/main"), merged);

    // The `process`-only driver form is refused the same way.
    let temp_process = TempDir::new("rollback-foreign-filter-process");
    let repo_process = temp_process.path().join("repo");
    let (before, merged) = seed_filtered_rollback(&repo_process, "secret.txt filter=scrub\n");
    configure_filter_driver(&repo_process, "filter.scrub.process", "scrub-process");
    let error = backend
        .set_branch_target_checked(&repo_process, "main", &merged, &before)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert!(error.message.contains("'scrub'"));
    assert_eq!(rev_parse(&repo_process, "refs/heads/main"), merged);
}

#[test]
fn checked_rollback_proceeds_over_lfs_and_unconfigured_filter_attributes() {
    // Decision 2 (A′) refinements: the `lfs` driver is allowlisted by name
    // (pointer blobs round-trip clean; the pointer-bytes-on-disk surprise is
    // disclosed doctrine, not a refusal), and a foreign attribute with NO
    // configured clean/process driver is passthrough by definition — no
    // wedge is possible, so rollback availability is preserved.
    let backend = Git2Backend::new();

    let temp = TempDir::new("rollback-lfs-allowlist");
    let repo = temp.path().join("repo");
    let (before, merged) = seed_filtered_rollback(&repo, "secret.txt filter=lfs\n");
    configure_filter_driver(&repo, "filter.lfs.clean", "git-lfs clean -- %f");
    configure_filter_driver(&repo, "filter.lfs.process", "git-lfs filter-process");
    let result = backend
        .set_branch_target_checked(&repo, "main", &merged, &before)
        .unwrap();
    assert!(result.updated);
    assert_eq!(rev_parse(&repo, "refs/heads/main"), before);
    assert_eq!(fs::read(repo.join("secret.txt")).unwrap(), b"plain-v1\n");

    let temp_unconfigured = TempDir::new("rollback-unconfigured-filter");
    let repo_unconfigured = temp_unconfigured.path().join("repo");
    let (before, merged) = seed_filtered_rollback(&repo_unconfigured, "secret.txt filter=crypt\n");
    let result = backend
        .set_branch_target_checked(&repo_unconfigured, "main", &merged, &before)
        .unwrap();
    assert!(result.updated);
    assert_eq!(rev_parse(&repo_unconfigured, "refs/heads/main"), before);
    assert_eq!(
        fs::read(repo_unconfigured.join("secret.txt")).unwrap(),
        b"plain-v1\n"
    );
}

#[test]
fn checked_native_abort_refuses_configured_foreign_filter_before_any_mutation() {
    // Decision 2 (A′), second recovery-grade site: the checked native-merge
    // abort restore. A conflict ON the filter-covered path puts it in the
    // restore's rewrite set; with a configured foreign driver the abort must
    // refuse before touching the worktree or the merge state.
    let temp = TempDir::new("abort-foreign-filter");
    let repo = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    let base = commit_file(&repo, "secret.txt", "plain-base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    let attrs = commit_file(
        &repo,
        ".gitattributes",
        "secret.txt filter=crypt\n",
        "attrs",
        &[base_oid],
    )
    .unwrap();
    let attrs_oid = git2::Oid::from_str(&attrs).unwrap();
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source = commit_file(&repo, "secret.txt", "feature\n", "feature", &[attrs_oid]).unwrap();
    run_git(&repo, &["checkout", "main"]);
    let before = commit_file(&repo, "secret.txt", "main\n", "main", &[attrs_oid]).unwrap();
    let result = backend.merge_upstream(&repo, "main", "feature").unwrap();
    assert_eq!(result.conflicts, vec!["secret.txt".to_owned()]);
    configure_filter_driver(&repo, "filter.crypt.clean", "crypt-clean %f");

    let conflicted_bytes = fs::read(repo.join("secret.txt")).unwrap();
    let error = backend.abort_merge(&repo, &before, &source).unwrap_err();
    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert!(
        error.message.contains("secret.txt") && error.message.contains("'crypt'"),
        "refusal must name path and filter: {}",
        error.message
    );
    // Pre-mutation refusal: still mid-merge, conflict intact, worktree and
    // HEAD untouched.
    assert!(backend.merge_state(&repo).unwrap().is_some());
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));
    assert_eq!(fs::read(repo.join("secret.txt")).unwrap(), conflicted_bytes);
}

/// DOCTRINE SENTINEL — real-Windows CRLF fail-closed classification.
///
/// An ADOPTED-style repository (raw `git2` init, NOT gwz `create_repo`, with
/// repo-local `core.autocrlf=true` set before any materialization — the
/// ordinary porcelain-clone shape on a Windows autocrlf host) keeps
/// filter-smudged CRLF bytes on disk for every path the recovery checkouts do
/// not rewrite. The v1 reverse observers (`observe_v1_participant_rollback`
/// and siblings) reduce to the raw-byte `checkout_matches_commit_*` compare;
/// a worktree matching NEITHER candidate commit is their
/// `(before=false, after=false)` arm — classification `Ambiguous`, rollback
/// never starts: availability loss, never wrong evidence.
///
/// This test exists to END the windows-matrix CI blindness to that class
/// (it is the only fixture whose worktree is filter-SMUDGED before feeding
/// the raw-byte classification — other autocrlf=true fixtures pin the key
/// after LF materialization — so matrix-green otherwise says nothing about
/// ambient-CRLF worktrees). It PINS today's fail-closed
/// doctrine: `GwzM5-8ExactEvidencePlatformAmendment.md` Clause A scope
/// limits ("ordinary Windows-CRLF worktrees remain unsatisfiable for the
/// raw-byte model") and the `GwzWindowsMatrix-Classification.md` standing
/// residual tripwire. Decision 1 Option B deliberately leaves adopted repos
/// in this class (fail-closed), serving them later via the renormalize
/// operator command. If this test ever FAILS — the smudged worktree stops
/// classifying Ambiguous — someone changed the raw-byte doctrine (clean-side
/// comparison, entry re-materialization, …) and MUST update those frozen
/// texts together with this sentinel.
#[cfg(windows)]
#[test]
fn doctrine_sentinel_adopted_crlf_worktree_classifies_ambiguous_in_the_reverse_observer() {
    let temp = TempDir::new("crlf-doctrine-sentinel");
    let repo_path = temp.path().join("repo");
    let mut opts = git2::RepositoryInitOptions::new();
    opts.bare(false).no_reinit(true).initial_head("main");
    let repository = git2::Repository::init_opts(&repo_path, &opts).unwrap();
    repository
        .config()
        .unwrap()
        .set_bool("core.autocrlf", true)
        .unwrap();
    drop(repository);
    // A text file UNCHANGED between the two candidate commits: recovery
    // checkouts rewrite deltas only, so no recovery edge ever rewrites it.
    let before = commit_file(&repo_path, "unchanged.txt", "stable-line\n", "before", &[]).unwrap();
    let before_oid = git2::Oid::from_str(&before).unwrap();
    let result = commit_file(
        &repo_path,
        "moving.txt",
        "result\n",
        "result",
        &[before_oid],
    )
    .unwrap();
    // Filter-materialize the worktree the way the adopted clone/checkout did:
    // missing files rewritten through the ACTIVE smudge filter land as CRLF.
    fs::remove_file(repo_path.join("unchanged.txt")).unwrap();
    fs::remove_file(repo_path.join("moving.txt")).unwrap();
    let repository = git2::Repository::open(&repo_path).unwrap();
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repository.checkout_head(Some(&mut checkout)).unwrap();
    drop(repository);
    assert_eq!(
        fs::read(repo_path.join("unchanged.txt")).unwrap(),
        b"stable-line\r\n",
        "precondition: the ambient-style smudge materialized CRLF on disk"
    );

    let backend = Git2Backend::new();
    // Filter-aware status stays clean (clean direction strips CRLF): the
    // exposure is invisible to porcelain and bites only raw-byte evidence.
    assert!(!backend.status(&repo_path).unwrap().is_dirty);
    // Raw-byte observation: the worktree matches NEITHER candidate commit —
    // the reverse observer's Ambiguous arm. Fail-closed: rollback never
    // starts, wrong evidence is never accepted.
    assert!(
        !backend
            .checkout_matches_commit_except(&repo_path, &result, &[])
            .unwrap(),
        "doctrine change detected: smudged worktree raw-matched the result commit"
    );
    assert!(
        !backend
            .checkout_matches_commit_except(&repo_path, &before, &[])
            .unwrap(),
        "doctrine change detected: smudged worktree raw-matched the before commit"
    );
}

#[test]
fn checked_resolution_binds_parents_and_rejects_unsafe_index_states() {
    let temp = TempDir::new("merge-checked-resolution");
    let repo = temp.path().join("repo");
    let (before, source) = seed_conflict(&repo);
    let backend = Git2Backend::new();
    backend.merge_upstream(&repo, "main", "feature").unwrap();
    let reject = |expected_before: &str, expected_head: &str| {
        backend
            .commit_merge_resolution_checked(
                &repo,
                "main",
                expected_before,
                expected_head,
                "resolved",
                None,
            )
            .unwrap_err()
            .code
    };
    assert_eq!(reject(&source, &source), ErrorCode::MergeDrift);
    assert_eq!(reject(&before, &before), ErrorCode::MergeDrift);
    assert_eq!(reject(&before, &source), ErrorCode::DirtyMember);

    fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
    run_git(&repo, &["add", "conflict.txt"]);
    fs::write(repo.join("conflict.txt"), "unstaged\n").unwrap();
    assert_eq!(reject(&before, &source), ErrorCode::DirtyMember);
    run_git(&repo, &["checkout", "--", "conflict.txt"]);
    fs::write(repo.join("stable.txt"), "unrelated\n").unwrap();
    run_git(&repo, &["add", "stable.txt"]);
    assert_eq!(reject(&before, &source), ErrorCode::DirtyMember);
    run_git(&repo, &["checkout", "HEAD", "--", "stable.txt"]);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));

    let index_before = fs::read(repo.join(".git/index")).unwrap();
    let status_before = backend.status(&repo).unwrap();
    let prepared = backend
        .prepare_merge_resolution_checked(&repo, "main", &before, &source, None)
        .unwrap();
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    assert_eq!(
        git2::Repository::open(&repo).unwrap().state(),
        git2::RepositoryState::Merge
    );

    let mut wrong = prepared.clone();
    wrong.tree_oid = git2::Repository::open(&repo)
        .unwrap()
        .find_commit(git2::Oid::from_str(&before).unwrap())
        .unwrap()
        .tree_id()
        .to_string();
    let error = backend
        .commit_prepared_merge_resolution_checked(
            &repo, "main", &before, &source, "resolved", &wrong,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(backend.head(&repo).unwrap().commit, Some(before.clone()));
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index_before);
    assert_eq!(backend.status(&repo).unwrap(), status_before);
    assert_eq!(
        git2::Repository::open(&repo).unwrap().state(),
        git2::RepositoryState::Merge
    );

    let result = backend
        .commit_prepared_merge_resolution_checked(
            &repo, "main", &before, &source, "resolved", &prepared,
        )
        .unwrap();
    let repository = git2::Repository::open(&repo).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(&result.commit).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
    assert_eq!(commit.message(), Ok("resolved"));
    assert!(
        backend
            .commit_matches_prepared_merge(
                &repo,
                &result.commit,
                &before,
                &commit.parent_id(1).unwrap().to_string(),
                "resolved",
                &prepared,
            )
            .unwrap()
    );
    assert!(backend.merge_state(&repo).unwrap().is_none());
}
