//! The scan for repository values (design §3; §9's "the scan" row).

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use super::fixture::{NATIVE, Workspace, fifo, home, within, write};
use crate::transport_scope::Operation;
use crate::transport_setting::{IgnoredValue, Location, Scope, Transport, ignored_values, resolve};

/// The scan's bound on a file it opens.
const ONE_MIB: usize = 1 << 20;

fn ignored(scope: Scope, file: PathBuf, included: bool, value: Option<&str>) -> IgnoredValue {
    IgnoredValue {
        scope,
        location: Location { file, included },
        value: value.map(str::to_owned),
    }
}

fn member(id: &str) -> Scope {
    Scope::Member(id.to_owned())
}

/// The root's `config`, and a targeted member's `config` and
/// `config.worktree`, each reported once, with its scope, member ID, file and
/// value as written: a malformed value, a file's last of several values, and
/// a key with no value.
#[test]
fn the_root_and_a_targeted_members_files_are_each_reported_once() {
    let ws = Workspace::new("setting-scan", &[("mem_a", "a"), ("mem_b", "b")]);
    write(&ws.git_dir("").join("config"), NATIVE);
    write(
        &ws.git_dir("a").join("config"),
        "[gwz]\n\ttransport = Bogus\n\ttransport = also bogus\n",
    );
    write(
        &ws.git_dir("a").join("config.worktree"),
        "[gwz]\n\ttransport\n",
    );
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&["@root", "mem_a"])),
        vec![
            ignored(
                Scope::Root,
                ws.git_dir("").join("config"),
                false,
                Some("native")
            ),
            ignored(
                member("mem_a"),
                ws.git_dir("a").join("config"),
                false,
                Some("also bogus")
            ),
            ignored(
                member("mem_a"),
                ws.git_dir("a").join("config.worktree"),
                false,
                None
            ),
        ]
    );
}

/// A member the request does not target is never read: a value in its
/// `config.worktree` gets no note, and a FIFO at its `.git/config` blocks
/// nothing.
#[test]
fn an_untargeted_member_is_never_read() {
    let ws = Workspace::new("setting-untargeted", &[("mem_a", "a"), ("mem_b", "b")]);
    write(&ws.git_dir("a").join("config"), NATIVE);
    fifo(&ws.git_dir("b").join("config"));
    write(&ws.git_dir("b").join("config.worktree"), NATIVE);
    let (root, meta) = (ws.root.clone(), ws.meta(&["mem_a"]));
    assert_eq!(
        within(move || ignored_values(Operation::Push, &root, &meta)),
        vec![ignored(
            member("mem_a"),
            ws.git_dir("a").join("config"),
            false,
            Some("native")
        )]
    );
}

/// A FIFO, a symbolic link and a file over 1 MiB are skipped without a note,
/// a FIFO at `.git/config` included, since the scan finds the git directory
/// from text and never opens the repository; a file of exactly 1 MiB is
/// read.
#[test]
fn fifos_links_and_files_over_1_mib_are_skipped_without_a_note() {
    let ws = Workspace::new(
        "setting-guard",
        &[
            ("mem_a", "a"),
            ("mem_b", "b"),
            ("mem_c", "c"),
            ("mem_d", "d"),
        ],
    );
    fifo(&ws.git_dir("").join("config"));
    fifo(&ws.git_dir("").join("config.worktree"));
    let real = ws.root.join("real.config");
    write(&real, NATIVE);
    symlink(&real, ws.git_dir("a").join("config")).unwrap();
    symlink(&real, ws.git_dir("a").join("config.worktree")).unwrap();
    let mut big = NATIVE.as_bytes().to_vec();
    big.resize(ONE_MIB + 1, b'\n');
    write(&ws.git_dir("b").join("config"), &big);
    let mut exact = NATIVE.as_bytes().to_vec();
    exact.push(b'#');
    exact.resize(ONE_MIB - 1, b'x');
    exact.push(b'\n');
    write(&ws.git_dir("c").join("config"), &exact);
    fifo(&ws.git_dir("d").join("config"));
    let (root, meta) = (
        ws.root.clone(),
        ws.meta(&["@root", "mem_a", "mem_b", "mem_c", "mem_d"]),
    );
    assert_eq!(
        within(move || ignored_values(Operation::Fetch, &root, &meta)),
        vec![ignored(
            member("mem_c"),
            ws.git_dir("c").join("config"),
            false,
            Some("native")
        )]
    );
}

