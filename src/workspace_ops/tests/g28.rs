//! gwz 1.0.19 hotfix rows: `gwz pull` and untracked files.
//!
//! Every row runs a real root workspace with one member against two bare remotes
//! whose new commits each add files: the root's records the member's new commit in
//! the lock and adds `root-new.txt`. Real Git only (`Git2Backend`), local identity.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{Git2Backend, GitBackend};
use crate::model::{ErrorCode, ModelError};

use super::*;

mod round1;
mod round2;
mod round3;
mod round4;
mod round5;

struct PullWorkspace {
    _temp: TempDir,
    _remotes: TempDir,
    _member_fixture: RemoteFixture,
    root: PathBuf,
    member: PathBuf,
    root_before: String,
    root_after: String,
    member_before: String,
    member_after: String,
}

/// Write `files` (creating parents), add them and `extra` to the index and commit on HEAD.
fn commit_paths(repo_path: &Path, files: &[(&str, &str)], extra: &[&str], message: &str) -> String {
    for (path, content) in files {
        let full = repo_path.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }
    let repo = git2::Repository::open(repo_path).unwrap();
    let mut index = repo.index().unwrap();
    for path in files
        .iter()
        .map(|(path, _)| *path)
        .chain(extra.iter().copied())
    {
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

/// A committed root workspace plus member, both one commit behind their remotes. The
/// member's remote commit adds `member_files`; the root's adds `root-new.txt` (plus
/// `root_files`) and records the member's new commit in the lock.
fn pull_workspace(member_files: &[(&str, &str)], root_files: &[(&str, &str)]) -> PullWorkspace {
    pull_workspace_with(member_files, root_files, false)
}

/// `pull_workspace`, and with `add_member` the root's remote commit also registers a
/// second member, `mem_extra` at `extra`, that is not materialized locally.
fn pull_workspace_with(
    member_files: &[(&str, &str)],
    root_files: &[(&str, &str)],
    add_member: bool,
) -> PullWorkspace {
    let backend = Git2Backend::new();
    let temp = TempDir::new("pull-untracked-root");
    let root = temp.path().to_path_buf();
    let member_fixture = init_one_member_workspace(&root, &backend, "pull-untracked-member");
    let member = root.join("remote");
    let member_before = backend.head(&member).unwrap().commit.unwrap();

    backend.stage_paths(&root, &["."]).unwrap();
    backend.commit(&root, "root baseline", false).unwrap();
    assert!(
        !backend.status(&root).unwrap().is_dirty,
        "fixture root must start clean"
    );
    let root_before = backend.head(&root).unwrap().commit.unwrap();

    let remotes = TempDir::new("pull-untracked-remotes");
    let root_remote = remotes.path().join("root.git");
    init_bare_main(&root_remote);
    backend
        .add_remote(&root, "origin", root_remote.to_str().unwrap())
        .unwrap();
    backend
        .push(&root, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    set_identity(&member_fixture.source);
    let member_after = commit_paths(&member_fixture.source, member_files, &[], "member new");
    backend
        .push(
            &member_fixture.source,
            "origin",
            "refs/heads/main:refs/heads/main",
        )
        .unwrap();

    let peer = remotes.path().join("peer");
    backend
        .clone_repo(root_remote.to_str().unwrap(), &peer)
        .unwrap();
    set_identity(&peer);
    let mut lock = crate::artifact::read_lock(&peer).unwrap();
    for state in lock.members.values_mut() {
        state.commit = Some(member_after.clone());
    }
    if add_member {
        let mut manifest = crate::artifact::read_manifest(&peer).unwrap();
        let mut extra = manifest.members[0].clone();
        extra.id = "mem_extra".to_owned();
        extra.path = "extra".to_owned();
        extra.source_id = "src_extra".to_owned();
        manifest.members.push(extra);
        crate::artifact::write_manifest(&peer, &manifest).unwrap();
        let mut state = lock.members.values().next().unwrap().clone();
        state.path = "extra".to_owned();
        state.source_id = Some("src_extra".to_owned());
        lock.members.insert("mem_extra".to_owned(), state);
    }
    crate::artifact::write_lock(&peer, &lock).unwrap();
    let mut files = vec![("root-new.txt", "root new\n")];
    files.extend_from_slice(root_files);
    let metadata = [
        crate::workspace::WORKSPACE_MANIFEST,
        crate::artifact::LOCK_PATH,
        crate::artifact::CONF_INTEGRITY_MARKER_PATH,
    ]
    .into_iter()
    .filter(|path| peer.join(path).exists())
    .collect::<Vec<_>>();
    let root_after = commit_paths(&peer, &files, &metadata, "root new");
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    PullWorkspace {
        _temp: temp,
        _remotes: remotes,
        _member_fixture: member_fixture,
        root,
        member,
        root_before,
        root_after,
        member_before,
        member_after,
    }
}

fn pull_request(sync: crate::SyncBehavior, force: bool, dry_run: bool) -> crate::PullHeadRequest {
    let mut request = pull_head_request_with_sync(sync);
    if force {
        request
            .meta
            .policy
            .get_or_insert_with(Default::default)
            .destructive = Some(crate::DestructiveBehavior::Allow);
    }
    if dry_run {
        request.meta.dry_run = Some(true);
    }
    request
}

fn pull(
    ws: &PullWorkspace,
    sync: crate::SyncBehavior,
    force: bool,
) -> Result<crate::PullHeadResponse, ModelError> {
    handle_pull_head(
        &Git2Backend::new(),
        &ws.root,
        pull_request(sync, force, false),
        "op_pull",
    )
}

/// Every worktree file, directory and link of the root and member (outside `.git` and the
/// runtime dir), both indexes, both HEADs and branch refs, and the root's local exclude.
#[derive(Debug, Eq, PartialEq)]
struct Snapshot {
    entries: BTreeMap<String, Option<Vec<u8>>>,
    metadata: BTreeMap<String, Option<Vec<u8>>>,
    heads: Vec<String>,
}

fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let name = entry.file_name();
        if name == ".git" || relative == crate::workspace::RUNTIME_DIR {
            continue;
        }
        let kind = fs::symlink_metadata(&path).unwrap();
        if kind.file_type().is_symlink() {
            // A link (or junction) is an entry of its own: record where it points, and
            // never follow it.
            let target = fs::read_link(&path).unwrap();
            out.insert(
                format!("{relative}@"),
                Some(target.to_string_lossy().into_owned().into_bytes()),
            );
        } else if kind.is_dir() {
            out.insert(format!("{relative}/"), None);
            walk(base, &path, out);
        } else {
            out.insert(relative, Some(fs::read(&path).unwrap()));
        }
    }
}

fn snapshot(ws: &PullWorkspace) -> Snapshot {
    let backend = Git2Backend::new();
    let mut entries = BTreeMap::new();
    walk(&ws.root, &ws.root, &mut entries);
    let mut metadata = BTreeMap::new();
    for path in [
        ws.root.join(".git/index"),
        ws.root.join(".git/info/exclude"),
        ws.member.join(".git/index"),
    ] {
        metadata.insert(path.to_string_lossy().into_owned(), fs::read(&path).ok());
    }
    let heads = [&ws.root, &ws.member]
        .iter()
        .flat_map(|repo| {
            [
                format!("{:?}", backend.head(repo).unwrap()),
                format!("{:?}", backend.read_ref(repo, "refs/heads/main").unwrap()),
            ]
        })
        .collect();
    Snapshot {
        entries,
        metadata,
        heads,
    }
}

fn head(repo: &Path) -> String {
    Git2Backend::new().head(repo).unwrap().commit.unwrap()
}

fn assert_pulled(ws: &PullWorkspace) {
    assert_eq!(head(&ws.root), ws.root_after, "root did not move");
    assert_eq!(head(&ws.member), ws.member_after, "member did not move");
    assert_eq!(
        fs::read_to_string(ws.root.join("root-new.txt")).unwrap(),
        "root new\n"
    );
}

fn assert_untouched(ws: &PullWorkspace, before: &Snapshot) {
    assert_eq!(head(&ws.root), ws.root_before, "root moved on a refusal");
    assert_eq!(
        head(&ws.member),
        ws.member_before,
        "member moved on a refusal"
    );
    assert_eq!(&snapshot(ws), before, "a refusal mutated the workspace");
}

fn assert_collision_refusal(error: &ModelError, paths: &[&str]) {
    eprintln!(
        "refusal: {:?} {:?}: {}",
        error.code, error.member_id, error.message
    );
    assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
    for path in paths {
        assert!(
            error.message.contains(path),
            "refusal does not name '{path}': {}",
            error.message
        );
    }
    assert!(
        error.message.contains("move or delete"),
        "refusal does not tell the user to move or delete the files: {}",
        error.message
    );
    assert!(
        !error.message.contains("--force"),
        "refusal advises --force, which cannot help: {}",
        error.message
    );
}

/// Pull with and without `--force`: both refuse the same way, naming `paths`, and leave
/// the workspace exactly as `before`.
fn assert_refused_with_and_without_force(
    ws: &PullWorkspace,
    sync: crate::SyncBehavior,
    paths: &[&str],
    before: &Snapshot,
) {
    for force in [false, true] {
        let error = pull(ws, sync, force).unwrap_err();
        assert_collision_refusal(&error, paths);
        assert_untouched(ws, before);
    }
}

/// Append `patterns` (one per line) to the repository's local exclude file.
fn exclude(repo: &Path, patterns: &str) {
    use std::io::Write;
    let path = repo.join(".git/info/exclude");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(patterns.as_bytes()).unwrap();
}

fn filesystem_is_case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("CaseProbe.gwz-test");
    fs::write(&probe, "probe").unwrap();
    let insensitive = dir.join("caseprobe.gwz-test").exists();
    fs::remove_file(probe).unwrap();
    insensitive
}

