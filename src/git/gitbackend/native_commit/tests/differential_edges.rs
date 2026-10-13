//! Edge cases native and the Git CLI must agree on: locks, races, and the
//! kinds of tracked change `commit -a` stages.

use std::fs;

use super::support::*;
use crate::git::gitbackend::native_commit::{GitEnvironment, exec, route};
use crate::model::ErrorCode;

/// A repository with files of several kinds committed.
fn rich(path: &std::path::Path) {
    seeded(path);
    write(path, "dir/sub/nested.txt", "nested\n");
    write(path, "crlf.txt", "one\r\ntwo\r\n");
    write(path, "\u{fc}n\u{ef}.txt", "unicode name\n");
    write(path, "empty.txt", "");
    fs::write(path.join("bin.dat"), [0u8, 1, 2, 255, 0, 7]).unwrap();
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            std::os::unix::fs::symlink("tracked.txt", path.join("link")).unwrap();
        }
    }
    git(path, &["add", "-A"]);
    git(path, &["commit", "-q", "-m", "rich"]);
}

#[test]
fn all_stages_every_kind_of_tracked_change_like_git() {
    hermetic!();
    let twin = twin(rich);
    twin.both(|path| {
        write(path, "dir/sub/nested.txt", "nested, changed\n");
        write(path, "crlf.txt", "one\r\ntwo\r\nthree\r\n");
        write(path, "\u{fc}n\u{ef}.txt", "unicode name, changed\n");
        write(path, "empty.txt", "no longer empty\n");
        fs::write(path.join("bin.dat"), [9u8, 0, 0, 1]).unwrap();
        write(path, "new-untracked.txt", "left alone\n");
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                fs::remove_file(path.join("link")).unwrap();
                std::os::unix::fs::symlink("other.txt", path.join("link")).unwrap();
            }
        }
    });
    assert!(matches!(twin.commit("all kinds", true), Outcome::Done(_)));
    assert!(!git(&twin.native, &["ls-files"]).contains("new-untracked.txt"));
    twin.both(|path| {
        fs::remove_file(path.join("dir/sub/nested.txt")).unwrap();
        fs::remove_file(path.join("crlf.txt")).unwrap();
    });
    assert!(matches!(twin.commit("deletions", true), Outcome::Done(_)));
    assert!(!git(&twin.native, &["ls-files"]).contains("nested.txt"));
}

#[test]
fn all_with_file_mode_ignored_has_nothing_to_commit_like_git() {
    hermetic!();
    let twin = twin(|path| {
        seeded(path);
        git(path, &["config", "core.fileMode", "false"]);
    });
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            twin.both(|path| {
                fs::set_permissions(
                    path.join("tracked.txt"),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            });
        }
    }
    assert_eq!(
        twin.commit("mode only", true),
        Outcome::Failed(ErrorCode::GitCommandFailed)
    );
}

#[test]
fn all_on_an_unborn_branch_commits_what_is_staged_like_git() {
    hermetic!();
    let twin = twin(unborn);
    twin.both(|path| {
        write(path, "a.txt", "a\n");
        git(path, &["add", "a.txt"]);
        write(path, "untracked.txt", "u\n");
    });
    assert!(matches!(
        twin.commit("first, with -a", true),
        Outcome::Done(_)
    ));
}

#[test]
fn a_held_branch_lock_fails_like_git_and_leaves_the_same_objects() {
    hermetic!();
    let twin = twin(seeded);
    twin.both(|path| {
        write(path, "tracked.txt", "locked out\n");
        git(path, &["add", "tracked.txt"]);
        fs::write(path.join(".git/refs/heads/main.lock"), "").unwrap();
    });
    assert_eq!(
        twin.commit("locked", false),
        Outcome::Failed(ErrorCode::GitCommandFailed)
    );
    twin.both(|path| fs::remove_file(path.join(".git/refs/heads/main.lock")).unwrap());
    assert!(matches!(twin.commit("unlocked", false), Outcome::Done(_)));
}

#[test]
fn a_held_tag_lock_fails_like_git() {
    hermetic!();
    let twin = twin(seeded);
    twin.both(|path| {
        fs::create_dir_all(path.join(".git/refs/tags")).unwrap();
        fs::write(path.join(".git/refs/tags/locked.lock"), "").unwrap();
    });
    for message in [None, Some("note")] {
        assert_eq!(
            twin.tag_create("locked", message),
            Outcome::Failed(ErrorCode::GitCommandFailed)
        );
    }
    twin.both(|path| fs::remove_file(path.join(".git/refs/tags/locked.lock")).unwrap());
    assert!(matches!(twin.tag_create("locked", None), Outcome::Done(_)));
}