/// A `.git` file is read as libgit2 reads it, `gitdir:` then a path relative
/// to the worktree, and a `commondir` file names the directory that holds
/// `config`, relative to the git directory; `config.worktree` stays in the
/// git directory.
#[test]
fn a_gitfile_and_commondir_locate_the_files() {
    let ws = Workspace::new("setting-gitfile", &[("mem_a", "a"), ("mem_b", "b")]);
    let git_dir = ws.root.join("store/worktrees/a");
    fs::remove_dir(ws.git_dir("a")).unwrap();
    write(&ws.git_dir("a"), format!("gitdir: {}\n", git_dir.display()));
    write(&git_dir.join("commondir"), "../..\n");
    write(&ws.root.join("store/config"), NATIVE);
    write(
        &git_dir.join("config.worktree"),
        "[gwz]\n\ttransport = gwz\n",
    );
    fs::remove_dir(ws.git_dir("b")).unwrap();
    write(&ws.git_dir("b"), "gitdir:   ../b.git  \n");
    write(&ws.root.join("b.git/config"), NATIVE);
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&["mem_a", "mem_b"])),
        vec![
            ignored(
                member("mem_a"),
                git_dir.join("../../config"),
                false,
                Some("native")
            ),
            ignored(
                member("mem_a"),
                git_dir.join("config.worktree"),
                false,
                Some("gwz")
            ),
            ignored(
                member("mem_b"),
                ws.root.join("b/../b.git/config"),
                false,
                Some("native")
            ),
        ]
    );
}

/// A `.git` or `commondir` that is not a regular file of at most 1 MiB, or
/// that does not say where the git directory is, gives the repository no
/// note.
#[test]
fn a_git_directory_that_cannot_be_located_from_text_gives_no_note() {
    let ws = Workspace::new(
        "setting-gitfile-bad",
        &[
            ("mem_a", "a"),
            ("mem_b", "b"),
            ("mem_c", "c"),
            ("mem_d", "d"),
        ],
    );
    let elsewhere = ws.root.join("elsewhere.git");
    write(&elsewhere.join("config"), NATIVE);
    fs::remove_dir(ws.git_dir("a")).unwrap();
    fifo(&ws.git_dir("a"));
    fs::remove_dir(ws.git_dir("b")).unwrap();
    write(
        &ws.git_dir("b"),
        format!("git dir: {}\n", elsewhere.display()),
    );
    fs::remove_dir(ws.git_dir("c")).unwrap();
    symlink(&elsewhere, ws.git_dir("c")).unwrap();
    write(&ws.git_dir("d").join("config"), NATIVE);
    fifo(&ws.git_dir("d").join("commondir"));
    let (root, meta) = (
        ws.root.clone(),
        ws.meta(&["mem_a", "mem_b", "mem_c", "mem_d"]),
    );
    assert_eq!(
        within(move || ignored_values(Operation::Fetch, &root, &meta)),
        Vec::new()
    );
}

/// A value that a repository's file reaches through `include.path` is noted
/// as in a file that it includes.
#[test]
fn a_value_reached_through_include_path_is_reported_as_included() {
    let ws = Workspace::new("setting-scan-include", &[]);
    write(
        &ws.git_dir("").join("config"),
        "[include]\n\tpath = ../extra.inc\n",
    );
    write(&ws.root.join("extra.inc"), NATIVE);
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&[])),
        vec![ignored(
            Scope::Root,
            ws.git_dir("").join("config"),
            true,
            Some("native")
        )]
    );
}

/// A request that creates its repositories, a clone into a new directory,
/// checks nothing, even when its workspace's repositories hold the key.
#[test]
fn a_clone_into_a_new_directory_checks_nothing() {
    let ws = Workspace::new("setting-clone", &[("mem_a", "a")]);
    write(&ws.git_dir("").join("config"), NATIVE);
    write(&ws.git_dir("a").join("config"), NATIVE);
    let mut into_new = ws.meta(&[]);
    into_new.workspace.as_mut().unwrap().root =
        Some(ws.root.join("new").to_str().unwrap().to_owned());
    for operation in [
        Operation::CloneWorkspace,
        Operation::CloneRepoMember,
        Operation::InitFromSources,
    ] {
        for meta in [&ws.meta(&[]), &into_new] {
            assert_eq!(
                ignored_values(operation, &ws.root, meta),
                Vec::new(),
                "{operation:?}"
            );
        }
    }
}

