//! `local_clone::tests::privacy`: the family record is private by
//! enforcement, not by inheritance (LCM1.0c follow-up 4, lane C). On real
//! workspaces: after a create, every family file at the destination and at
//! the root is ignored by that workspace's own root repository and appears
//! in neither its status nor its index; the tracked `gwz.conf/` of both
//! carries no family binding, no family id and no host path; a source
//! whose `.git/info/exclude` was stripped still yields an ignoring
//! destination (the install writes the block rather than inheriting it) and
//! every create regenerates the root's; and no family binding is a git
//! remote, before or after `capture` and `repo sync` (design §6).

use std::fs;
use std::path::{Path, PathBuf};

use gwz_family_model::{
    ALLOCATION_MARKER_RELATIVE_PATH, INDEX_RELATIVE_PATH, LOCK_RELATIVE_PATH, POINTER_RELATIVE_PATH,
};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilyStore};

use super::fixture::{family_workspace, meta};
use crate::git::Git2Backend;
use crate::operation::NullSink;
use crate::workspace_ops::{handle_capture, handle_clone_local_workspace, handle_repo_sync};

/// The record: the index and the lock (a root's), the pointer and the
/// allocation marker (a clone's).
const FAMILY_FILES: [&str; 4] = [
    INDEX_RELATIVE_PATH,
    LOCK_RELATIVE_PATH,
    POINTER_RELATIVE_PATH,
    ALLOCATION_MARKER_RELATIVE_PATH,
];
const EXCLUDE_BEGIN: &str = "# BEGIN GWZ managed member repositories";

fn clone(source: &Path, name: &str) {
    handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        source,
        crate::CloneLocalWorkspaceRequest {
            meta: meta("req-clone-local"),
            name: name.to_owned(),
            dest: None,
            mode: crate::LocalCloneMode::Verbatim,
            branch: None,
            copy_source: None,
        },
        "op-clone",
        &NullSink,
    )
    .unwrap_or_else(|error| panic!("clone {name} from {}: {}", source.display(), error.message));
}

/// The workspace's root repository, exactly there (no upward search).
fn open(workspace: &Path) -> git2::Repository {
    git2::Repository::open_ext(
        workspace,
        git2::RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&std::ffi::OsStr>(),
    )
    .unwrap_or_else(|error| panic!("open {}: {error}", workspace.display()))
}

/// `git status --porcelain` as libgit2 reports it: every tracked change and
/// every untracked entry (untracked directories recursed), ignored entries
/// left out.
fn status_paths(repository: &git2::Repository) -> Vec<String> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    repository
        .statuses(Some(&mut options))
        .unwrap()
        .iter()
        .filter_map(|entry| entry.path().ok().map(str::to_owned))
        .collect()
}

/// `git ls-files`: every index entry.
fn index_paths(repository: &git2::Repository) -> Vec<String> {
    repository
        .index()
        .unwrap()
        .iter()
        .map(|entry| String::from_utf8(entry.path).unwrap())
        .collect()
}

fn read_exclude(workspace: &Path) -> String {
    fs::read_to_string(workspace.join(".git/info/exclude"))
        .unwrap_or_else(|error| panic!("{}: .git/info/exclude: {error}", workspace.display()))
}

/// The invariant at one workspace: every family path -- present or not --
/// is ignored by its root repository, none is in its status or its index,
/// and the managed block is in its exclude exactly once.
fn assert_record_private(workspace: &Path, label: &str) {
    let repository = open(workspace);
    for relative in FAMILY_FILES
        .iter()
        .chain([".gwz", ".gwz/locks/workspace.lock", ".gwz/merge/open.yaml"].iter())
    {
        assert!(
            repository.is_path_ignored(Path::new(relative)).unwrap(),
            "{label}: {relative} is ignored by {}",
            workspace.display()
        );
    }
    let status = status_paths(&repository);
    assert!(
        status.iter().all(|path| !path.starts_with(".gwz")),
        "{label}: no family entry in git status: {status:?}"
    );
    let index = index_paths(&repository);
    assert!(
        index.iter().all(|path| !path.starts_with(".gwz")),
        "{label}: no family entry in the index (git ls-files): {index:?}"
    );
    let exclude = read_exclude(workspace);
    assert_eq!(
        exclude.matches(EXCLUDE_BEGIN).count(),
        1,
        "{label}: the managed block once: {exclude}"
    );
    assert!(exclude.contains("/.gwz/\n"), "{label}: {exclude}");
}

