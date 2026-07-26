use std::{fs, path::Path};

use super::*;

fn seed_true_merge(path: &Path, conflict: bool) -> (String, String) {
    let backend = Git2Backend::new();
    backend.create_repo(path).unwrap();
    let base = commit_file(path, "shared.txt", "base\n", "base", &[]).unwrap();
    let base_oid = git2::Oid::from_str(&base).unwrap();
    run_git(path, &["branch", "feature"]);
    run_git(path, &["checkout", "feature"]);
    let source_path = if conflict {
        "shared.txt"
    } else {
        "feature.txt"
    };
    let source = commit_file(path, source_path, "source\n", "source", &[base_oid]).unwrap();
    run_git(path, &["checkout", "main"]);
    let target = commit_file(path, "shared.txt", "target\n", "target", &[base_oid]).unwrap();
    (target, source)
}

fn assert_repository_unchanged(
    backend: &Git2Backend,
    path: &Path,
    head: &GitHeadState,
    status: &GitStatus,
    index: &[u8],
) {
    assert_eq!(&backend.head(path).unwrap(), head);
    assert_eq!(&backend.status(path).unwrap(), status);
    assert_eq!(fs::read(path.join(".git/index")).unwrap(), index);
    assert_eq!(
        git2::Repository::open(path).unwrap().state(),
        git2::RepositoryState::Clean
    );
}

#[test]
fn merge_simulation_reports_clean_and_conflicted_true_merges_without_mutation() {
    let temp = TempDir::new("merge-simulation");
    let clean = temp.path().join("clean");
    let conflicted = temp.path().join("conflicted");
    let (clean_target, clean_source) = seed_true_merge(&clean, false);
    let (conflict_target, conflict_source) = seed_true_merge(&conflicted, true);
    let backend = Git2Backend::new();

    let clean_head = backend.head(&clean).unwrap();
    let clean_status = backend.status(&clean).unwrap();
    let clean_index = fs::read(clean.join(".git/index")).unwrap();
    let conflict_head = backend.head(&conflicted).unwrap();
    let conflict_status = backend.status(&conflicted).unwrap();
    let conflict_index = fs::read(conflicted.join(".git/index")).unwrap();

    assert_eq!(
        backend
            .merge_simulate(&clean, &clean_target, &clean_source)
            .unwrap(),
        GitMergeSimulation::Clean
    );
    assert_eq!(
        backend
            .merge_simulate(&conflicted, &conflict_target, &conflict_source)
            .unwrap(),
        GitMergeSimulation::Conflicts(vec!["shared.txt".to_owned()])
    );

    assert_repository_unchanged(&backend, &clean, &clean_head, &clean_status, &clean_index);
    assert_repository_unchanged(
        &backend,
        &conflicted,
        &conflict_head,
        &conflict_status,
        &conflict_index,
    );
}
