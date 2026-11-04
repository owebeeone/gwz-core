//! gwz 1.0.19 hotfix, remediation round 3 rows: one per finding of the round-3 State
//! (S3-Fn) and Code (R3-Fn) reviews, same fixture as the parent rows.
use std::fs;
use std::path::PathBuf;

use crate::SyncBehavior;
use crate::git::{Git2Backend, GitBackend, GitPullUpdate};
use crate::model::ErrorCode;

use super::round1::{commit_change, push_root_commit};
use super::round2::set_ignore_case;
use super::*;

pub(super) const MODES: [SyncBehavior; 3] = [
    SyncBehavior::FfOnly,
    SyncBehavior::Merge,
    SyncBehavior::Rebase,
];

/// Case-variant name pairs: ASCII, and a non-ASCII letter the filesystem folds but
/// libgit2's ASCII folding does not (NFC, as git stores it).
const NAMES: [(&str, &str); 2] = [("Case", "case"), ("\u{c4}rger", "\u{e4}rger")];

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Untracked `<lower>.txt`; the update adds `<Upper>.txt`.
    File,
    /// Untracked file `<lower>`; the update adds `<Upper>/inner.txt`.
    Parent,
    /// HEAD tracks `<Upper>/a.txt`; the update replaces it with the file `<lower>`;
    /// untracked `<Upper>/local.txt`.
    DirectoryToFile,
}

struct CaseScenario {
    ws: PullWorkspace,
    /// The participant whose worktree holds the case variant.
    repo: PathBuf,
    /// The untracked file in the way and the incoming path it blocks.
    local: PathBuf,
    collision: String,
    /// The incoming file and its content once it lands.
    incoming: String,
    content: &'static str,
}

/// Push a root commit from a fresh peer clone writing `writes` and removing `removes`.
fn push_root_change(
    ws: &PullWorkspace,
    peer: &str,
    writes: &[(&str, &str)],
    removes: &[&str],
) -> String {
    let backend = Git2Backend::new();
    let peer = ws._remotes.path().join(peer);
    backend
        .clone_repo(ws._remotes.path().join("root.git").to_str().unwrap(), &peer)
        .unwrap();
    set_identity(&peer);
    let commit = commit_change(&peer, writes, removes, "root peer change");
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    commit
}

fn case_scenario(shape: Shape, upper: &str, lower: &str, in_root: bool) -> CaseScenario {
    let added = match shape {
        Shape::File => format!("{upper}.txt"),
        Shape::Parent => format!("{upper}/inner.txt"),
        Shape::DirectoryToFile => format!("{upper}/a.txt"),
    };
    let first = [(added.as_str(), "incoming\n")];
    let mut ws = if in_root {
        pull_workspace(&[("new.txt", "incoming\n")], &first)
    } else {
        pull_workspace(&first, &[])
    };
    let repo = if in_root {
        ws.root.clone()
    } else {
        ws.member.clone()
    };
    let (local, collision, incoming, content) = match shape {
        Shape::File => (format!("{lower}.txt"), added.clone(), added, "incoming\n"),
        Shape::Parent => (lower.to_owned(), added.clone(), added, "incoming\n"),
        Shape::DirectoryToFile => {
            pull(&ws, SyncBehavior::FfOnly, false).unwrap();
            assert_pulled(&ws);
            ws.member_before = ws.member_after.clone();
            ws.root_before = ws.root_after.clone();
            let file = [(lower, "file\n")];
            if in_root {
                ws.root_after = push_root_change(&ws, "peer-case", &file, &[upper]);
            } else {
                let source = ws._member_fixture.source.clone();
                ws.member_after = commit_change(&source, &file, &[upper], "dir becomes a file");
                Git2Backend::new()
                    .push(&source, "origin", "refs/heads/main:refs/heads/main")
                    .unwrap();
                ws.root_after = push_root_commit(&ws, "peer-case", "root-case.txt");
            }
            (
                format!("{upper}/local.txt"),
                lower.to_owned(),
                lower.to_owned(),
                "file\n",
            )
        }
    };
    let local = repo.join(local);
    fs::write(&local, "mine\n").unwrap();
    fs::write(repo.join("junk.txt"), "junk\n").unwrap();
    CaseScenario {
        ws,
        repo,
        local,
        collision,
        incoming,
        content,
    }
}