// Row 1: an unrelated untracked member file never blocks an ff-only pull.
#[test]
fn row1_unrelated_untracked_member_file_does_not_block_pull() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();

    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("new.txt")).unwrap(),
        "incoming\n"
    );
}

// Row 1b (B1): the integrating modes and ignored files behave the same.
#[test]
fn row1b_unrelated_untracked_and_ignored_files_do_not_block_any_integrating_mode() {
    for sync in [
        crate::SyncBehavior::Merge,
        crate::SyncBehavior::Rebase,
        crate::SyncBehavior::DriverSelected,
    ] {
        let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
        fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();
        fs::write(ws.member.join(".git/info/exclude"), "ignored.txt\n").unwrap();
        fs::write(ws.member.join("ignored.txt"), "ignored\n").unwrap();
        fs::write(ws.root.join("junk-root.txt"), "root junk\n").unwrap();

        pull(&ws, sync, false).unwrap_or_else(|error| panic!("{sync:?}: {}", error.message));

        assert_pulled(&ws);
        assert_eq!(
            fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
            "mine\n"
        );
        assert_eq!(
            fs::read_to_string(ws.member.join("ignored.txt")).unwrap(),
            "ignored\n"
        );
    }
}

// Row 2 (B8): an unrelated untracked root file never blocks.
#[test]
fn row2_unrelated_untracked_root_file_does_not_block_pull() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.root.join("junk-root.txt"), "root junk\n").unwrap();

    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.root.join("junk-root.txt")).unwrap(),
        "root junk\n"
    );
}

