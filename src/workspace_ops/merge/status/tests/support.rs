use crate::git::{Git2Backend, GitBackend, GitPreparedCommit, GitPreparedSignature};
use crate::workspace_ops::merge::{
    PendingCommitSpec, PendingGitSignature, PendingMergeExpectedResult,
};

use super::*;

pub(super) fn run_git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=GWZ",
            "-c",
            "user.email=gwz@example.invalid",
        ])
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub(super) fn commit(repo: &Path, file: &str, content: &str, message: &str) -> String {
    fs::write(repo.join(file), content).unwrap();
    run_git(repo, &["add", file]);
    run_git(repo, &["commit", "-m", message]);
    run_git(repo, &["rev-parse", "HEAD"])
}

pub(super) fn seed_divergence(repo: &Path) -> (String, String) {
    Git2Backend::new().create_repo(repo).unwrap();
    commit(repo, "base.txt", "base\n", "base");
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["checkout", "feature"]);
    let source = commit(repo, "source.txt", "source\n", "source");
    run_git(repo, &["checkout", "main"]);
    let before = commit(repo, "main.txt", "main\n", "main");
    (before, source)
}

#[derive(Clone, Copy)]
pub(super) enum CandidateDifference {
    Tree,
    AuthorTime,
    CommitterTime,
}

pub(super) fn alternate_merge_commit(
    repo_path: &Path,
    before: &str,
    source: &str,
    message: &str,
    prepared: &GitPreparedCommit,
    difference: CandidateDifference,
) -> String {
    let repo = git2::Repository::open(repo_path).unwrap();
    let expected_tree = repo
        .find_tree(git2::Oid::from_str(&prepared.tree_oid).unwrap())
        .unwrap();
    let tree_oid = if matches!(difference, CandidateDifference::Tree) {
        let blob = repo.blob(b"post-intent content\n").unwrap();
        let mut builder = repo.treebuilder(Some(&expected_tree)).unwrap();
        builder.insert("post-intent.txt", blob, 0o100644).unwrap();
        builder.write().unwrap()
    } else {
        expected_tree.id()
    };
    let tree = repo.find_tree(tree_oid).unwrap();
    let first = repo
        .find_commit(git2::Oid::from_str(before).unwrap())
        .unwrap();
    let second = repo
        .find_commit(git2::Oid::from_str(source).unwrap())
        .unwrap();
    let signature = |value: &GitPreparedSignature, delta: i64| {
        git2::Signature::new(
            &value.name,
            &value.email,
            &git2::Time::new(value.time_seconds + delta, value.timezone_offset_minutes),
        )
        .unwrap()
    };
    let author = signature(
        &prepared.author,
        i64::from(matches!(difference, CandidateDifference::AuthorTime)),
    );
    let committer = signature(
        &prepared.committer,
        i64::from(matches!(difference, CandidateDifference::CommitterTime)),
    );
    repo.commit(
        None,
        &author,
        &committer,
        message,
        &tree,
        &[&first, &second],
    )
    .unwrap()
    .to_string()
}

pub(super) fn set_expected_conflict(record: &mut MergeParticipantRecord) {
    let pending = record.pending_action.as_mut().unwrap();
    pending.expected_result = Some(PendingMergeExpectedResult::ExpectedConflict);
    pending.commit_spec = None;
}

pub(super) fn set_prepared_commit(
    record: &mut MergeParticipantRecord,
    prepared: &GitPreparedCommit,
) {
    let pending = record.pending_action.as_mut().unwrap();
    pending.expected_result = Some(PendingMergeExpectedResult::Commit);
    pending.commit_spec = Some(PendingCommitSpec {
        tree_oid: prepared.tree_oid.clone(),
        author: pending_signature(&prepared.author),
        committer: pending_signature(&prepared.committer),
        extensions: BTreeMap::new(),
    });
}

pub(super) fn pending_signature(signature: &GitPreparedSignature) -> PendingGitSignature {
    PendingGitSignature {
        name: signature.name.clone(),
        email: signature.email.clone(),
        time_seconds: signature.time_seconds,
        timezone_offset_minutes: signature.timezone_offset_minutes,
        extensions: BTreeMap::new(),
    }
}