/// S3-F1 (P1): with the filesystem as the authority, a case variant the filesystem
/// folds collides like any other entry in the way, ASCII or not, in the root and in a
/// member, in every integrating mode: refused before anything moves, with and without
/// `--force`; the pull lands once the user deletes it. On a case-sensitive filesystem
/// the variant is another path.
fn case_variant_rows(shape: Shape) {
    for (upper, lower) in NAMES {
        for in_root in [false, true] {
            for sync in MODES {
                let tag = format!("{shape:?} {upper}/{lower} root={in_root} {sync:?}");
                let scenario = case_scenario(shape, upper, lower, in_root);
                let ws = &scenario.ws;
                if !filesystem_is_case_insensitive(&scenario.repo) {
                    eprintln!("row24 [{tag}]: NOT APPLICABLE on this case-sensitive filesystem");
                    pull(ws, sync, false)
                        .unwrap_or_else(|error| panic!("[{tag}] {}", error.message));
                    assert_eq!(
                        fs::read_to_string(&scenario.local).unwrap(),
                        "mine\n",
                        "[{tag}]"
                    );
                    continue;
                }
                let before = snapshot(ws);

                for force in [false, true] {
                    let error = pull(ws, sync, force).unwrap_err();

                    assert_collision_refusal(&error, &[&scenario.collision]);
                    assert_untouched(ws, &before);
                }

                fs::remove_file(&scenario.local).unwrap();
                pull(ws, sync, false).unwrap_or_else(|error| panic!("[{tag}] {}", error.message));

                assert_eq!(head(&ws.root), ws.root_after, "[{tag}] root did not move");
                assert_eq!(
                    head(&ws.member),
                    ws.member_after,
                    "[{tag}] member did not move"
                );
                assert_eq!(
                    fs::read_to_string(scenario.repo.join(&scenario.incoming)).unwrap(),
                    scenario.content,
                    "[{tag}]"
                );
                assert_eq!(
                    fs::read_to_string(scenario.repo.join("junk.txt")).unwrap(),
                    "junk\n",
                    "[{tag}]"
                );
            }
        }
    }
}

#[test]
fn row24a_case_variant_file_collides_where_the_filesystem_folds_case() {
    case_variant_rows(Shape::File);
}

#[test]
fn row24b_case_variant_file_at_a_parent_path_collides_where_the_filesystem_folds_case() {
    case_variant_rows(Shape::Parent);
}

#[test]
fn row24c_case_variant_directory_becoming_a_file_keeps_untracked_content() {
    case_variant_rows(Shape::DirectoryToFile);
}

// S3-F1 (P1), the other side: a tracked case-only rename upstream (a file, ASCII or
// not; a directory, and an ASCII-renamed directory holding an untracked file) still
// pulls on every filesystem, in every mode: the entry found at the new name is proven
// to be the tracked one the update removes.
#[test]
fn row25_tracked_case_only_renames_still_pull() {
    let renames = [
        ("Foo.txt", "foo.txt", None),
        ("\u{c4}rger.txt", "\u{e4}rger.txt", None),
        ("\u{d6}l/a.txt", "\u{f6}l/a.txt", None),
        ("Docs/a.txt", "docs/a.txt", Some("Docs/notes.txt")),
    ];
    for (from, to, untracked) in renames {
        for sync in MODES {
            let tag = format!("{from} -> {to} {sync:?}");
            let ws = case_rename_workspace(from, to, untracked);

            pull(&ws, sync, false).unwrap_or_else(|error| panic!("[{tag}] {}", error.message));

            assert_eq!(
                head(&ws.member),
                ws.member_after,
                "[{tag}] member did not move"
            );
            assert_eq!(head(&ws.root), ws.root_after, "[{tag}] root did not move");
            assert_eq!(
                fs::read_to_string(ws.member.join(to)).unwrap(),
                "new\n",
                "[{tag}]"
            );
            if let Some(untracked) = untracked {
                assert_eq!(
                    fs::read_to_string(ws.member.join(untracked)).unwrap(),
                    "notes\n",
                    "[{tag}]"
                );
            }
        }
    }
}

