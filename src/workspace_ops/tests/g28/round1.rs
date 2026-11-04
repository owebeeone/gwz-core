//! gwz 1.0.19 hotfix, remediation round 1 rows: one per finding of the round-1 Code
//! (C-Fn) and State (S-Fn) reviews, same fixture as the parent rows.
use std::fs;
use std::path::Path;

use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;

use super::*;

/// Commit on HEAD: write `writes` (creating parents) and remove `removes` (files or
/// directories) from the worktree and the index.
pub(super) fn commit_change(
    repo_path: &Path,
    writes: &[(&str, &str)],
    removes: &[&str],
    message: &str,
) -> String {
    let repo = git2::Repository::open(repo_path).unwrap();
    let mut index = repo.index().unwrap();
    for path in removes {
        let full = repo_path.join(path);
        if full.is_dir() {
            fs::remove_dir_all(&full).unwrap();
            index.remove_dir(Path::new(path), 0).unwrap();
        } else {
            fs::remove_file(&full).unwrap();
            index.remove_path(Path::new(path)).unwrap();
        }
    }
    for (path, content) in writes {
        let full = repo_path.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
        index.add_path(Path::new(path)).unwrap();
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &[&parent],
    )
    .unwrap()
    .to_string()
}

/// Push a new root commit from a fresh peer clone, adding `path`.
pub(super) fn push_root_commit(ws: &PullWorkspace, peer: &str, path: &str) -> String {
    let backend = Git2Backend::new();
    let peer = ws._remotes.path().join(peer);
    backend
        .clone_repo(ws._remotes.path().join("root.git").to_str().unwrap(), &peer)
        .unwrap();
    set_identity(&peer);
    let commit = commit_change(&peer, &[(path, "peer\n")], &[], "root peer commit");
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    commit
}

/// A workspace pulled once, so the member tracks `conf/a.txt`; then the member's
/// remote replaces the directory `conf/` with a file `conf`, and the root's remote
/// gains a commit too, so the next pull would move both.
pub(super) fn typechange_workspace() -> PullWorkspace {
    let mut ws = pull_workspace(&[("conf/a.txt", "a\n")], &[]);
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
    let backend = Git2Backend::new();
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_change(&source, &[("conf", "file\n")], &["conf"], "conf is a file");
    backend
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer2", "root-two.txt");
    ws
}

