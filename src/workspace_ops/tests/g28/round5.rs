//! gwz 1.0.19 hotfix, round 6 rows (no deletion at all): pull deletes and overwrites
//! nothing of its own, so an entry in the update's way refuses whether it is untracked or
//! ignored, a file, a directory or a symlink, whatever its case or normalization
//! spelling, with and without `--force`, naming the path, before any participant moves,
//! and the pull lands once the user has moved or deleted it. One row per shape of the
//! removed ignored-entry replacement (rows 17 to 34 of rounds 1 to 5) and one per
//! counterexample of the round-5 State review (F1 to F3). Same fixture as the parent
//! rows; real Git only.
use std::fs;
use std::path::{Path, PathBuf};

use crate::SyncBehavior;
use crate::git::Git2Backend;
use crate::model::ErrorCode;

use super::round1::typechange_workspace;
use super::round3::{assert_rebased, diverged_member};
use super::*;

/// With the entry in place the pull refuses with and without `--force`, naming `named`,
/// and leaves the workspace as `before`; then `clear` (the user moving or deleting the
/// entry) and the pull lands in the root and the member.
fn assert_refuses_then_lands(
    ws: &PullWorkspace,
    sync: SyncBehavior,
    named: &[&str],
    before: &Snapshot,
    clear: impl FnOnce(),
) {
    assert_refused_with_and_without_force(ws, sync, named, before);
    clear();
    pull(ws, sync, false).unwrap_or_else(|error| panic!("{sync:?}: {}", error.message));
    assert_eq!(head(&ws.member), ws.member_after, "member did not move");
    assert_eq!(head(&ws.root), ws.root_after, "root did not move");
}

/// Remove a file, a directory or a link (a junction is removed as a directory).
fn remove_entry(path: &Path) {
    let meta = fs::symlink_metadata(path).unwrap();
    if meta.file_type().is_symlink() {
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .unwrap();
    } else if meta.is_dir() {
        fs::remove_dir_all(path).unwrap();
    } else {
        fs::remove_file(path).unwrap();
    }
}

fn set_config_bool(repo: &Path, key: &str, value: bool) {
    git2::Repository::open(repo)
        .unwrap()
        .config()
        .unwrap()
        .set_bool(key, value)
        .unwrap();
}

/// A shape of an entry in the update's way: the workspace with it in place, the incoming
/// path the refusal names, and the entry the user deletes.
struct InTheWay {
    tag: &'static str,
    ws: PullWorkspace,
    named: &'static str,
    entry: PathBuf,
}

fn in_the_way(tag: &'static str, ws: PullWorkspace, named: &'static str, entry: &str) -> InTheWay {
    let entry = ws.member.join(entry);
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();
    InTheWay {
        tag,
        ws,
        named,
        entry,
    }
}

fn ignored_shapes() -> Vec<InTheWay> {
    let mut shapes = Vec::new();
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    exclude(&ws.member, "new.txt\n");
    fs::write(ws.member.join("new.txt"), "mine\n").unwrap();
    shapes.push(in_the_way(
        "an ignored file at an added path",
        ws,
        "new.txt",
        "new.txt",
    ));

    let ws = pull_workspace(&[("blocked.txt", "incoming\n")], &[]);
    exclude(&ws.member, "*.log\n");
    fs::create_dir_all(ws.member.join("blocked.txt")).unwrap();
    fs::write(ws.member.join("blocked.txt/a.log"), "log\n").unwrap();
    shapes.push(in_the_way(
        "a directory holding only ignored content at an added file's path",
        ws,
        "blocked.txt",
        "blocked.txt",
    ));

    let ws = pull_workspace(&[("sub/inner.txt", "incoming\n")], &[]);
    exclude(&ws.member, "sub\n");
    fs::write(ws.member.join("sub"), "a file where a directory comes\n").unwrap();
    shapes.push(in_the_way(
        "an ignored file at a parent path of an added file",
        ws,
        "sub/inner.txt",
        "sub",
    ));

    let ws = pull_workspace(&[("blocked.txt", "incoming\n")], &[]);
    fs::create_dir_all(ws.member.join("blocked.txt")).unwrap();
    shapes.push(in_the_way(
        "an empty directory at an added file's path",
        ws,
        "blocked.txt",
        "blocked.txt",
    ));

    let ws = typechange_workspace();
    exclude(&ws.member, "*.log\n");
    fs::write(ws.member.join("conf/build.log"), "log\n").unwrap();
    shapes.push(in_the_way(
        "ignored content in a tracked directory the update turns into a file",
        ws,
        "conf",
        "conf/build.log",
    ));
    shapes
}