/// A member pulled once tracking `from`; its remote then renames it to `to` (by case
/// only) and the root's remote gains a commit; `untracked` is written in the member.
fn case_rename_workspace(from: &str, to: &str, untracked: Option<&str>) -> PullWorkspace {
    let mut ws = pull_workspace(&[(from, "old\n")], &[]);
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    let source = ws._member_fixture.source.clone();
    ws.member_before = ws.member_after.clone();
    ws.member_after = commit_change(&source, &[(to, "new\n")], &[from], "case rename");
    Git2Backend::new()
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    ws.root_before = ws.root_after.clone();
    ws.root_after = push_root_commit(&ws, "peer-rename", "root-rename.txt");
    if let Some(untracked) = untracked {
        fs::write(ws.member.join(untracked), "notes\n").unwrap();
    }
    ws
}

// S3-F1 (P1), where libgit2 cannot follow the filesystem: an incoming file whose parent
// directory the filesystem reaches under a non-ASCII case variant (an untracked
// directory, or a tracked one the update renames by case that keeps an untracked file)
// would land under the old spelling and leave the member looking changed after the
// root moved. It refuses before anything moves, with or without `--force`. The ASCII
// variant, which libgit2 matches under `core.ignorecase`, pulls.
#[test]
fn row24d_case_variant_parent_directory_refuses_unless_libgit2_matches_it() {
    for (upper, lower) in NAMES {
        let ascii = upper.is_ascii();
        let tag = format!("{upper}/{lower}");
        let ws = pull_workspace(&[(&format!("{upper}/inner.txt"), "incoming\n")], &[]);
        fs::create_dir_all(ws.member.join(lower)).unwrap();
        let mine = ws.member.join(format!("{lower}/mine.txt"));
        fs::write(&mine, "mine\n").unwrap();
        let renamed = case_rename_workspace(
            &format!("{upper}/a.txt"),
            &format!("{lower}/a.txt"),
            Some(&format!("{upper}/notes.txt")),
        );
        let applies = filesystem_is_case_insensitive(&ws.member);
        if ascii || !applies {
            eprintln!(
                "row24d [{tag}]: the pull must land (case-insensitive filesystem: {applies})"
            );
            pull(&ws, SyncBehavior::FfOnly, false)
                .unwrap_or_else(|error| panic!("[{tag}] {}", error.message));
            assert_pulled(&ws);
            assert_eq!(fs::read_to_string(&mine).unwrap(), "mine\n", "[{tag}]");
            assert_eq!(
                fs::read_to_string(ws.member.join(format!("{upper}/inner.txt"))).unwrap(),
                "incoming\n"
            );
            pull(&renamed, SyncBehavior::FfOnly, false)
                .unwrap_or_else(|error| panic!("[{tag}] {}", error.message));
            assert_eq!(head(&renamed.member), renamed.member_after, "[{tag}]");
            continue;
        }
        for (ws, blocked) in [
            (&ws, format!("{upper}/inner.txt")),
            (&renamed, format!("{lower}/a.txt")),
        ] {
            let before = snapshot(ws);
            for force in [false, true] {
                let error = pull(ws, SyncBehavior::FfOnly, force).unwrap_err();
                assert_eq!(
                    error.code,
                    ErrorCode::DirtyMember,
                    "[{tag}] {}",
                    error.message
                );
                assert!(
                    error.message.contains(&blocked) && error.message.contains("spelled otherwise"),
                    "[{tag}] {}",
                    error.message
                );
                assert_untouched(ws, &before);
            }
        }
    }
}