// Row 2b (B2/B8): a colliding untracked root file refuses before any mutation, with
// and without `--force` (which deletes no untracked file); once the user deletes it the
// second pull lands and leaves the other untracked file alone.
#[test]
fn row2b_colliding_untracked_root_file_refuses_with_and_without_force() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.root.join("root-new.txt"), "local root\n").unwrap();
    fs::write(ws.root.join("junk-root.txt"), "root junk\n").unwrap();
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(
        &ws,
        crate::SyncBehavior::FfOnly,
        &["root-new.txt"],
        &before,
    );

    fs::remove_file(ws.root.join("root-new.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.root.join("junk-root.txt")).unwrap(),
        "root junk\n"
    );
}

fn row_message(response: &crate::PullHeadResponse, member_id: &str) -> String {
    response
        .response
        .members
        .iter()
        .find(|row| row.member_id == member_id)
        .and_then(|row| row.planned.as_ref())
        .and_then(|planned| planned.message.clone())
        .unwrap_or_default()
}

// Row 3 (B2/B5): a colliding untracked member file refuses, naming it; the root and
// member stay byte-identical; a second pull after removing it succeeds.
#[test]
fn row3_colliding_untracked_member_file_refuses_without_mutation() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
    let before = snapshot(&ws);

    let error = pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap_err();

    assert_collision_refusal(&error, &["new.txt"]);
    assert_eq!(error.member_path.as_deref(), Some("remote"));
    assert_untouched(&ws, &before);

    fs::remove_file(ws.member.join("new.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
}

// Row 4 (B3 withdrawn): `--force` does not overwrite the colliding file either: the
// pull refuses, the workspace is byte-identical, another untracked file is untouched,
// and a second pull after the user deletes the file succeeds.
#[test]
fn row4_force_does_not_overwrite_the_colliding_member_file() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
    fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(&ws, crate::SyncBehavior::FfOnly, &["new.txt"], &before);
    assert_eq!(
        fs::read_to_string(ws.member.join("new.txt")).unwrap(),
        "local\n"
    );

    fs::remove_file(ws.member.join("new.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, true).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("new.txt")).unwrap(),
        "incoming\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "mine\n"
    );
}

// Row 5 (B4): a tracked edit refuses with and without `--force`; nothing is mutated.
#[test]
fn row5_tracked_member_edit_refuses_with_and_without_force() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("README.md"), "edited\n").unwrap();
    let before = snapshot(&ws);

    for force in [false, true] {
        let error = pull(&ws, crate::SyncBehavior::FfOnly, force).unwrap_err();
        assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
        assert!(
            error.message.contains("uncommitted changes"),
            "{}",
            error.message
        );
        assert_untouched(&ws, &before);
    }
}