// Row 17 (B2, replaces the ignored-only replacement of rows 17 and 37): an ignored entry
// in the update's way refuses like an untracked one, whatever its shape, with and
// without `--force`; the pull lands once the user deletes it, and an untracked file that
// is not in the way is still there.
#[test]
fn row17_ignored_entries_in_the_way_refuse_like_untracked_ones() {
    for shape in ignored_shapes() {
        eprintln!("row17: {}", shape.tag);
        let before = snapshot(&shape.ws);

        assert_refuses_then_lands(
            &shape.ws,
            SyncBehavior::FfOnly,
            &[shape.named],
            &before,
            || remove_entry(&shape.entry),
        );

        assert_eq!(
            fs::read_to_string(shape.ws.member.join("junk.txt")).unwrap(),
            "junk\n",
            "{}",
            shape.tag
        );
    }
}

// Row 17b: the same in every integrating mode, in the root as in a member.
#[test]
fn row17b_an_ignored_entry_in_the_way_refuses_in_every_mode_in_the_root_and_a_member() {
    let modes = [
        SyncBehavior::FfOnly,
        SyncBehavior::Merge,
        SyncBehavior::Rebase,
        SyncBehavior::DriverSelected,
    ];
    for sync in modes {
        for in_root in [false, true] {
            let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
            let (repo, named) = if in_root {
                (ws.root.clone(), "root-new.txt")
            } else {
                (ws.member.clone(), "new.txt")
            };
            exclude(&repo, &format!("{named}\n"));
            fs::write(repo.join(named), "mine\n").unwrap();
            let before = snapshot(&ws);

            assert_refuses_then_lands(&ws, sync, &[named], &before, || {
                fs::remove_file(repo.join(named)).unwrap();
            });
        }
    }
}

// Row 17c (B7): `--dry-run` reports the refusal for an ignored entry, with and without
// `--force`, in a member and in the root, and writes nothing.
#[test]
fn row17c_dry_run_refuses_an_ignored_entry_and_mutates_nothing() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    exclude(&ws.member, "new.txt\n");
    fs::write(ws.member.join("new.txt"), "mine\n").unwrap();
    exclude(&ws.root, "root-new.txt\n");
    fs::write(ws.root.join("root-new.txt"), "root mine\n").unwrap();
    let dry_run = |force: bool| {
        handle_pull_head(
            &Git2Backend::new(),
            &ws.root,
            pull_request(SyncBehavior::FfOnly, force, true),
            "op_pull_dry",
        )
        .unwrap_err()
    };
    for (named, gone) in [
        ("root-new.txt", ws.root.join("root-new.txt")),
        ("new.txt", ws.member.join("new.txt")),
    ] {
        let before = snapshot(&ws);
        for force in [false, true] {
            assert_collision_refusal(&dry_run(force), &[named]);
            assert_untouched(&ws, &before);
        }
        fs::remove_file(gone).unwrap();
    }
    let before = snapshot(&ws);
    handle_pull_head(
        &Git2Backend::new(),
        &ws.root,
        pull_request(SyncBehavior::FfOnly, false, true),
        "op_pull_dry",
    )
    .unwrap();
    assert_untouched(&ws, &before);
}

