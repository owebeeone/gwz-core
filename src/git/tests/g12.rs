use std::fs;
use std::path::Path;

use crate::model::ErrorCode;

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