#[test]
fn a_branch_that_moves_after_the_plan_is_refused_without_moving_it_back() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let repo = route::open(&lab.path).unwrap();
    let plan = route::plan_commit(&lab.backend.environment, &repo, "late", false, false)
        .unwrap_or_else(|reason| panic!("plan refused: {reason:?}"));
    git(
        &lab.path,
        &["commit", "-q", "--allow-empty", "-m", "raced ahead"],
    );
    let raced = lab.head();
    let error = exec::commit(&repo, plan).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert_eq!(lab.head(), raced, "the racing commit stays");
}

#[test]
fn an_index_lock_taken_after_the_plan_is_refused() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let repo = route::open(&lab.path).unwrap();
    let plan = route::plan_commit(&lab.backend.environment, &repo, "late", false, false)
        .unwrap_or_else(|reason| panic!("plan refused: {reason:?}"));
    fs::write(lab.path.join(".git/index.lock"), "").unwrap();
    let before = lab.head();
    assert_eq!(
        exec::commit(&repo, plan).unwrap_err().code,
        ErrorCode::GitCommandFailed
    );
    assert_eq!(lab.head(), before);
}

/// What the tag `name` points at, or `None` when there is no such tag.
fn tag_target(path: &std::path::Path, name: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--verify", "-q", &format!("refs/tags/{name}")])
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[test]
fn a_tag_created_after_the_early_check_is_not_overwritten() {
    hermetic!();
    for message in [None, Some("annotated")] {
        let lab = lab();
        let repo = route::open(&lab.path).unwrap();
        let plan = route::plan_tag_create(&lab.backend.environment, &repo, "raced", message, false)
            .unwrap_or_else(|reason| panic!("plan refused: {reason:?}"));
        let head = lab.head();
        let mut raced_to = None;
        let error = exec::tag_create_observed(&repo, "raced", plan, &mut |stage| {
            if stage == exec::TagStage::Checked {
                // Another process makes the tag after native saw there was none.
                git(
                    &lab.path,
                    &["commit", "-q", "--allow-empty", "-m", "elsewhere"],
                );
                git(&lab.path, &["tag", "raced"]);
                raced_to = tag_target(&lab.path, "raced");
                git(&lab.path, &["reset", "-q", "--hard", &head]);
            }
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::GitCommandFailed, "{message:?}");
        assert!(raced_to.is_some());
        assert_eq!(
            tag_target(&lab.path, "raced"),
            raced_to,
            "{message:?}: the tag Git made stays"
        );
        assert_ne!(tag_target(&lab.path, "raced").unwrap(), head);
    }
}

#[test]
fn the_existence_check_and_the_write_happen_under_the_reference_lock() {
    hermetic!();
    for message in [None, Some("annotated")] {
        let lab = lab();
        let repo = route::open(&lab.path).unwrap();
        let plan = route::plan_tag_create(&lab.backend.environment, &repo, "held", message, false)
            .unwrap_or_else(|reason| panic!("plan refused: {reason:?}"));
        let mut tried = false;
        exec::tag_create_observed(&repo, "held", plan, &mut |stage| {
            if stage == exec::TagStage::Locked {
                // Between native's check under the lock and its write, Git cannot make the tag.
                let refused = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&lab.path)
                    .args(["tag", "held"])
                    .output()
                    .unwrap();
                assert!(
                    !refused.status.success(),
                    "{message:?}: Git tagged under native's lock"
                );
                assert!(
                    String::from_utf8_lossy(&refused.stderr).contains("held.lock"),
                    "{message:?}: {}",
                    String::from_utf8_lossy(&refused.stderr)
                );
                tried = true;
            }
        })
        .unwrap();
        assert!(tried, "{message:?}");
        assert!(tag_target(&lab.path, "held").is_some(), "{message:?}");
    }
}

#[test]
fn author_and_committer_share_one_now() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    // No explicit dates, so both identities take "now" from the clock.
    let environment = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", "1")]);
    let repo = route::open(&lab.path).unwrap();
    let plan = route::plan_commit(&environment, &repo, "one clock", false, false)
        .unwrap_or_else(|reason| panic!("plan refused: {reason:?}"));
    assert!(plan.author.seconds.is_none() && plan.committer.seconds.is_none());
    // A clock that has moved on a second by every reading.
    let mut readings = 0;
    let mut clock = || {
        readings += 1;
        Ok(git2::Time::new(1_000_000_000 + readings, 0))
    };
    let oid = exec::commit_at(&repo, plan, &mut clock).unwrap();
    assert_eq!(readings, 1, "the clock is read once");
    let commit = repo.find_commit(oid).unwrap();
    assert_eq!(commit.author().when(), commit.committer().when());
    assert_eq!(commit.author().when().seconds(), 1_000_000_001);
}