/// An ignored directory `blocked.txt` holding `a.log`, in the way of the incoming file.
fn blocked_by_ignored_log() -> (PullWorkspace, PathBuf) {
    let ws = pull_workspace(&[("blocked.txt", "blocked\n")], &[]);
    exclude(&ws.member, "*.log\n");
    fs::create_dir_all(ws.member.join("blocked.txt")).unwrap();
    let log = ws.member.join("blocked.txt/a.log");
    fs::write(&log, "log\n").unwrap();
    (ws, log)
}

// Row 19 (B5, replaces the consent re-check): an ignored entry that appears after the
// plan is refused where the pull next looks, and nothing is replaced. Before the barrier
// it refuses the whole pull with the root unmoved; after the barrier, in the member's own
// update under the branch lock, the member does not move and the entry keeps its content.
#[test]
fn row19_an_ignored_entry_appearing_after_planning_refuses_and_is_kept() {
    let (ws, log) = blocked_by_ignored_log();
    fs::remove_dir_all(ws.member.join("blocked.txt")).unwrap();
    let late = log.clone();
    crate::workspace_ops::pull_head_barrier::before_next_pull_barrier(move || {
        fs::create_dir_all(late.parent().unwrap()).unwrap();
        fs::write(late, "precious\n").unwrap();
    });

    let error = pull(&ws, SyncBehavior::FfOnly, false).unwrap_err();

    assert_collision_refusal(&error, &["blocked.txt"]);
    assert_eq!(fs::read_to_string(&log).unwrap(), "precious\n");
    assert_eq!(head(&ws.root), ws.root_before, "root moved on a refusal");
    assert_eq!(head(&ws.member), ws.member_before);

    let (ws, log) = blocked_by_ignored_log();
    fs::remove_dir_all(ws.member.join("blocked.txt")).unwrap();
    let late = log.clone();
    Git2Backend::before_next_prepared_execution(move || {
        fs::create_dir_all(late.parent().unwrap()).unwrap();
        fs::write(late, "precious\n").unwrap();
    });

    let error = pull(&ws, SyncBehavior::FfOnly, true).unwrap_err();

    assert_collision_refusal(&error, &["blocked.txt"]);
    assert_eq!(fs::read_to_string(&log).unwrap(), "precious\n");
    assert_eq!(
        head(&ws.member),
        ws.member_before,
        "member moved on a refusal"
    );
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        // Row 23, Windows: an ignored file that another process holds open without
        // FILE_SHARE_DELETE is in the way like any other: refused before anything moves.
        #[test]
        fn row23_an_ignored_blocker_held_open_without_delete_sharing_refuses() {
            use std::os::windows::fs::OpenOptionsExt;
            let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
            exclude(&ws.member, "new.txt\n");
            let file = ws.member.join("new.txt");
            fs::write(&file, "local\n").unwrap();
            // FILE_SHARE_READ | FILE_SHARE_WRITE, no FILE_SHARE_DELETE.
            let handle = fs::OpenOptions::new()
                .read(true)
                .share_mode(0x1 | 0x2)
                .open(&file)
                .unwrap();
            let before = snapshot(&ws);

            assert_refused_with_and_without_force(
                &ws,
                SyncBehavior::FfOnly,
                &["new.txt"],
                &before,
            );

            drop(handle);
            fs::remove_file(&file).unwrap();
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
        }
    } else {
        /// Restores the permissions of a directory when dropped, so a failing row does
        /// not leave a directory its temporary root cannot delete.
        struct RestoreMode(PathBuf);

        impl RestoreMode {
            fn set(path: &Path, mode: u32) -> Self {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
                Self(path.to_path_buf())
            }
        }

        impl Drop for RestoreMode {
            fn drop(&mut self) {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
        }

        // Row 23, Unix: an ignored directory its owner may not write (mode 0575) is in
        // the way like any other: refused before anything moves, whoever could delete it.
        #[test]
        fn row23_an_ignored_blocker_in_a_directory_this_user_cannot_write_refuses() {
            let ws = pull_workspace(&[("blocked.txt", "blocked\n")], &[]);
            exclude(&ws.member, "blocked.txt\n");
            let directory = ws.member.join("blocked.txt");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("mine.txt"), "mine\n").unwrap();
            let before = snapshot(&ws);
            let restore = RestoreMode::set(&directory, 0o575);

            assert_refused_with_and_without_force(
                &ws,
                SyncBehavior::FfOnly,
                &["blocked.txt"],
                &before,
            );

            drop(restore);
            fs::remove_dir_all(&directory).unwrap();
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
        }

        /// Pull with `dir` set to `mode` (so its content may be unreadable), with and
        /// without `--force`: it refuses naming `named` before anything moves, and once
        /// the mode is restored and the user deletes `entry` the pull lands. `hidden` is
        /// a child of `dir` this process must not be able to see under `mode`: when it
        /// still can (root), the row does not apply.
        fn assert_refuses_under_mode(
            ws: &PullWorkspace,
            entry: &Path,
            dir: &Path,
            mode: u32,
            hidden: Option<&Path>,
            named: &str,
        ) {
            let before = snapshot(ws);
            let restore = RestoreMode::set(dir, mode);
            if hidden.is_some_and(|child| fs::symlink_metadata(child).is_ok()) {
                eprintln!("NOT APPLICABLE (mode {mode:o}): this process may search any directory");
                return;
            }
            let errors = [false, true].map(|force| pull(ws, SyncBehavior::FfOnly, force).unwrap_err());
            drop(restore);
            for error in &errors {
                assert_collision_refusal(error, &[named]);
            }
            assert_untouched(ws, &before);
            fs::remove_dir_all(entry).unwrap();
            pull(ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(ws);
        }

        // Row 26: an ignored blocker directory this user may read but not search (0600,
        // 0644, at the top or nested) cannot be inspected: the pull refuses at plan
        // time, naming it, instead of failing inside the member update after the root
        // moved.
        #[test]
        fn row26_an_ignored_blocker_directory_this_user_cannot_search_refuses() {
            for (nested, mode) in [(false, 0o600), (false, 0o644), (true, 0o644)] {
                let ws = pull_workspace(&[("blocked.txt", "blocked\n")], &[]);
                exclude(&ws.member, "blocked.txt\n");
                let top = ws.member.join("blocked.txt");
                let dir = if nested { top.join("sub") } else { top.clone() };
                fs::create_dir_all(&dir).unwrap();
                let child = dir.join("mine.txt");
                fs::write(&child, "mine\n").unwrap();
                eprintln!("row26: nested={nested} mode={mode:o}");
                assert_refuses_under_mode(&ws, &top, &dir, mode, Some(&child), "blocked.txt");
            }
        }

        // Row 27: an ignored directory at an incoming path that this user cannot search
        // (0644) or write (0555) refuses at plan time, without `--force`, instead of
        // failing mid-checkout.
        #[test]
        fn row27_unsearchable_or_unwritable_ignored_directory_in_the_way_refuses() {
            for mode in [0o644, 0o555] {
                let ws = pull_workspace(&[("build", "incoming build file\n")], &[]);
                exclude(&ws.member, "build\n");
                let dir = ws.member.join("build");
                fs::create_dir_all(&dir).unwrap();
                let child = dir.join("out.o");
                fs::write(&child, "object\n").unwrap();
                eprintln!("row27: mode={mode:o}");
                let hidden = (mode == 0o644).then_some(child.as_path());
                assert_refuses_under_mode(&ws, &dir, &dir, mode, hidden, "build");
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "macos")] {
        // Row 23b, macOS: a user-immutable (`chflags uchg`) ignored file in the way is in
        // the way like any other: refused before anything moves.
        #[test]
        fn row23b_an_ignored_immutable_blocker_refuses() {
            let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
            exclude(&ws.member, "new.txt\n");
            let file = ws.member.join("new.txt");
            fs::write(&file, "local\n").unwrap();
            let chflags = |flag: &str| {
                assert!(
                    std::process::Command::new("chflags")
                        .arg(flag)
                        .arg(&file)
                        .status()
                        .unwrap()
                        .success()
                );
            };
            chflags("uchg");
            let before = snapshot(&ws);

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_refused_with_and_without_force(
                    &ws,
                    SyncBehavior::FfOnly,
                    &["new.txt"],
                    &before,
                );
            }));
            chflags("nouchg");
            if let Err(panic) = result {
                std::panic::resume_unwind(panic);
            }
            fs::remove_file(&file).unwrap();
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
        }

        fn chmod_acl(path: &Path, args: &[&str]) {
            let status = std::process::Command::new("chmod")
                .args(args)
                .arg(path)
                .status()
                .unwrap();
            assert!(status.success(), "chmod {args:?} {path:?} failed");
        }

        /// Takes an ACL off a path when dropped.
        struct RemoveAcl(PathBuf);

        impl Drop for RemoveAcl {
            fn drop(&mut self) {
                let _ = std::process::Command::new("chmod")
                    .arg("-N")
                    .arg(&self.0)
                    .status();
            }
        }

        // Row 34, macOS: an ACL that forbids deleting an ignored entry in the way
        // (`deny delete_child` on its directory, `deny delete` on a file inside it or on
        // the entry itself) changes nothing: the entry is in the way, so the pull refuses
        // before the root moves, and lands once the user has removed the ACL and the entry.
        #[test]
        fn row34_an_ignored_entry_with_an_acl_in_the_way_refuses() {
            // (what is in the way, the entry the ACL goes on, the ACL)
            let cases = [
                ("directory", "blocked.txt", "everyone deny delete_child"),
                ("file in directory", "blocked.txt/mine.txt", "everyone deny delete"),
                ("ignored file", "new.txt", "everyone deny delete"),
            ];
            for (tag, target, acl) in cases {
                let ws = pull_workspace(
                    &[("blocked.txt", "blocked\n"), ("new.txt", "incoming\n")],
                    &[],
                );
                exclude(&ws.member, "blocked.txt\nnew.txt\n");
                let named = if target == "new.txt" { "new.txt" } else { "blocked.txt" };
                if target == "new.txt" {
                    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
                } else {
                    fs::create_dir_all(ws.member.join("blocked.txt")).unwrap();
                    fs::write(ws.member.join("blocked.txt/mine.txt"), "mine\n").unwrap();
                }
                let before = snapshot(&ws);
                let acl_path = ws.member.join(target);
                chmod_acl(&acl_path, &["+a", acl]);
                let remove = RemoveAcl(acl_path);
                eprintln!("row34 [{tag}]: {acl} on {target}");

                assert_refused_with_and_without_force(&ws, SyncBehavior::FfOnly, &[named], &before);

                drop(remove);
                remove_entry(&ws.member.join(named));
                pull(&ws, SyncBehavior::FfOnly, false)
                    .unwrap_or_else(|error| panic!("[{tag}] {}", error.message));
                assert_pulled(&ws);
            }
        }
    }
}