/// Every file under `gwz.conf/`, the tracked workspace configuration.
fn conf_files(workspace: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(directory: &Path, into: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, into);
            } else {
                into.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    walk(&workspace.join(crate::workspace::WORKSPACE_DIR), &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "{}: gwz.conf/ is not empty",
        workspace.display()
    );
    files
}

/// Design §6: no family binding in the tracked configuration -- not the
/// family id, not a member's name or recorded path, not a host path of the
/// root or a clone, and no recorded remote URL naming one. (A remote the
/// operator configured on a source member is recorded by `repo sync` as it
/// always was, `file:` or not; that is the operator's binding, not the
/// family's, and the markers name only family workspaces.)
fn assert_conf_free_of_family_data(workspace: &Path, markers: &[&str], label: &str) {
    for (path, bytes) in conf_files(workspace) {
        let text = String::from_utf8_lossy(&bytes);
        for marker in markers {
            assert!(
                !text.contains(marker),
                "{label}: {} carries {marker:?}:\n{text}",
                path.display()
            );
        }
    }
    let manifest = crate::artifact::read_manifest(workspace).unwrap();
    for member in &manifest.members {
        for remote in &member.remotes {
            for marker in markers {
                assert!(
                    !remote.url.contains(marker),
                    "{label}: member {} remote {} names {marker:?}",
                    member.id,
                    remote.url
                );
            }
        }
    }
}

