//! gwz 1.0.19 hotfix, round 4 rows (the narrowed hotfix: `--force` deletes no untracked
//! file): one per finding of the round-4 Code (R4-Fn) and State (S4-Fn) reviews, same
//! fixture as the parent rows.
use std::fs;
use std::path::Path;

use crate::SyncBehavior;
use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;

use super::round1::{commit_change, push_root_commit, typechange_workspace};
use super::*;

/// A workspace pulled once, so the member tracks `from/a.txt`; then the member's remote
/// replaces the directory `from/` with the file `to` (a case variant of it when the
/// names differ only by case), and the root's remote gains a commit.
fn directory_becomes_file(from: &str, to: &str) -> PullWorkspace {
    let mut ws = pull_workspace(&[(&format!("{from}/a.txt"), "a\n")], &[]);
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_change(&source, &[(to, "file\n")], &[from], "dir becomes a file");
    Git2Backend::new()
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer-link", "root-link.txt");
    ws
}

/// With the untracked entry in place the pull refuses, with and without `--force`;
/// once the user deletes it the pull lands and the other untracked file is kept.
fn assert_refuses_then_lands_after_delete(
    ws: &PullWorkspace,
    collision: &str,
    untracked: &Path,
    landed: (&Path, &str),
) {
    let before = snapshot(ws);

    assert_refused_with_and_without_force(ws, SyncBehavior::FfOnly, &[collision], &before);

    fs::remove_file(untracked).unwrap();
    pull(ws, SyncBehavior::FfOnly, false).unwrap_or_else(|error| panic!("{}", error.message));

    assert_eq!(head(&ws.member), ws.member_after, "member did not move");
    assert_eq!(head(&ws.root), ws.root_after, "root did not move");
    assert_eq!(fs::read_to_string(landed.0).unwrap(), landed.1);
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "junk\n"
    );
}

// R4-F1 (P2): a hard link is never proof that an entry is the tracked file the update
// removes. An untracked hard link of a tracked file, inside the tracked directory the
// update turns into a file, collides (round 3 deleted it in a plain pull).
#[test]
fn row31_untracked_hard_link_in_a_directory_becoming_a_file_collides() {
    let ws = typechange_workspace();
    fs::hard_link(
        ws.member.join("conf/a.txt"),
        ws.member.join("conf/copy.txt"),
    )
    .unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();

    assert_refuses_then_lands_after_delete(
        &ws,
        "conf",
        &ws.member.join("conf/copy.txt"),
        (&ws.member.join("conf"), "file\n"),
    );
}

// R4-F1 (P2), the case-variant spelling: the incoming `Conf` replaces the tracked
// `conf/`; the filesystem resolves `Conf` to that directory, whose hard link is still
// an untracked entry. Only where the filesystem folds case.
#[test]
fn row31b_untracked_hard_link_in_a_directory_becoming_a_case_variant_file_collides() {
    let probe = pull_workspace(&[("x.txt", "x\n")], &[]);
    if !filesystem_is_case_insensitive(&probe.member) {
        eprintln!("row31b: NOT APPLICABLE on this case-sensitive filesystem");
        return;
    }
    let ws = directory_becomes_file("conf", "Conf");
    fs::hard_link(
        ws.member.join("conf/a.txt"),
        ws.member.join("conf/copy.txt"),
    )
    .unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();

    assert_refuses_then_lands_after_delete(
        &ws,
        "Conf",
        &ws.member.join("conf/copy.txt"),
        (&ws.member.join("Conf"), "file\n"),
    );
}