// Row 27b: an ignored directory holding a repository at an incoming path refuses, with
// and without `--force`; the repository is untouched, and the pull lands once the user
// moves it away.
#[test]
fn row27b_ignored_directory_holding_a_repository_refuses() {
    let ws = pull_workspace(&[("build", "incoming build file\n")], &[]);
    exclude(&ws.member, "build\n");
    fs::create_dir_all(ws.member.join("build/inner")).unwrap();
    git2::Repository::init(ws.member.join("build/inner")).unwrap();
    fs::write(ws.member.join("build/out.o"), "object\n").unwrap();
    let before = snapshot(&ws);

    for force in [false, true] {
        let error = pull(&ws, SyncBehavior::FfOnly, force).unwrap_err();

        assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
        assert!(
            error.message.contains("repository")
                && error.message.contains("build")
                && !error.message.contains("--force"),
            "{}",
            error.message
        );
        assert_untouched(&ws, &before);
    }
    assert!(ws.member.join("build/inner/.git").exists());

    fs::remove_dir_all(ws.member.join("build")).unwrap();
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
}

// Row 28: an entry holding only ignored content in the way of a local commit that
// `--sync rebase` replays refuses at plan time, with and without `--force`, naming it and
// the replay, before the root or the member moves; the rebase lands once the user deletes it.
#[test]
fn row28_rebase_refuses_an_ignored_entry_in_the_way_of_a_replayed_commit() {
    let ws = diverged_member(&[("out", "x\n")], &[], &["out"], &[]);
    exclude(&ws.member, "*.log\n");
    fs::create_dir_all(ws.member.join("out")).unwrap();
    fs::write(ws.member.join("out/build.log"), "log\n").unwrap();
    fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();
    let before = snapshot(&ws);

    for force in [false, true] {
        let error = pull(&ws, SyncBehavior::Rebase, force).unwrap_err();

        assert_collision_refusal(&error, &["out"]);
        assert!(error.message.contains("replays"), "{}", error.message);
        assert_untouched(&ws, &before);
    }

    fs::remove_dir_all(ws.member.join("out")).unwrap();
    pull(&ws, SyncBehavior::Rebase, false).unwrap_or_else(|error| panic!("{}", error.message));

    assert_rebased(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "junk\n"
    );
}

