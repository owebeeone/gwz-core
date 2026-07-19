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
    let result = backend
        .merge_upstream_checked(&repo, "main", &before, &source, message, None)
        .unwrap();
    let commit = result.commit.unwrap();

    assert!(
        backend
            .commit_matches_merge(&repo, &commit, &before, &source, message)
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

    let result = backend
        .commit_merge_resolution_checked(&repo, &before, &source, "resolved", None)
        .unwrap();
    let repository = git2::Repository::open(&repo).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(&result.commit).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
    assert_eq!(commit.message(), Ok("resolved"));
    assert!(backend.merge_state(&repo).unwrap().is_none());
}
