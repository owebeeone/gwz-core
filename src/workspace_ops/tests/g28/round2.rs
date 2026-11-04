//! gwz 1.0.19 hotfix, remediation round 2 rows: one per finding of the round-2 Code
//! (R2-Fn) and State (S2-Fn) reviews, same fixture as the parent rows.
use std::fs;

use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;

use super::round1::{commit_change, push_root_commit};
use super::*;

pub(super) fn set_ignore_case(repo: &Path) {
    git2::Repository::open(repo)
        .unwrap()
        .config()
        .unwrap()
        .set_bool("core.ignorecase", true)
        .unwrap();
}

// R2-F2 (P3): a conflict libgit2 reports is never dropped. On a case-insensitive
// filesystem the untracked `notes.md` is the entry in the way of `Notes.md`: refused,
// with and without `--force`, and the pull lands once it is deleted. Under
// `core.ignorecase` on a case-sensitive one (the filesystem is the authority, so gwz
// does not guess which entry libgit2 matched) it refuses before anything moves, naming
// `core.ignorecase`.
#[test]
fn row20_case_variant_under_ignorecase_refuses_on_every_filesystem() {
    let ws = pull_workspace(&[("Notes.md", "incoming\n")], &[]);
    set_ignore_case(&ws.member);
    fs::write(ws.member.join("notes.md"), "mine\n").unwrap();
    let insensitive = filesystem_is_case_insensitive(&ws.member);
    eprintln!("row20: case-insensitive filesystem: {insensitive}");
    let before = snapshot(&ws);

    if !insensitive {
        for force in [false, true] {
            let error = pull(&ws, crate::SyncBehavior::FfOnly, force).unwrap_err();
            assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
            assert!(
                error.message.contains("Notes.md")
                    && error.message.contains("core.ignorecase")
                    && !error.message.contains("--force"),
                "{}",
                error.message
            );
            assert_untouched(&ws, &before);
        }
        return;
    }
    assert_refused_with_and_without_force(&ws, crate::SyncBehavior::FfOnly, &["Notes.md"], &before);

    fs::remove_file(ws.member.join("notes.md")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false)
        .unwrap_or_else(|error| panic!("{}", error.message));

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("Notes.md")).unwrap(),
        "incoming\n"
    );
}

// R2-F3 (P3): `--sync rebase` judges the local commits it replays too. An untracked
// file at a path a replayed local commit writes (and a later one deletes) refuses at
// plan time, with or without `--force`, before the root or the member moves.
#[test]
fn row21_rebase_refuses_an_untracked_file_in_the_way_of_a_replayed_commit() {
    let backend = Git2Backend::new();
    let mut ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    let source = ws._member_fixture.source.clone();
    commit_change(&source, &[("remote-two.txt", "r2\n")], &[], "remote two");
    backend
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    commit_change(&ws.member, &[("x.txt", "x\n")], &[], "add x");
    ws.member_before = commit_change(&ws.member, &[], &["x.txt"], "drop x");
    ws.root_before = head(&ws.root);
    push_root_commit(&ws, "peer-rebase", "root-rebase.txt");
    fs::write(ws.member.join("x.txt"), "my x\n").unwrap();
    let before = snapshot(&ws);

    for force in [false, true] {
        let error = pull(&ws, crate::SyncBehavior::Rebase, force).unwrap_err();

        assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
        assert!(
            error.message.contains("x.txt")
                && error.message.contains("replays")
                && error.message.contains("move or delete")
                && !error.message.contains("--force"),
            "{}",
            error.message
        );
        assert_untouched(&ws, &before);
    }
}

// S2-F1 (P1): a tracked directory the update replaces with a file whose name differs
// only in case (`Conf/` becomes `conf`) keeps untracked content inside it: the pull
// refuses before anything moves, with and without `--force`; once the user deletes that
// content the update lands. On a case-sensitive filesystem the row runs under
// `core.ignorecase`, where libgit2 matches the names the same way.
#[test]
fn row22_case_variant_directory_becoming_a_file_keeps_untracked_content() {
    let mut ws = pull_workspace(&[("Conf/a.txt", "a\n")], &[]);
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
    let insensitive = filesystem_is_case_insensitive(&ws.member);
    eprintln!("row22: case-insensitive filesystem: {insensitive}");
    if !insensitive {
        set_ignore_case(&ws.member);
    }
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_change(&source, &[("conf", "file\n")], &["Conf"], "conf is a file");
    Git2Backend::new()
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer-case", "root-case.txt");
    fs::write(ws.member.join("Conf/local.txt"), "mine\n").unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(&ws, crate::SyncBehavior::FfOnly, &["conf"], &before);

    fs::remove_file(ws.member.join("Conf/local.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false)
        .unwrap_or_else(|error| panic!("{}", error.message));

    assert_eq!(head(&ws.member), ws.member_after, "member did not move");
    assert_eq!(head(&ws.root), ws.root_after, "root did not move");
    assert_eq!(
        fs::read_to_string(ws.member.join("conf")).unwrap(),
        "file\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "junk\n"
    );
}