// F1 (State round 5, P1): an ignore pattern that names the incoming spelling does not make
// an untracked entry that the filesystem folds onto it ignored. Where the filesystem folds
// case, a pull no longer deletes `ärger.txt` because `Ärger.txt` is excluded, in a member
// or in the root; it refuses naming `Ärger.txt`, keeps the file, and lands once the user
// deletes it. Elsewhere the variant is another path, as for git.
fn case_variant_of_an_excluded_name(
    incoming: &'static str,
    variant: &str,
    configure: impl Fn(&Path),
    entry_is_directory: bool,
) {
    for in_root in [false, true] {
        let file = [(incoming, "incoming\n")];
        let ws = if in_root {
            pull_workspace(&[("new.txt", "incoming\n")], &file)
        } else {
            pull_workspace(&file, &[])
        };
        let repo = if in_root { &ws.root } else { &ws.member };
        configure(repo);
        exclude(repo, &format!("/{incoming}\n"));
        let entry = repo.join(variant);
        let precious = if entry_is_directory {
            fs::create_dir_all(entry.join("sub")).unwrap();
            fs::write(entry.join("thesis.tex"), "PRECIOUS\n").unwrap();
            fs::write(entry.join("sub/notes.md"), "PRECIOUS\n").unwrap();
            entry.join("thesis.tex")
        } else {
            fs::write(&entry, "PRECIOUS\n").unwrap();
            entry.clone()
        };
        if !filesystem_is_case_insensitive(repo) {
            eprintln!(
                "case variant [{incoming} / {variant}, root={in_root}]: NOT APPLICABLE on this \
                 case-sensitive filesystem; checking that the variant is a separate path"
            );
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
            assert_eq!(fs::read_to_string(&precious).unwrap(), "PRECIOUS\n");
            continue;
        }
        let before = snapshot(&ws);

        assert_refuses_then_lands(&ws, SyncBehavior::FfOnly, &[incoming], &before, || {
            assert_eq!(fs::read_to_string(&precious).unwrap(), "PRECIOUS\n");
            remove_entry(&entry);
        });

        assert!(repo.join(incoming).is_file(), "{incoming} did not land");
    }
}