fn git_cli(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// The bytes and modification time of a repository's index.
fn index_stamp(repo: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let index = repo.join(".git/index");
    (
        fs::read(&index).unwrap(),
        fs::metadata(&index).unwrap().modified().unwrap(),
    )
}

// C-F1 (P1): untracked content in a tracked directory the update turns into a file
// refuses before anything is mutated, as git does ("would lose untracked files"), with
// and without `--force`; once the user deletes that content the update lands.
#[test]
fn row11_untracked_content_in_a_directory_becoming_a_file_refuses_even_with_force() {
    let ws = typechange_workspace();
    fs::write(ws.member.join("conf/local.txt"), "mine\n").unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(&ws, crate::SyncBehavior::FfOnly, &["conf"], &before);

    fs::remove_file(ws.member.join("conf/local.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("conf")).unwrap(),
        "file\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "junk\n"
    );
}

// C-F2 / S-F2 (P2), member: a true merge is judged against the merged tree. An
// untracked file at a path the local side deleted (the merge leaves it absent) neither
// refuses nor is removed; a path the merge adds still collides, with and without `--force`.
#[test]
fn row12_member_merge_judges_collisions_against_the_merge_result() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    commit_change(&ws.member, &[], &["README.md"], "drop readme locally");
    fs::write(ws.member.join("README.md"), "my notes\n").unwrap();
    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
    let before = snapshot(&ws);

    let error = pull(&ws, crate::SyncBehavior::Merge, false).unwrap_err();

    assert_collision_refusal(&error, &["new.txt"]);
    assert!(
        !error.message.contains("README.md"),
        "the merge result does not touch README.md: {}",
        error.message
    );
    assert_eq!(snapshot(&ws), before, "a refusal mutated the workspace");

    let error = pull(&ws, crate::SyncBehavior::Merge, true).unwrap_err();
    assert_collision_refusal(&error, &["new.txt"]);
    assert_eq!(snapshot(&ws), before, "a refusal mutated the workspace");

    fs::remove_file(ws.member.join("new.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::Merge, false).unwrap();

    assert_eq!(head(&ws.root), ws.root_after);
    assert_eq!(
        fs::read_to_string(ws.member.join("new.txt")).unwrap(),
        "incoming\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("README.md")).unwrap(),
        "my notes\n",
        "an untracked file the merge never writes was removed"
    );
}

// C-F2 / S-F2 (P2), root: the same rule for a diverged root under `--sync merge`.
#[test]
fn row12b_root_merge_judges_collisions_against_the_merge_result() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    let backend = Git2Backend::new();
    commit_change(&ws.root, &[("notes.txt", "tracked\n")], &[], "add notes");
    backend
        .push(&ws.root, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    commit_change(&ws.root, &[], &["notes.txt"], "drop notes locally");
    push_root_commit(&ws, "peer3", "root-three.txt");
    fs::write(ws.root.join("notes.txt"), "my notes\n").unwrap();

    pull(&ws, crate::SyncBehavior::Merge, false)
        .unwrap_or_else(|error| panic!("refused where git merges: {}", error.message));

    assert_eq!(
        fs::read_to_string(ws.root.join("notes.txt")).unwrap(),
        "my notes\n"
    );
    assert_eq!(
        fs::read_to_string(ws.root.join("root-three.txt")).unwrap(),
        "peer\n"
    );
}

// S-F1 (P1): a pull past untracked member and root files leaves the root clean (no
// staged gwz.conf), so the next pull succeeds too.
#[test]
fn row13_successful_pull_with_untracked_files_leaves_the_root_clean() {
    for sync in [crate::SyncBehavior::FfOnly, crate::SyncBehavior::Merge] {
        let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
        fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();
        fs::write(ws.root.join("junk-root.txt"), "root junk\n").unwrap();

        pull(&ws, sync, false).unwrap_or_else(|error| panic!("{sync:?}: {}", error.message));

        assert_pulled(&ws);
        let status = Git2Backend::new().status(&ws.root).unwrap();
        assert!(
            !status.has_tracked_changes(),
            "{sync:?}: the pull left root changes: {:?}",
            status.files
        );
        pull(&ws, sync, false)
            .unwrap_or_else(|error| panic!("{sync:?}: second pull: {}", error.message));
        assert_eq!(
            fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
            "mine\n"
        );
    }
}

// C-F3 (P2): a stale committed conf-integrity marker (gwz.conf committed by plain git)
// with a moving root: the pull succeeds and the marker verifies, instead of the repair
// staging the marker before the root update and that update refusing it.
#[test]
fn row14_stale_committed_marker_with_a_moving_root_pulls() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    let backend = Git2Backend::new();
    {
        backend.fetch(&ws.root, "origin").unwrap();
        let repo = git2::Repository::open(&ws.root).unwrap();
        let after = repo
            .find_object(git2::Oid::from_str(&ws.root_after).unwrap(), None)
            .unwrap();
        repo.reset(&after, git2::ResetType::Hard, None).unwrap();
    }
    let marker = ws.root.join(crate::artifact::CONF_INTEGRITY_MARKER_PATH);
    let stale = fs::read_to_string(&marker)
        .unwrap()
        .replace("sha256:", "sha256:0");
    commit_change(
        &ws.root,
        &[(crate::artifact::CONF_INTEGRITY_MARKER_PATH, stale.as_str())],
        &[],
        "stale marker",
    );
    backend
        .push(&ws.root, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let remote = push_root_commit(&ws, "peer3", "root-three.txt");
    assert!(crate::artifact::inspect_conf_integrity(&ws.root).refuses());
    assert!(!backend.status(&ws.root).unwrap().has_tracked_changes());

    pull(&ws, crate::SyncBehavior::FfOnly, false)
        .unwrap_or_else(|error| panic!("{}", error.message));

    assert_eq!(head(&ws.root), remote);
    assert!(
        !crate::artifact::inspect_conf_integrity(&ws.root).refuses(),
        "the marker was not repaired"
    );
}

// S-F3 (P2): an untracked blocker that holds a nested repository refuses before the
// root or the member moves, with and without `--force`, naming the repository (the
// reason is not "move or delete").
#[test]
fn row15_a_blocker_holding_a_repository_refuses_before_any_mutation() {
    let ws = pull_workspace(&[("blocked.txt", "blocked\n")], &[]);
    fs::create_dir_all(ws.member.join("blocked.txt/inner")).unwrap();
    fs::write(ws.member.join("blocked.txt/work.txt"), "work\n").unwrap();
    git2::Repository::init(ws.member.join("blocked.txt/inner")).unwrap();
    let before = snapshot(&ws);

    for force in [false, true] {
        let error = pull(&ws, crate::SyncBehavior::FfOnly, force).unwrap_err();

        assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
        assert!(error.message.contains("repository"), "{}", error.message);
        assert!(
            error.message.contains("blocked.txt") && !error.message.contains("--force"),
            "{}",
            error.message
        );
        assert_untouched(&ws, &before);
    }
}

// S-F4 (P2): the collision dry run never writes `.git/index`: a git-written index (v4,
// untracked cache) keeps its bytes and mtime across a refused pull, a refused and a
// successful dry run, and a pull refused at the barrier.
#[test]
fn row16_refused_and_dry_run_pulls_leave_every_index_unwritten() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    git_cli(&ws.member, &["update-index", "--index-version", "4"]);
    git_cli(
        &ws.member,
        &[
            "-c",
            "core.untrackedCache=true",
            "update-index",
            "--untracked-cache",
        ],
    );
    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
    git_cli(
        &ws.member,
        &["-c", "core.untrackedCache=true", "status", "--porcelain"],
    );
    let stamps = || [index_stamp(&ws.root), index_stamp(&ws.member)];
    let before = stamps();
    assert!(
        before[1].0.windows(4).any(|window| window == b"UNTR"),
        "fixture: git wrote no untracked cache"
    );
    let dry_run = || {
        handle_pull_head(
            &Git2Backend::new(),
            &ws.root,
            pull_request(crate::SyncBehavior::FfOnly, true, true),
            "op_pull_dry",
        )
    };

    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap_err();
    assert_eq!(stamps(), before, "a refused pull wrote an index");

    dry_run().unwrap_err();
    assert_eq!(stamps(), before, "a refused dry run wrote an index");

    fs::remove_file(ws.member.join("new.txt")).unwrap();
    dry_run().unwrap();
    assert_eq!(stamps(), before, "a dry run wrote an index");

    let readme = ws.member.join("README.md");
    let original = fs::read(&readme).unwrap();
    let late = readme.clone();
    crate::workspace_ops::pull_head_barrier::before_next_pull_barrier(move || {
        fs::write(late, "late edit\n").unwrap();
    });
    let error = pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap_err();
    fs::write(&readme, original).unwrap();
    assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
    assert_eq!(
        stamps(),
        before,
        "a pull refused at the barrier wrote an index"
    );
}