// Row 6 (D2/D3): a member refusal after the root would move leaves the root exactly as
// it was (no fast-forward, no staged gwz.conf), and the next pull succeeds once the
// member is clean.
#[test]
fn row6_member_refusal_leaves_the_moving_root_unchanged() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("README.md"), "edited\n").unwrap();
    let before = snapshot(&ws);

    let error = pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
    assert_untouched(&ws, &before);
    assert!(
        !Git2Backend::new().status(&ws.root).unwrap().is_dirty,
        "a refused pull left root changes behind"
    );

    let repo = git2::Repository::open(&ws.member).unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
}

// Row 7 (B2): an untracked file at a parent path of an added file, and an untracked
// directory at an added file's path, both collide, with and without `--force`; once the
// user deletes them the pull lands.
#[test]
fn row7_parent_path_file_and_directory_at_file_path_collide() {
    let ws = pull_workspace(
        &[("sub/inner.txt", "inner\n"), ("blocked.txt", "blocked\n")],
        &[],
    );
    fs::write(ws.member.join("sub"), "a file where a directory comes\n").unwrap();
    fs::create_dir_all(ws.member.join("blocked.txt")).unwrap();
    fs::write(ws.member.join("blocked.txt/mine.txt"), "mine\n").unwrap();
    fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(
        &ws,
        crate::SyncBehavior::FfOnly,
        &["sub", "blocked.txt"],
        &before,
    );

    fs::remove_file(ws.member.join("sub")).unwrap();
    fs::remove_dir_all(ws.member.join("blocked.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("sub/inner.txt")).unwrap(),
        "inner\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("blocked.txt")).unwrap(),
        "blocked\n"
    );
    assert_eq!(
        fs::read_to_string(ws.member.join("junk.txt")).unwrap(),
        "mine\n"
    );
}

// Row 7b (B2): the refusal caps the list and gives the count.
#[test]
fn row7b_collision_list_is_capped_with_a_count() {
    let names = (0..14)
        .map(|index| format!("many-{index:02}.txt"))
        .collect::<Vec<_>>();
    let files = names
        .iter()
        .map(|name| (name.as_str(), "incoming\n"))
        .collect::<Vec<_>>();
    let ws = pull_workspace(&files, &[]);
    for name in &names {
        fs::write(ws.member.join(name), "local\n").unwrap();
    }

    let error = pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap_err();

    assert_collision_refusal(&error, &["many-00.txt", "14"]);
    assert!(
        !error.message.contains("many-13.txt"),
        "uncapped: {}",
        error.message
    );
}

// Row 8 (B2): a case variant collides on a case-insensitive filesystem (macOS,
// Windows). On a case-sensitive filesystem the variant is a different path, as for git:
// the row checks that and reports that the case rule is not applicable here.
#[test]
fn row8_case_variant_collides_on_case_insensitive_filesystems() {
    let ws = pull_workspace(&[("Case.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("case.txt"), "local\n").unwrap();
    if !filesystem_is_case_insensitive(&ws.member) {
        eprintln!(
            "row8: NOT APPLICABLE on this case-sensitive filesystem; checking that the \
             variant is a separate path instead"
        );
        pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
        assert_pulled(&ws);
        assert_eq!(
            fs::read_to_string(ws.member.join("case.txt")).unwrap(),
            "local\n"
        );
        return;
    }
    eprintln!("row8: case-insensitive filesystem; the case variant must collide");
    let before = snapshot(&ws);

    assert_refused_with_and_without_force(&ws, crate::SyncBehavior::FfOnly, &["Case.txt"], &before);

    fs::remove_file(ws.member.join("case.txt")).unwrap();
    pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();
    assert_pulled(&ws);
    assert_eq!(
        fs::read_to_string(ws.member.join("Case.txt")).unwrap(),
        "incoming\n"
    );
}

// Row 9 (B7): `--dry-run` reports the collision it would refuse, with and without
// `--force`; it mutates nothing.
#[test]
fn row9_dry_run_reports_collisions_and_mutates_nothing() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("new.txt"), "local\n").unwrap();
    fs::write(ws.root.join("root-new.txt"), "local root\n").unwrap();
    let before = snapshot(&ws);
    let backend = Git2Backend::new();
    let dry_run = |force: bool| {
        handle_pull_head(
            &backend,
            &ws.root,
            pull_request(crate::SyncBehavior::FfOnly, force, true),
            "op_pull_dry",
        )
        .unwrap_err()
    };

    for force in [false, true] {
        assert_collision_refusal(&dry_run(force), &["root-new.txt"]);
        assert_untouched(&ws, &before);
    }

    fs::remove_file(ws.root.join("root-new.txt")).unwrap();
    let before = snapshot(&ws);
    for force in [false, true] {
        assert_collision_refusal(&dry_run(force), &["new.txt"]);
        assert_untouched(&ws, &before);
    }
}

// B4: `--sync reset` keeps its rule: any dirt, untracked included, refuses unless --force.
#[test]
fn reset_still_refuses_untracked_files_without_force() {
    let ws = pull_workspace(&[("new.txt", "incoming\n")], &[]);
    fs::write(ws.member.join("junk.txt"), "mine\n").unwrap();
    let before = snapshot(&ws);

    let error = pull(&ws, crate::SyncBehavior::Reset, false).unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember, "{}", error.message);
    assert_untouched(&ws, &before);
}

// B5 consequence: a member the root update registers and that is not materialized here
// has nothing to integrate; the pull moves the root and the existing member and leaves
// the new one to `gwz materialize` (1.0.18 moved the root, then failed on it).
#[test]
fn row10_member_added_by_the_root_update_is_left_to_materialize() {
    let ws = pull_workspace_with(&[("new.txt", "incoming\n")], &[], true);

    let response = pull(&ws, crate::SyncBehavior::FfOnly, false).unwrap();

    assert_pulled(&ws);
    // The skipped member gets a row saying what to do (review C-F5).
    let message = row_message(&response, "mem_extra");
    assert!(
        message.contains("not materialized") && message.contains("gwz materialize"),
        "no row for the unmaterialized member: {message:?}"
    );
    assert!(!ws.root.join("extra").exists());
    let lock = crate::artifact::read_lock(&ws.root).unwrap();
    assert_eq!(lock.members["mem_extra"].path, "extra");
    assert!(
        crate::artifact::read_manifest(&ws.root)
            .unwrap()
            .members
            .iter()
            .any(|member| member.id == "mem_extra")
    );
}