#[test]
fn row38a_a_non_ascii_case_variant_of_an_excluded_name_is_not_ignored() {
    case_variant_of_an_excluded_name("\u{c4}rger.txt", "\u{e4}rger.txt", |_| {}, false);
}

#[test]
fn row38b_a_case_variant_of_an_excluded_name_under_ignorecase_false_is_not_ignored() {
    case_variant_of_an_excluded_name(
        "Notes.txt",
        "notes.txt",
        |repo| set_config_bool(repo, "core.ignorecase", false),
        false,
    );
}

#[test]
fn row38c_an_untracked_directory_folded_onto_an_excluded_file_name_is_kept() {
    case_variant_of_an_excluded_name("\u{c4}rger", "\u{e4}rger", |_| {}, true);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        fn make_dir_link(link: &Path, target: &Path) -> bool {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    } else {
        /// A junction, which needs no privilege.
        fn make_dir_link(link: &Path, target: &Path) -> bool {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        }
    }
}

// F2 (State round 5, P2): a symlink (a junction on Windows) to a directory is an entry of
// its own, whatever a directory-only pattern (`cache/`) says about the directory it
// points to. The pull refuses naming the incoming path, with and without `--force`;
// the link and the data behind it are untouched; it lands once the user removes the link.
#[test]
fn row39_a_directory_link_under_a_directory_only_pattern_is_in_the_way() {
    for (tag, incoming, named) in [
        (
            "a file below the link's path",
            ("cache/README.md", "readme\n"),
            "cache/README.md",
        ),
        ("a file at the link's path", ("cache", "file\n"), "cache"),
    ] {
        let ws = pull_workspace(&[incoming], &[]);
        let outside = TempDir::new("pull-link-target");
        fs::write(outside.path().join("big.bin"), "BIG\n").unwrap();
        exclude(&ws.member, "cache/\n");
        let link = ws.member.join("cache");
        if !make_dir_link(&link, outside.path()) {
            eprintln!("row39 [{tag}]: NOT APPLICABLE: this process cannot create a directory link");
            continue;
        }
        fs::write(ws.member.join("junk.txt"), "junk\n").unwrap();
        let before = snapshot(&ws);
        let target = fs::read_link(&link).unwrap();

        assert_refuses_then_lands(&ws, SyncBehavior::FfOnly, &[named], &before, || {
            assert_eq!(
                fs::read_link(&link).unwrap(),
                target,
                "[{tag}] the link changed"
            );
            assert_eq!(
                fs::read_to_string(outside.path().join("big.bin")).unwrap(),
                "BIG\n"
            );
            remove_entry(&link);
        });

        assert_eq!(
            fs::read_to_string(outside.path().join("big.bin")).unwrap(),
            "BIG\n",
            "[{tag}] the data behind the link changed"
        );
        assert_eq!(
            fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
            "junk\n"
        );
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Kills the child when dropped.
        struct Running(std::process::Child);

        impl Drop for Running {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        // F3 (State round 5, P2), Windows: an ignored directory holding a running
        // executable cannot be deleted while it runs, and a probe that only opens it for
        // DELETE does not see that. Pull never tries: the directory is in the way, the
        // pull refuses before the root moves, the program keeps running, and the pull
        // lands once it has stopped and the user has deleted the directory.
        #[test]
        fn row40_an_ignored_directory_holding_a_running_executable_refuses() {
            let ws = pull_workspace(&[("out", "incoming file\n")], &[]);
            exclude(&ws.member, "out/\n");
            let directory = ws.member.join("out");
            fs::create_dir_all(&directory).unwrap();
            let program = directory.join("pinger.exe");
            let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            fs::copy(PathBuf::from(system).join("System32").join("PING.EXE"), &program).unwrap();
            let mut running = Running(
                std::process::Command::new(&program)
                    .args(["-n", "300", "127.0.0.1"])
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let before = snapshot(&ws);

            assert_refused_with_and_without_force(&ws, SyncBehavior::FfOnly, &["out"], &before);

            assert!(
                running.0.try_wait().unwrap().is_none(),
                "the program stopped running"
            );
            running.0.kill().unwrap();
            running.0.wait().unwrap();
            fs::remove_dir_all(&directory).unwrap();
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
        }
    } else {
        #[test]
        fn row40_an_ignored_directory_holding_a_running_executable_refuses() {
            eprintln!("row40: NOT APPLICABLE: a running program locks its file only on Windows");
        }
    }
}