/// A value in the user's global configuration is never an ignored value:
/// the scan reads the targets' own files only.
#[test]
fn a_global_value_is_never_reported() {
    let ws = Workspace::new("setting-global", &[("mem_a", "a")]);
    write(&ws.root.join(".gitconfig"), NATIVE);
    write(&ws.git_dir("a").join("config"), "[user]\n\tname = U\n");
    assert_eq!(
        resolve(None, &home(&ws.root)).unwrap().transport,
        Transport::Native
    );
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&[])),
        Vec::new()
    );
}

/// With no selection each operation's own default applies: fetch, push and
/// pull target the root and every member, and tag, materialize and pull
/// snapshot the members alone.
#[test]
fn each_operations_default_targets_apply() {
    let ws = Workspace::new("setting-defaults", &[("mem_a", "a")]);
    write(&ws.git_dir("").join("config"), NATIVE);
    write(&ws.git_dir("a").join("config"), NATIVE);
    let root = ignored(
        Scope::Root,
        ws.git_dir("").join("config"),
        false,
        Some("native"),
    );
    let a = ignored(
        member("mem_a"),
        ws.git_dir("a").join("config"),
        false,
        Some("native"),
    );
    for operation in [Operation::Fetch, Operation::Push, Operation::PullHead] {
        assert_eq!(
            ignored_values(operation, &ws.root, &ws.meta(&[])),
            vec![root.clone(), a.clone()],
            "{operation:?}"
        );
    }
    for operation in [
        Operation::Tag,
        Operation::Materialize,
        Operation::PullSnapshot,
    ] {
        assert_eq!(
            ignored_values(operation, &ws.root, &ws.meta(&[])),
            vec![a.clone()],
            "{operation:?}"
        );
    }
}

/// What the operation would refuse gets no note, and the scan refuses
/// nothing: a selection it rejects, another workspace's ID, and a workspace
/// with no manifest.
#[test]
fn what_the_operation_would_refuse_gets_no_note() {
    let ws = Workspace::new("setting-refused", &[("mem_a", "a")]);
    write(&ws.git_dir("").join("config"), NATIVE);
    write(&ws.git_dir("a").join("config"), NATIVE);
    assert_eq!(
        ignored_values(Operation::Materialize, &ws.root, &ws.meta(&["@root"])),
        Vec::new()
    );
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&["mem_zz"])),
        Vec::new()
    );
    let mut other = ws.meta(&[]);
    other.workspace.as_mut().unwrap().workspace_id = Some("ws_other".to_owned());
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &other),
        Vec::new()
    );
    fs::remove_file(ws.root.join("gwz.conf/gwz.yml")).unwrap();
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&[])),
        Vec::new()
    );
}

/// A file that two targets share gets one note, the first target's.
#[test]
fn a_file_two_targets_share_gets_one_note() {
    let ws = Workspace::new("setting-shared", &[("mem_a", "a")]);
    write(&ws.git_dir("").join("config"), NATIVE);
    fs::remove_dir(ws.git_dir("a")).unwrap();
    write(
        &ws.git_dir("a"),
        format!("gitdir: {}\n", ws.git_dir("").display()),
    );
    assert_eq!(
        ignored_values(Operation::Fetch, &ws.root, &ws.meta(&[])),
        vec![ignored(
            Scope::Root,
            ws.git_dir("").join("config"),
            false,
            Some("native")
        )]
    );
}

/// A member at a path that holds a newline is scanned, and its file is
/// reported under that path; the notes escape it (`text.rs`).
#[test]
fn a_member_under_a_newline_is_scanned() {
    let ws = Workspace::new("setting-scan-newline", &[("mem_a", "line\nbreak")]);
    write(&ws.git_dir("line\nbreak").join("config"), NATIVE);
    let found = ignored_values(Operation::Fetch, &ws.root, &ws.meta(&["mem_a"]));
    assert_eq!(
        found,
        vec![ignored(
            member("mem_a"),
            ws.git_dir("line\nbreak").join("config"),
            false,
            Some("native")
        )]
    );
    assert_eq!(found[0].location.remove_text(), "remove it from that file");
    assert!(!found[0].location.where_text().contains('\n'));
}
