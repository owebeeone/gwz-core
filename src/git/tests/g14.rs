use std::fs;

use crate::model::ErrorCode;

use super::*;

#[test]
fn committed_file_reads_the_exact_tree_without_consulting_the_worktree() {
    let temp = TempDir::new("read-file-at-commit");
    let repo = temp.path().join("repo");
    let backend = Git2Backend::new();
    backend.create_repo(&repo).unwrap();
    run_git(&repo, &["config", "user.name", "GWZ Test"]);
    run_git(&repo, &["config", "user.email", "gwz-test@example.invalid"]);
    fs::create_dir_all(repo.join("gwz.conf")).unwrap();
    fs::write(repo.join("gwz.conf/gwz.yml"), "version: one\n").unwrap();
    run_git(&repo, &["add", "gwz.conf/gwz.yml"]);
    run_git(&repo, &["commit", "-m", "one"]);
    let first = rev_parse(&repo, "HEAD");
    fs::write(repo.join("gwz.conf/gwz.yml"), "version: two\n").unwrap();

    assert_eq!(
        backend
            .read_file_at_commit(&repo, &first, "gwz.conf/gwz.yml")
            .unwrap(),
        Some(b"version: one\n".to_vec())
    );
    assert_eq!(
        backend
            .read_file_at_commit(&repo, &first, "gwz.conf/missing.yml")
            .unwrap(),
        None
    );
    assert_eq!(
        backend
            .read_file_at_commit(&repo, &first, "../outside")
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}