/// A member pulled once that then commits `writes`, and a second local commit removing
/// `drops`, while its remote gains `remote-two.txt` (removing `remote_drops`) and the
/// root's remote gains a commit: the next `--sync rebase` replays both local commits.
pub(super) fn diverged_member(
    first: &[(&str, &str)],
    second_writes: &[(&str, &str)],
    drops: &[&str],
    remote_drops: &[&str],
) -> PullWorkspace {
    let backend = Git2Backend::new();
    let mut ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    pull(&ws, SyncBehavior::FfOnly, false).unwrap();
    let source = ws._member_fixture.source.clone();
    commit_change(
        &source,
        &[("remote-two.txt", "r2\n")],
        remote_drops,
        "remote two",
    );
    backend
        .push(&source, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    commit_change(&ws.member, first, &[], "local one");
    ws.member_before = commit_change(&ws.member, second_writes, drops, "local two");
    ws.root_before = head(&ws.root);
    ws.root_after = push_root_commit(&ws, "peer-rebase", "root-rebase.txt");
    ws
}

pub(super) fn assert_rebased(ws: &PullWorkspace) {
    let backend = Git2Backend::new();
    let member = head(&ws.member);
    let remote = backend
        .read_ref(&ws.member, "refs/remotes/origin/main")
        .unwrap()
        .unwrap();
    assert!(
        backend.is_ancestor(&ws.member, &remote, &member).unwrap() && member != ws.member_before,
        "member was not rebased"
    );
    assert_eq!(head(&ws.root), ws.root_after, "root did not move");
}

// R3-F2 (P3): the replay rule judges what each replayed commit writes, not its whole
// tree: an untracked `README.md` that the base tracked, that upstream and a later local
// commit delete, and that no replayed commit writes, does not block `--sync rebase`.
#[test]
fn row29_rebase_ignores_untracked_files_no_replayed_commit_writes() {
    let ws = diverged_member(
        &[("local.txt", "l\n")],
        &[("local-two.txt", "l2\n")],
        &["README.md"],
        &["README.md"],
    );
    fs::write(ws.member.join("README.md"), "my notes\n").unwrap();

    pull(&ws, SyncBehavior::Rebase, false).unwrap_or_else(|error| panic!("{}", error.message));

    assert_rebased(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("README.md")).unwrap(),
        "my notes\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("local-two.txt")).unwrap(),
        "l2\n"
    );
}

// R3-F1 (P3): planning stays linear under `core.ignorecase`: adding 3000 files into a
// directory of 3000 other entries costs about what it costs with the setting off
// (round 2 scanned the directory once per added file).
#[test]
fn row30_collision_query_under_ignorecase_stays_linear() {
    let n = 3000;
    let names = (0..n)
        .map(|index| format!("big/f{index:05}.txt"))
        .collect::<Vec<_>>();
    let files = names
        .iter()
        .map(|name| (name.as_str(), "x\n"))
        .collect::<Vec<_>>();
    let ws = pull_workspace(&files, &[]);
    fs::write(ws.member.join(".git/info/exclude"), "*.o\n").unwrap();
    fs::create_dir_all(ws.member.join("big")).unwrap();
    for index in 0..n {
        fs::write(ws.member.join(format!("big/g{index:05}.o")), "o\n").unwrap();
    }
    let backend = Git2Backend::new();
    backend.fetch(&ws.member, "origin").unwrap();
    let time = |ignore_case: bool| {
        if ignore_case {
            set_ignore_case(&ws.member);
        } else {
            git2::Repository::open(&ws.member)
                .unwrap()
                .config()
                .unwrap()
                .set_bool("core.ignorecase", false)
                .unwrap();
        }
        (0..2)
            .map(|_| {
                let start = std::time::Instant::now();
                let collisions = backend
                    .update_collisions(&ws.member, &ws.member_after)
                    .unwrap();
                assert!(collisions.is_empty(), "{collisions:?}");
                start.elapsed()
            })
            .min()
            .unwrap()
    };
    let plain = time(false);
    let folded = time(true);
    eprintln!("row30: n={n} core.ignorecase=false {plain:?}, true {folded:?}");
    // Generous for a linear rule (ten times, plus half a second); round 2 took 1.2 s
    // (ext4) to 3.6 s (APFS) here against 5-20 ms with the setting off.
    let bound = plain * 10 + std::time::Duration::from_millis(500);
    assert!(
        folded <= bound,
        "ignorecase planning took {folded:?} (bound {bound:?})"
    );
}