fn family_id(root: &Path) -> String {
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(root))
        .unwrap()
    {
        FamilyObservation::Family { view, .. } => view.family_id.to_string(),
        FamilyObservation::NoFamily => panic!("{} is in no family", root.display()),
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// After `root -> A`: the pointer and marker A holds, and the index and lock
/// the root holds, are ignored by each workspace's own root repository and
/// are in neither its status nor its index. The source's exclude carried an
/// operator line, which the copy kept and the install's write left alone:
/// A's exclude is byte-identical to the root's (the write is idempotent on
/// an inherited block), and the root's is what the operator left.
#[test]
fn every_family_file_is_ignored_by_the_destination_and_the_root_after_a_create() {
    let fixture = family_workspace("privacy-ignored");
    let source_exclude = fixture.root.join(".git/info/exclude");
    let mut text = fs::read_to_string(&source_exclude).unwrap();
    assert!(
        text.contains(EXCLUDE_BEGIN),
        "the workspace handler wrote the block"
    );
    text.push_str("# operator line\n/scratch-notes/\n");
    fs::write(&source_exclude, &text).unwrap();

    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    assert!(dest.join(POINTER_RELATIVE_PATH).is_file());
    assert!(dest.join(ALLOCATION_MARKER_RELATIVE_PATH).is_file());
    assert!(fixture.root.join(INDEX_RELATIVE_PATH).is_file());
    assert_record_private(&dest, "A");
    assert_record_private(&fixture.root, "root");
    // The member's own repository holds no family entry either.
    let app = open(&dest.join("app"));
    assert!(index_paths(&app).iter().all(|path| !path.contains(".gwz")));
    assert!(status_paths(&app).iter().all(|path| !path.contains(".gwz")));

    let dest_exclude = read_exclude(&dest);
    assert!(
        dest_exclude.contains("# operator line\n/scratch-notes/\n"),
        "{dest_exclude}"
    );
    assert!(dest_exclude.contains("/app/\n"), "{dest_exclude}");
    assert_eq!(
        dest_exclude,
        fs::read_to_string(&source_exclude).unwrap(),
        "verbatim inherited the file and the install's write was a no-op on it"
    );
    assert_eq!(
        fs::read_to_string(&source_exclude).unwrap(),
        text,
        "the root's exclude is byte-identical to what the operator left: regenerating an \
         identical block writes nothing"
    );
}

/// Design §6 ("family bindings are not persisted as git remotes; `repo
/// sync` / capture must not write family URLs into `gwz.yml`"): the family
/// data exists -- the pointer names the family id and the root, the index
/// names the member and its recorded path -- and every byte of it is
/// confined to `.gwz/`; the tracked `gwz.conf/` of the root and of A carry
/// none of it, nor any host path.
#[test]
fn the_tracked_configuration_carries_no_family_binding() {
    let fixture = family_workspace("privacy-conf");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let family_id = family_id(&fixture.root);
    let pointer = fs::read_to_string(dest.join(POINTER_RELATIVE_PATH)).unwrap();
    let index = fs::read_to_string(fixture.root.join(INDEX_RELATIVE_PATH)).unwrap();
    assert!(pointer.contains(&family_id), "{pointer}");
    assert!(pointer.contains(&display(&fixture.root)), "{pointer}");
    assert!(index.contains(&family_id), "{index}");
    assert!(index.contains("../root-A"), "{index}");

    let root_path = display(&fixture.root);
    let dest_path = display(&dest);
    let tree_path = display(fixture.tree.path());
    let markers = [
        family_id.as_str(),
        "../root-A",
        "family",
        root_path.as_str(),
        dest_path.as_str(),
        tree_path.as_str(),
    ];
    assert_conf_free_of_family_data(&fixture.root, &markers, "root");
    assert_conf_free_of_family_data(&dest, &markers, "A");
    // The manifest A publishes is the source's, byte for byte, so the
    // absence above is the source's too; and the copied lock names commits
    // and branches, never a path outside the workspace.
    assert_eq!(
        fs::read(dest.join("gwz.conf/gwz.yml")).unwrap(),
        fs::read(fixture.root.join("gwz.conf/gwz.yml")).unwrap()
    );
}

/// The block is written by the install, not inherited: A is a member, not
/// the root, so nothing regenerates its exclude once it is stripped, and a
/// copy of A carries no block -- yet B, cloned from A, ignores the record,
/// while A (the source, untouched) still shows it. This is the case a
/// constructed destination (clean, bare) is in from birth.
#[test]
fn a_destination_gets_its_exclude_from_the_install_not_from_the_source() {
    let fixture = family_workspace("privacy-install-writes");
    clone(&fixture.root, "A");
    let a = fixture.sibling("A");
    let a_exclude = a.join(".git/info/exclude");
    fs::remove_file(&a_exclude).unwrap();
    let stripped = open(&a);
    assert!(
        !stripped
            .is_path_ignored(Path::new(POINTER_RELATIVE_PATH))
            .unwrap(),
        "precondition: with the exclude gone, A's pointer is visible"
    );
    assert!(
        status_paths(&stripped)
            .iter()
            .any(|path| path.starts_with(".gwz/")),
        "precondition: git status at A shows the record"
    );
    drop(stripped);

    clone(&a, "B");
    let b = fixture.sibling("B");
    assert_record_private(&b, "B");
    assert!(
        !a_exclude.exists(),
        "the source is untouched: A still has no exclude, so B's block came from the install"
    );
    assert!(read_exclude(&b).contains("/app/\n"));
}

/// Every create regenerates the family root's block under the family lock,
/// before founding or rewriting the index: a root whose exclude was
/// stripped founds a family whose index is ignored from birth (and the
/// copy to A inherits the regenerated file); stripped again, a clone of A
/// -- whose source is A, not the root -- regenerates it once more.
#[test]
fn every_create_regenerates_the_roots_exclude() {
    let fixture = family_workspace("privacy-root");
    let root_exclude = fixture.root.join(".git/info/exclude");
    fs::remove_file(&root_exclude).unwrap();
    assert!(
        !open(&fixture.root)
            .is_path_ignored(Path::new(INDEX_RELATIVE_PATH))
            .unwrap(),
        "precondition: the index would be visible"
    );

    clone(&fixture.root, "A");
    assert!(fixture.root.join(INDEX_RELATIVE_PATH).is_file());
    assert_record_private(&fixture.root, "root, founded");
    assert_record_private(&fixture.sibling("A"), "A");
    assert!(read_exclude(&fixture.root).contains("/app/\n"));

    fs::remove_file(&root_exclude).unwrap();
    clone(&fixture.sibling("A"), "B");
    assert_record_private(&fixture.root, "root, after a clone of a clone");
    assert_record_private(&fixture.sibling("B"), "B");
}

/// Design §6 and §4.1's last row, end to end on a real create: the source
/// member carries an ordinary https origin, a filesystem-path remote at a
/// sibling checkout (the shape a family binding would take if it were a
/// remote) and a `file:` mirror; at A no remote URL is a `file:` URL or a
/// filesystem path, none names the source, the root or A, the https origin
/// stays, and the source keeps all three. Then `gwz capture` and `gwz repo
/// sync` on the family root and on A write no family data into `gwz.conf/`.
/// The URL rule itself is
/// `adapters::git_config::tests::filesystem_and_credential_urls_go_and_ordinary_origins_stay`;
/// this row proves the port runs on every repository of a real destination.
#[test]
fn no_family_binding_is_a_git_remote_and_capture_and_repo_sync_write_none() {
    let fixture = family_workspace("privacy-remotes");
    let sibling = fixture.tree.repo("sibling-checkout");
    let app = fixture.workspace.member("app");
    app.remote("sibling", &sibling);
    {
        let repository = app.open();
        repository
            .remote("origin", "https://example.invalid/org/app.git")
            .unwrap();
        repository
            .remote("mirror", "file:///srv/mirrors/app.git")
            .unwrap();
    }
    let sibling_path = display(sibling.path());

    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let root_path = display(&fixture.root);
    let dest_path = display(&dest);
    for relative in ["", "app"] {
        let repository = open(&dest.join(relative));
        let names = repository.remotes().unwrap();
        for name in names.iter() {
            let Ok(Some(name)) = name else {
                continue;
            };
            let url: String = repository
                .find_remote(name)
                .ok()
                .and_then(|remote| remote.url().ok().map(str::to_owned))
                .unwrap_or_default();
            assert!(!url.starts_with("file:"), "A/{relative}: {name} = {url}");
            assert!(
                !Path::new(&url).is_absolute(),
                "A/{relative}: {name} = {url}"
            );
            for named in [&sibling_path, &root_path, &dest_path] {
                assert!(
                    !url.contains(named.as_str()),
                    "A/{relative}: {name} = {url}"
                );
            }
        }
    }
    let dest_app = open(&dest.join("app"));
    assert_eq!(
        dest_app.find_remote("origin").unwrap().url().ok(),
        Some("https://example.invalid/org/app.git"),
        "the ordinary origin stays"
    );
    // libgit2 lists a remote by its `url`/`pushurl` keys, so the loop above
    // never saw the two that were stripped; their URL keys are gone from
    // the copied configuration itself, and their fetch refspecs stay.
    let config = dest_app.config().unwrap();
    for stripped in ["sibling", "mirror"] {
        assert!(
            config
                .get_string(&format!("remote.{stripped}.url"))
                .is_err(),
            "A/app: remote.{stripped}.url is removed"
        );
        assert!(
            config
                .get_string(&format!("remote.{stripped}.fetch"))
                .is_ok(),
            "A/app: remote.{stripped}.fetch is copied configuration and stays"
        );
    }
    drop(config);
    drop(dest_app);
    let source_app = app.open();
    assert!(
        source_app
            .find_remote("sibling")
            .unwrap()
            .url()
            .unwrap()
            .contains(&sibling_path),
        "the source keeps its path remote"
    );
    assert_eq!(
        source_app.find_remote("mirror").unwrap().url().ok(),
        Some("file:///srv/mirrors/app.git"),
        "and its file: mirror"
    );
    drop(source_app);

    let family_id = family_id(&fixture.root);
    let markers = [
        family_id.as_str(),
        "../root-A",
        "family",
        root_path.as_str(),
        dest_path.as_str(),
    ];
    let backend = Git2Backend::without_credential_helpers();
    for (workspace, label) in [(&fixture.root, "root"), (&dest, "A")] {
        handle_capture(
            &backend,
            workspace,
            crate::CaptureRequest {
                meta: meta("req-capture"),
            },
            "op-capture",
        )
        .unwrap_or_else(|error| panic!("{label}: capture: {}", error.message));
        assert_conf_free_of_family_data(workspace, &markers, &format!("{label} after capture"));
        let synced = handle_repo_sync(
            &backend,
            workspace,
            crate::RepoSyncRequest {
                meta: meta("req-sync"),
            },
            "op-sync",
        )
        .unwrap_or_else(|error| panic!("{label}: repo sync: {}", error.message));
        for member in &synced.response.members {
            eprintln!(
                "{label}: repo sync {} {:?} {:?}",
                member.member_path,
                member.status,
                member.error.as_ref().map(|error| &error.message)
            );
        }
        assert_conf_free_of_family_data(workspace, &markers, &format!("{label} after repo sync"));
        assert_record_private(workspace, &format!("{label} after capture and repo sync"));
    }
}