// R4-F1 (P2), the fail-closed side: an untracked hard link `b.txt` of the tracked
// `README.md`, which the update removes while it adds `b.txt`, is an ordinary
// collision (round 3 refused it as unidentifiable, advising `--force`).
#[test]
fn row31c_untracked_hard_link_of_a_removed_tracked_file_is_an_ordinary_collision() {
    let mut ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_change(&source, &[("b.txt", "new\n")], &["README.md"], "rename");
    Git2Backend::new()
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer-link", "root-link.txt");
    fs::hard_link(ws.member.join("README.md"), ws.member.join("b.txt")).unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();

    assert_refuses_then_lands_after_delete(
        &ws,
        "b.txt",
        &ws.member.join("b.txt"),
        (&ws.member.join("b.txt"), "new\n"),
    );
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "macos")] {
        use super::round3::MODES;

        const NFC_DIRECTORY: &str = "caf\u{e9}";
        const NFD_DIRECTORY: &str = "cafe\u{301}";

        /// A member pulled once tracking `café/a.txt` (NFC in the index) whose directory
        /// is spelled NFD on disk (as HFS+ kept it), then upstream adds `café/b.txt`.
        fn nfd_directory_workspace() -> Option<PullWorkspace> {
            let mut ws = pull_workspace(&[(&format!("{NFC_DIRECTORY}/a.txt"), "a\n")], &[]);
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            fs::rename(
                ws.member.join(NFC_DIRECTORY),
                ws.member.join(NFD_DIRECTORY),
            )
            .unwrap();
            let names = fs::read_dir(&ws.member)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if !names.iter().any(|name| name == NFD_DIRECTORY) {
                return None;
            }
            let mut config = git2::Repository::open(&ws.member)
                .unwrap()
                .config()
                .unwrap();
            config.set_bool("core.precomposeunicode", true).unwrap();
            config.set_bool("core.ignorecase", true).unwrap();
            assert!(
                !Git2Backend::new().status(&ws.member).unwrap().has_tracked_changes(),
                "fixture: the NFD directory must look clean"
            );
            let source = ws._member_fixture.source.clone();
            ws.member_before = ws.member_after.clone();
            ws.member_after = commit_change(
                &source,
                &[(&format!("{NFC_DIRECTORY}/b.txt"), "b\n")],
                &[],
                "add b",
            );
            Git2Backend::new()
                .push(&source, "origin", "refs/heads/main:refs/heads/main")
                .unwrap();
            ws.root_before = ws.root_after.clone();
            ws.root_after = push_root_commit(&ws, "peer-nfd", "root-nfd.txt");
            Some(ws)
        }

        // S4-F3 (P2): a tracked directory spelled NFD on disk while the index holds NFC
        // (macOS repositories from HFS+) takes an incoming file in a clean pull, as
        // libgit2 (which composes names under `core.precomposeunicode`), 1.0.18 and git
        // do, in every mode, with and without `--force`; an untracked file in it stays.
        #[test]
        fn row32_a_tracked_directory_spelled_nfd_on_disk_takes_an_incoming_file() {
            for sync in MODES {
                for force in [false, true] {
                    let Some(ws) = nfd_directory_workspace() else {
                        eprintln!("row32: NOT APPLICABLE: the filesystem normalizes names");
                        return;
                    };
                    fs::write(ws.member.join(NFD_DIRECTORY).join("local.txt"), "mine\n").unwrap();

                    pull(&ws, sync, force).unwrap_or_else(|error| {
                        panic!("{sync:?} force={force}: {}", error.message)
                    });

                    assert_eq!(head(&ws.member), ws.member_after, "{sync:?}");
                    assert_eq!(head(&ws.root), ws.root_after, "{sync:?}");
                    assert_eq!(
                        fs::read_to_string(ws.member.join(NFC_DIRECTORY).join("b.txt")).unwrap(),
                        "b\n"
                    );
                    assert_eq!(
                        fs::read_to_string(ws.member.join(NFC_DIRECTORY).join("local.txt"))
                            .unwrap(),
                        "mine\n"
                    );
                    assert!(
                        !Git2Backend::new()
                            .status(&ws.member)
                            .unwrap()
                            .has_tracked_changes(),
                        "{sync:?}: the pull left tracked changes"
                    );
                }
            }
        }
    } else {
        #[test]
        fn row32_a_tracked_directory_spelled_nfd_on_disk_takes_an_incoming_file() {
            eprintln!("row32: NOT APPLICABLE: only macOS keeps NFD names beside NFC index paths");
        }
    }
}

/// Commit on HEAD of `repo_path` a tree that also holds `alias` (a blob), built from the
/// tree directly: on a case-folding filesystem the worktree cannot hold both names.
fn commit_case_alias(repo_path: &Path, alias: &str) -> String {
    let repo = git2::Repository::open(repo_path).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let blob = repo.blob(b"alias\n").unwrap();
    let mut builder = repo.treebuilder(Some(&parent.tree().unwrap())).unwrap();
    builder.insert(alias, blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "alias",
        &tree,
        &[&parent],
    )
    .unwrap()
    .to_string()
}

// S4-F2 (P2): the incoming `foo.txt` resolves, on a case-folding filesystem, to the
// tracked `Foo.txt` that the update keeps. The refusal names both, never advises
// `--force`, and nothing is deleted or moved (round 3 advised `--force`, which deleted
// the tracked file and then failed after the root and the member had moved).
#[test]
fn row33_a_case_alias_of_a_tracked_entry_the_update_keeps_refuses_naming_both() {
    let mut ws = pull_workspace(&[("Foo.txt", "foo\n")], &[]);
    if !filesystem_is_case_insensitive(&ws.member) {
        eprintln!("row33: NOT APPLICABLE on this case-sensitive filesystem");
        return;
    }
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_case_alias(&source, "foo.txt");
    Git2Backend::new()
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer-alias", "root-alias.txt");
    let before = snapshot(&ws);

    for sync in [SyncBehavior::FfOnly, SyncBehavior::Merge] {
        for force in [false, true] {
            let error = pull(&ws, sync, force).unwrap_err();

            assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
            assert!(
                error.message.contains("foo.txt")
                    && error.message.contains("Foo.txt")
                    && !error.message.contains("--force"),
                "{}",
                error.message
            );
            assert_untouched(&ws, &before);
            assert_eq!(
                fs::read_to_string(ws.member.join("Foo.txt")).unwrap(),
                "foo\n",
                "the tracked file was removed"
            );
        }
    }
}
