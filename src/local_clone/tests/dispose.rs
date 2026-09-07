//! `local_clone::tests::dispose`: `gwz local dispose <name>` on a real
//! family -- ordinary deletion refusing on every design §12 row before it
//! deletes on the one row that permits it -- plus `--keep`, `disband`, and
//! the disposal ports exercised directly against real repositories.
//!
//! Every refusal row asserts the same three things: the typed code, that
//! the target tree is byte-identical before and after (so the remover was
//! never reached), and that the row still stands.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Cancellation;
use gwz_family_model::{
    FamilyChange, MarkerObservation, MemberName, MemberState, PointerObservation, TargetObservation,
};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilySession, FamilyStore};
use gwz_local_disposal::{DisposalPorts, DisposeEffect, HistoryAnswer, HistoryQuery};
use gwz_repo_contract::{Observation, RepoKey, WorkKind};

use super::fixture::{FamilyFixture, clean_family_workspace, family_workspace, meta};
use crate::git::Git2Backend;
use crate::local_clone::adapters::disposal::CoreDisposalPorts;
use crate::local_clone::create;
use crate::local_clone::request::validate_clone_local;
use crate::model::{ErrorCode, ModelError};
use crate::operation::NullSink;
use crate::workspace_ops::{handle_clone_local_workspace, handle_local_family, open_merge_probe};

mod integrity;

fn clone_request(name: &str) -> crate::CloneLocalWorkspaceRequest {
    crate::CloneLocalWorkspaceRequest {
        meta: meta("req-clone-local"),
        name: name.to_owned(),
        dest: None,
        mode: crate::LocalCloneMode::Verbatim,
        branch: None,
        copy_source: None,
    }
}

fn family_request(
    op: crate::LocalFamilyOp,
    name: Option<&str>,
    keep: Option<bool>,
) -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        meta: meta("req-local-family"),
        op,
        name: name.map(ToOwned::to_owned),
        keep,
        force_hazards: Vec::new(),
    }
}

/// `gwz local dispose <name> [--force <hazards>]`.
fn delete_request(name: &str, force: &[&str]) -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        force_hazards: force.iter().map(|hazard| (*hazard).to_owned()).collect(),
        ..family_request(crate::LocalFamilyOp::Dispose, Some(name), None)
    }
}

fn clone(root: &Path, name: &str) {
    handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        root,
        clone_request(name),
        "op-clone",
        &NullSink,
    )
    .expect("clone");
}

fn try_local(
    start: &Path,
    request: crate::LocalFamilyRequest,
) -> Result<crate::LocalFamilyResponse, ModelError> {
    handle_local_family(
        &Git2Backend::without_credential_helpers(),
        start,
        request,
        "op-local",
        &NullSink,
    )
}

fn local(start: &Path, request: crate::LocalFamilyRequest) -> crate::LocalFamilyResponse {
    try_local(start, request).expect("local family operation")
}

fn refuse(start: &Path, request: crate::LocalFamilyRequest) -> ModelError {
    try_local(start, request).expect_err("the operation refuses")
}

fn family(root: &Path) -> Option<gwz_family_model::FamilyView> {
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(root))
        .expect("the family reads")
    {
        FamilyObservation::Family { view, .. } => Some(view),
        FamilyObservation::NoFamily => None,
    }
}

fn listed_names(start: &Path) -> Vec<String> {
    local(
        start,
        family_request(crate::LocalFamilyOp::List, None, None),
    )
    .members
    .into_iter()
    .map(|member| member.name)
    .collect()
}

/// Every file under `root` with its bytes, except the two family metadata
/// files a detach removes.
fn files_except_family_metadata(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    tree_bytes(root)
        .into_iter()
        .filter(|(path, _)| {
            let relative = path.strip_prefix(root).unwrap();
            relative != Path::new(gwz_family_model::POINTER_RELATIVE_PATH)
                && relative != Path::new(gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH)
        })
        .collect()
}

/// Every regular file under `root` with its bytes, sorted: the whole tree,
/// family metadata included, so a comparison before and after a refusal
/// proves nothing at all was removed or rewritten.
fn tree_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// The root's files that a disposal may not touch: everything but the index
/// and the lock file the family operation itself holds.
fn root_files_except_the_index(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    tree_bytes(root)
        .into_iter()
        .filter(|(path, _)| {
            let relative = path.strip_prefix(root).unwrap();
            relative != Path::new(gwz_family_model::INDEX_RELATIVE_PATH)
                && relative != Path::new(gwz_family_model::LOCK_RELATIVE_PATH)
        })
        .collect()
}

fn assert_refused_without_effect(
    fixture: &FamilyFixture,
    dest: &Path,
    before: &[(PathBuf, Vec<u8>)],
    error: &ModelError,
    code: ErrorCode,
    needles: &[&str],
) {
    assert_eq!(error.code, code, "{}", error.message);
    for needle in needles {
        assert!(
            error.message.contains(needle),
            "expected `{needle}` in: {}",
            error.message
        );
    }
    assert_eq!(
        tree_bytes(dest),
        before,
        "the refusal touched the target tree: {}",
        error.message
    );
    let view = family(&fixture.root).expect("the family stands");
    let name = dest
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_start_matches("root-");
    assert!(
        view.member(name).is_some(),
        "the row `{name}` still stands after: {}",
        error.message
    );
}

/// One commit on the current branch of the repository at `path`, adding
/// `file`; the fixture signature keeps the id deterministic.
fn commit_in(path: &Path, file: &str, content: &str, message: &str) -> String {
    let repository = git2::Repository::open(path).unwrap();
    fs::write(path.join(file), content).unwrap();
    let mut index = repository.index().unwrap();
    index.add_path(Path::new(file)).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repository.find_tree(tree_id).unwrap();
    let parent = repository.head().unwrap().peel_to_commit().unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    repository
        .commit(
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

fn head_of(path: &Path) -> String {
    git2::Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

/// `git reset --hard <oid>`: the branch, the index and the worktree move
/// back; only the reflog still names the abandoned commit.
fn reset_hard(path: &Path, oid: &str) {
    let repository = git2::Repository::open(path).unwrap();
    let object = repository
        .find_object(git2::Oid::from_str(oid).unwrap(), None)
        .unwrap();
    repository
        .reset(&object, git2::ResetType::Hard, None)
        .unwrap();
}

/// A native stash of one tracked edit: the worktree is clean afterwards and
/// `refs/stash` holds the only copy of the edit.
fn stash_in(path: &Path) -> String {
    fs::write(path.join("README"), b"stashed edit\n").unwrap();
    let mut repository = git2::Repository::open(path).unwrap();
    repository
        .stash_save(
            &gwz_local_testrepo::fixture_signature(),
            "wip",
            Some(git2::StashFlags::DEFAULT),
        )
        .unwrap()
        .to_string()
}

/// A tracked path marked `skip-worktree` and then removed, in a repository
/// with no sparse checkout: the one suppressed shape the classifier cannot
/// interpret (`UnsupportedIndexFlag`), never clean.
fn skip_worktree_absent(path: &Path, file: &str) {
    let repository = git2::Repository::open(path).unwrap();
    let mut index = repository.index().unwrap();
    let mut entry = index.get_path(Path::new(file), 0).expect("tracked");
    entry.flags |= git2::IndexEntryFlag::EXTENDED.bits();
    entry.flags_extended |= git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits();
    index.add(&entry).unwrap();
    index.write().unwrap();
    fs::remove_file(path.join(file)).unwrap();
}

/// Mark `name` `disposing` through the store session, the shape an
/// interrupted deletion leaves; the lock is released again afterwards.
fn mark_disposing(root: &Path, name: &str) {
    let store = gwz_family_store::YamlFamilyStore::new();
    let mut session = store.try_lock(&FamilyLocation::new(root)).unwrap();
    let view = session.reread().unwrap().unwrap();
    let name = MemberName::parse(name).unwrap();
    let allocation = view.members[&name].allocation_id.clone();
    session
        .apply(&FamilyChange::MarkDisposing {
            name,
            expected_allocation: allocation,
        })
        .unwrap();
}

struct CancelWhenExists(PathBuf);

impl Cancellation for CancelWhenExists {
    fn is_cancelled(&self) -> bool {
        self.0.exists()
    }
}

/// Design §5.2 step 2: `--keep` removes the matching pointer and row and
/// nothing else; every other file stays byte for byte, and the lane is no
/// longer a member.
#[test]
fn keep_detaches_a_and_leaves_every_file() {
    let fixture = family_workspace("dispose-keep");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = files_except_family_metadata(&dest);
    assert!(!before.is_empty());

    let response = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("detached local clone `A`"), "{message}");
    assert!(message.contains("every file at"), "{message}");
    assert!(response.members.is_empty() && response.root_path.is_none());

    assert!(
        !dest.join(".gwz/family-root").exists(),
        "the pointer is gone"
    );
    assert!(
        !dest.join(".gwz/local-clone-allocation").exists(),
        "the marker is gone"
    );
    assert_eq!(
        files_except_family_metadata(&dest),
        before,
        "every other file is retained byte for byte"
    );
    let view = family(&fixture.root).expect("the family stands");
    assert!(view.members.is_empty(), "the row is gone");
    assert_eq!(listed_names(&fixture.root), ["root"], "only the root lists");
    // A detached tree is in no family: listing from it is empty.
    let from_a = local(
        &dest,
        family_request(crate::LocalFamilyOp::List, None, None),
    );
    assert!(from_a.members.is_empty() && from_a.root_path.is_none());
    // A second keep has nothing to detach.
    let error = refuse(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert_eq!(error.code, ErrorCode::MemberNotFound, "{}", error.message);
}

/// Design §3.1, §12 ("Dirty/open/incomplete lane with keep: detach matching
/// metadata and retain all remaining files"): keep works on an interrupted
/// create and repairs nothing.
#[test]
fn keep_detaches_an_incomplete_target_and_retains_its_remainder() {
    let fixture = family_workspace("dispose-keep-incomplete");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &CancelWhenExists(dest.join(".gwz/family-root")),
    )
    .expect_err("interrupted before the manifest");
    let before = files_except_family_metadata(&dest);

    let response = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert!(
        response
            .response
            .meta
            .message
            .as_deref()
            .is_some_and(|message| message.contains("detached local clone `A`"))
    );
    assert!(family(&fixture.root).unwrap().members.is_empty());
    assert!(!dest.join(".gwz/family-root").exists());
    assert_eq!(files_except_family_metadata(&dest), before);
    assert!(
        !dest.join("gwz.conf/gwz.yml").exists(),
        "keep does not repair the tree: no manifest appears"
    );
}

/// Design §3.1, §8.6, §12 ("Disband after partial pointer removal"):
/// disband removes every pointer and marker and finally the index, leaves
/// every tree, works from a clone through its pointer, and is a no-op when
/// repeated.
#[test]
fn disband_removes_pointers_and_the_index_and_leaves_the_trees() {
    let fixture = family_workspace("dispose-disband");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let a = fixture.sibling("A");
    let b = fixture.sibling("B");
    let before_root = files_except_family_metadata(&fixture.root)
        .into_iter()
        .filter(|(path, _)| !path.ends_with("local-family.yml"))
        .collect::<Vec<_>>();
    let before_a = files_except_family_metadata(&a);
    let before_b = files_except_family_metadata(&b);
    // An interrupted earlier disband: A's pointer is already gone.
    fs::remove_file(a.join(".gwz/family-root")).unwrap();

    let response = local(
        &b,
        family_request(crate::LocalFamilyOp::Disband, None, None),
    );
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("disbanded local family"), "{message}");
    assert!(
        message.contains("1 pointer(s) and 2 marker(s) removed"),
        "{message}"
    );
    assert!(message.contains("index removed"), "{message}");

    assert!(family(&fixture.root).is_none(), "the index is gone");
    assert!(!fixture.root.join(".gwz/local-family.yml").exists());
    for (dest, before) in [(&a, &before_a), (&b, &before_b)] {
        assert!(!dest.join(".gwz/family-root").exists());
        assert!(!dest.join(".gwz/local-clone-allocation").exists());
        assert_eq!(
            &files_except_family_metadata(dest),
            before,
            "{}",
            dest.display()
        );
    }
    assert_eq!(
        files_except_family_metadata(&fixture.root)
            .into_iter()
            .filter(|(path, _)| !path.ends_with("local-family.yml"))
            .collect::<Vec<_>>(),
        before_root,
        "the root's own files are retained"
    );
    let again = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Disband, None, None),
    );
    assert_eq!(
        again.response.meta.aggregate_status,
        crate::AggregateStatus::Noop,
        "an explicit repeat is a no-op"
    );
}

/// Design §5.1, §12 ("Dirty ... lane"): the dirt a verbatim clone carried
/// -- an untracked note in the member, an unstaged edit at the root -- is
/// a known `dirty` hazard. Ordinary deletion refuses it as
/// `unwaived_hazard`, names the paths, and touches nothing; naming the
/// *other* two hazards does not waive it.
#[test]
fn a_dirty_lane_refuses_ordinary_deletion_naming_the_dirt_and_nothing_is_removed() {
    let fixture = family_workspace("dispose-dirty");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = tree_bytes(&dest);

    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_refused_without_effect(
        &fixture,
        &dest,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &[
            "<dirty>",
            "notes.txt",
            "README",
            "nothing was removed",
            "--keep",
        ],
    );
    assert!(
        !error.message.contains("<unpreserved-history>"),
        "a fresh verbatim clone's history is preserved in its source: {}",
        error.message
    );

    let error = refuse(
        &fixture.root,
        delete_request("A", &["open-merge", "unpreserved-history"]),
    );
    assert_refused_without_effect(
        &fixture,
        &dest,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &["<dirty>"],
    );
    assert!(
        dest.join(".gwz/family-root").is_file(),
        "the pointer still stands"
    );
    assert_eq!(
        family(&fixture.root).unwrap().member("A").unwrap().1.state,
        MemberState::Ready,
        "the row was never marked disposing"
    );
}

/// Design §5.1 and §12 ("Unique root/branch/reflog/tag/stash history:
/// refuse ordinary deletion until covered in a survivor"): a commit only
/// the lane holds, a commit only its reflog retains, and a native stash
/// each refuse as `unpreserved-history`, naming the object, and nothing is
/// removed. The clean lane beside them is the control: the refusal is the
/// history, not the fixture.
#[test]
fn a_lane_with_a_unique_commit_reflog_entry_or_stash_refuses() {
    let fixture = clean_family_workspace("dispose-unique-history");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    clone(&fixture.root, "C");

    // A: a commit made only in the lane's member.
    let a = fixture.sibling("A");
    let only_in_a = commit_in(&a.join("app"), "feature.txt", "from A\n", "only in A");
    let before = tree_bytes(&a);
    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_refused_without_effect(
        &fixture,
        &a,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &[
            "<unpreserved-history>",
            only_in_a.as_str(),
            "nothing was removed",
        ],
    );
    assert!(
        !error.message.contains("<dirty>"),
        "a clean lane is not dirty: {}",
        error.message
    );

    // B: the same commit, then `reset --hard` back -- the branch, index and
    // worktree are as cloned; only the reflog still retains the commit.
    let b = fixture.sibling("B");
    let base = head_of(&b.join("app"));
    let abandoned = commit_in(&b.join("app"), "feature.txt", "from B\n", "abandoned in B");
    reset_hard(&b.join("app"), &base);
    assert_eq!(head_of(&b.join("app")), base);
    let before = tree_bytes(&b);
    let error = refuse(&fixture.root, delete_request("B", &[]));
    assert_refused_without_effect(
        &fixture,
        &b,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &["<unpreserved-history>", abandoned.as_str(), "Reflog"],
    );

    // C: a native stash: the worktree is clean, `refs/stash` holds the
    // edit, and the stash is both `dirty` (a native stash entry) and
    // unpreserved (its commit is in no survivor).
    let c = fixture.sibling("C");
    let stashed = stash_in(&c.join("app"));
    let before = tree_bytes(&c);
    let error = refuse(&fixture.root, delete_request("C", &[]));
    assert_refused_without_effect(
        &fixture,
        &c,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &[
            "<dirty>",
            "native stash",
            "<unpreserved-history>",
            stashed.as_str(),
        ],
    );
    // Naming the dirt alone leaves the history unwaived.
    let error = refuse(&fixture.root, delete_request("C", &["dirty"]));
    assert_refused_without_effect(
        &fixture,
        &c,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &["<unpreserved-history>"],
    );
    assert!(!error.message.contains("<dirty>"), "{}", error.message);
    assert_eq!(listed_names(&fixture.root), ["root", "A", "B", "C"]);
}

/// Design §5.1 ("Unknown layouts/evidence refuse ordinary deletion") and
/// §12 ("Unknown or oversized work/history inventory: refuse ordinary
/// deletion before disposing"): a gwz stash record this build does not
/// decode, a suppressed index entry the classifier cannot interpret, and a
/// nested layout the inspector refuses each answer `unknown_evidence`, and
/// **no** force name waives any of them. `--keep` still detaches.
#[test]
fn unknown_work_or_history_refuses_and_no_force_name_waives_it() {
    let fixture = clean_family_workspace("dispose-unknown");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    clone(&fixture.root, "C");
    const ALL: [&str; 3] = ["open-merge", "dirty", "unpreserved-history"];

    // A: a gwz stash coordination record (design §5.1: needs surviving
    // interpretable copies and referenced objects; this build refuses).
    let a = fixture.sibling("A");
    let bundles = a.join(crate::stash::STASH_BUNDLE_DIR);
    fs::create_dir_all(&bundles).unwrap();
    fs::write(bundles.join("stash_00000001.yaml"), b"schema: x\n").unwrap();
    let before = tree_bytes(&a);
    for force in [&[][..], &ALL[..]] {
        let error = refuse(&fixture.root, delete_request("A", force));
        assert_refused_without_effect(
            &fixture,
            &a,
            &before,
            &error,
            ErrorCode::UnknownEvidence,
            &[
                "gwz stash record",
                "no force name waives",
                "--keep",
                "nothing was removed",
            ],
        );
    }

    // B: a tracked path marked skip-worktree and absent, with no sparse
    // checkout to make the absence valid.
    let b = fixture.sibling("B");
    skip_worktree_absent(&b.join("app"), "README");
    let before = tree_bytes(&b);
    for force in [&[][..], &ALL[..]] {
        let error = refuse(&fixture.root, delete_request("B", force));
        assert_refused_without_effect(
            &fixture,
            &b,
            &before,
            &error,
            ErrorCode::UnknownEvidence,
            &["README", "skip-worktree", "no force name waives"],
        );
    }

    // C: an unmanaged nested repository whose `.git` is a gitfile the
    // inspector refuses (design §4.0), so the tree's layout is unknown.
    let c = fixture.sibling("C");
    fs::create_dir_all(c.join("vendor/thing")).unwrap();
    fs::write(
        c.join("vendor/thing/.git"),
        b"gitdir: /nonexistent/elsewhere\n",
    )
    .unwrap();
    let before = tree_bytes(&c);
    for force in [&[][..], &ALL[..]] {
        let error = refuse(&fixture.root, delete_request("C", force));
        assert_refused_without_effect(
            &fixture,
            &c,
            &before,
            &error,
            ErrorCode::UnknownEvidence,
            &["nested:vendor/thing", "no force name waives"],
        );
    }

    // Keep is the way out for all three.
    for name in ["A", "B", "C"] {
        local(
            &fixture.root,
            family_request(crate::LocalFamilyOp::Dispose, Some(name), Some(true)),
        );
    }
    assert_eq!(listed_names(&fixture.root), ["root"]);
    assert!(a.join("gwz.conf/gwz.yml").is_file() && c.join("vendor/thing/.git").is_file());
}

/// Design §5.2 step 3 and §8.4: each known hazard refuses until its own
/// name is given, the names accumulate, and with all three named an intact
/// ready tree is deleted -- and only that tree.
#[test]
fn each_known_hazard_refuses_without_its_name_and_proceeds_with_it() {
    let fixture = clean_family_workspace("dispose-force-names");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let a = fixture.sibling("A");
    let app = a.join("app");
    // Dirt, an unfinished native operation, and a unique commit, all in A.
    fs::write(app.join("scratch.txt"), b"unsaved\n").unwrap();
    let unique = commit_in(&app, "feature.txt", "from A\n", "only in A");
    fs::write(app.join(".git/MERGE_HEAD"), format!("{unique}\n")).unwrap();
    fs::write(app.join(".git/MERGE_MSG"), b"fixture merge\n").unwrap();
    let before = tree_bytes(&a);
    let before_b = tree_bytes(&fixture.sibling("B"));
    let before_root = root_files_except_the_index(&fixture.root);

    for (force, still, waived) in [
        (
            &[][..],
            &["<open-merge>", "<dirty>", "<unpreserved-history>"][..],
            &[][..],
        ),
        (
            &["dirty"][..],
            &["<open-merge>", "<unpreserved-history>"][..],
            &["<dirty>"][..],
        ),
        (
            &["dirty", "open-merge"][..],
            &["<unpreserved-history>"][..],
            &["<dirty>", "<open-merge>"][..],
        ),
        (
            &["unpreserved-history", "open-merge"][..],
            &["<dirty>"][..],
            &["<open-merge>", "<unpreserved-history>"][..],
        ),
    ] {
        let error = refuse(&fixture.root, delete_request("A", force));
        assert_refused_without_effect(
            &fixture,
            &a,
            &before,
            &error,
            ErrorCode::UnwaivedHazard,
            still,
        );
        for name in waived {
            assert!(
                !error.message.contains(name),
                "{force:?} waived {name}, yet it is still named: {}",
                error.message
            );
        }
    }

    let response = local(
        &fixture.root,
        delete_request("A", &["open-merge", "dirty", "unpreserved-history"]),
    );
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("deleted local clone `A`"), "{message}");
    assert!(
        message.contains("forced past: open-merge, dirty, unpreserved-history"),
        "{message}"
    );
    assert!(!a.exists(), "the tree is gone");
    assert_eq!(listed_names(&fixture.root), ["root", "B"]);
    assert_eq!(
        tree_bytes(&fixture.sibling("B")),
        before_b,
        "B is untouched"
    );
    assert_eq!(
        root_files_except_the_index(&fixture.root),
        before_root,
        "the root is untouched"
    );
}

/// Design §5.2 step 3: an empty, unknown or repeated force name is a
/// malformed request, refused before any observation, and so is
/// `--keep` with a force name.
#[test]
fn a_force_with_an_empty_unknown_or_repeated_name_refuses_before_any_effect() {
    let fixture = clean_family_workspace("dispose-force-shape");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = tree_bytes(&dest);
    for force in [
        &[""][..],
        &["all"][..],
        &["true"][..],
        &["Dirty"][..],
        &["dirty", "dirty"][..],
        &["dirty", "open-merge", "dirty"][..],
    ] {
        let error = refuse(&fixture.root, delete_request("A", force));
        assert_refused_without_effect(
            &fixture,
            &dest,
            &before,
            &error,
            ErrorCode::InvalidRequest,
            &["hazard"],
        );
    }
    let error = refuse(
        &fixture.root,
        crate::LocalFamilyRequest {
            keep: Some(true),
            ..delete_request("A", &["dirty"])
        },
    );
    assert_refused_without_effect(
        &fixture,
        &dest,
        &before,
        &error,
        ErrorCode::InvalidRequest,
        &["mutually exclusive"],
    );
}

/// Design §5.2 step 1 and §8.4: the root is never deleted, the member the
/// operator stands in is never deleted, a moved root and a replaced target
/// are path mismatches, and none of these is forceable.
#[test]
fn the_root_the_working_directory_a_moved_root_and_a_replaced_target_refuse() {
    let fixture = clean_family_workspace("dispose-validation");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let before = tree_bytes(&dest);
    const ALL: [&str; 3] = ["open-merge", "dirty", "unpreserved-history"];

    let error = refuse(&fixture.root, delete_request("root", &ALL));
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.message);
    assert!(
        error.message.contains("never disposed"),
        "{}",
        error.message
    );

    // Standing inside A, addressing the family through A's pointer.
    let error = refuse(&dest.join("app"), delete_request("A", &ALL));
    assert_refused_without_effect(
        &fixture,
        &dest,
        &before,
        &error,
        ErrorCode::InvalidRequest,
        &["working directory"],
    );

    // A's allocation marker replaced: what stands at the recorded path is
    // not the recorded member.
    let marker = dest.join(gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH);
    let original = fs::read(&marker).unwrap();
    fs::write(
        &marker,
        String::from_utf8(original.clone())
            .unwrap()
            .replace("alloc_", "alloc_0"),
    )
    .unwrap();
    let replaced = tree_bytes(&dest);
    let error = refuse(&fixture.root, delete_request("A", &ALL));
    assert_refused_without_effect(
        &fixture,
        &dest,
        &replaced,
        &error,
        ErrorCode::InvalidRequest,
        &["not the recorded target"],
    );
    fs::write(&marker, original).unwrap();
    assert_eq!(tree_bytes(&dest), before);

    // The root moved: A's pointer names the root's old path, so A no longer
    // matches its row from the root's new one.
    let moved = fixture.tree.path().join("root-moved");
    fs::rename(&fixture.root, &moved).unwrap();
    let error = refuse(&moved, delete_request("A", &ALL));
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.message);
    assert!(
        error.message.contains("not the recorded target"),
        "{}",
        error.message
    );
    assert_eq!(tree_bytes(&dest), before, "nothing was removed");
    assert!(
        family(&moved).unwrap().member("A").is_some(),
        "the row still stands"
    );
    fs::rename(&moved, &fixture.root).unwrap();
    assert_eq!(listed_names(&fixture.root), ["root", "A"]);
}

/// Design §5.2 step 3 and §12: an interrupted create (`creating`) and an
/// interrupted deletion (`disposing`) are retained and are not forceable
/// through ordinary deletion, but `--keep` detaches them.
#[test]
fn a_creating_or_disposing_row_refuses_ordinary_deletion_but_accepts_keep() {
    const ALL: [&str; 3] = ["open-merge", "dirty", "unpreserved-history"];
    // Creating: the install cancelled after the pointer, before the manifest.
    let fixture = clean_family_workspace("dispose-creating");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &CancelWhenExists(dest.join(".gwz/family-root")),
    )
    .expect_err("interrupted before the manifest");
    assert_eq!(
        family(&fixture.root).unwrap().member("A").unwrap().1.state,
        MemberState::Creating
    );
    let before = tree_bytes(&dest);
    for force in [&[][..], &ALL[..]] {
        let error = refuse(&fixture.root, delete_request("A", force));
        assert_refused_without_effect(
            &fixture,
            &dest,
            &before,
            &error,
            ErrorCode::InvalidRequest,
            &["creating"],
        );
    }
    local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert!(family(&fixture.root).unwrap().members.is_empty());
    assert_eq!(
        files_except_family_metadata(&dest).len() + 2,
        before.len(),
        "only the pointer and the marker went"
    );

    // Disposing: the row an interrupted deletion leaves.
    let fixture = clean_family_workspace("dispose-disposing");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    mark_disposing(&fixture.root, "A");
    let before = tree_bytes(&dest);
    for force in [&[][..], &ALL[..]] {
        let error = refuse(&fixture.root, delete_request("A", force));
        assert_refused_without_effect(
            &fixture,
            &dest,
            &before,
            &error,
            ErrorCode::InvalidRequest,
            &["disposing"],
        );
    }
    let listed = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::List, None, None),
    );
    assert_eq!(
        listed.members[1].recorded_state,
        crate::LocalMemberState::Disposing
    );
    local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert!(family(&fixture.root).unwrap().members.is_empty());
    assert!(
        dest.join("gwz.conf/gwz.yml").is_file(),
        "keep retained the tree"
    );
}

/// Design §5.2 steps 4-5 and §12 ("Error/interruption during deletion:
/// stop; retain/report remainder; no later automatic deletion"): a removal
/// that fails part-way stops, reports what remains, leaves the row
/// `disposing`, and no later ordinary dispose replays it; `--keep` detaches
/// the remainder.
#[cfg(unix)]
#[test]
fn a_removal_error_part_way_stops_reports_the_remainder_and_leaves_the_row_disposing() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = clean_family_workspace("dispose-removal-error");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let dest = fixture.sibling("A");
    let locked = dest.join("app/zz-locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("held"), b"cannot be removed\n").unwrap();
    // The lane is otherwise clean and preserved; the untracked directory is
    // waived so the removal itself is what stops.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    let before_b = tree_bytes(&fixture.sibling("B"));

    let error = refuse(&fixture.root, delete_request("A", &["dirty"]));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        error.code,
        ErrorCode::DisposalIncomplete,
        "{}",
        error.message
    );
    for needle in [
        "removal stopped",
        "zz-locked/held",
        "disposing",
        "no replay",
        "--keep",
    ] {
        assert!(error.message.contains(needle), "{}", error.message);
    }
    assert!(locked.join("held").is_file(), "the remainder is retained");
    assert!(dest.is_dir());
    assert_eq!(
        family(&fixture.root).unwrap().member("A").unwrap().1.state,
        MemberState::Disposing,
        "no rollback: the interrupted state stands"
    );
    let listed = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::List, None, None),
    );
    assert_eq!(
        listed.members[1].recorded_state,
        crate::LocalMemberState::Disposing
    );
    assert_eq!(
        tree_bytes(&fixture.sibling("B")),
        before_b,
        "B is untouched"
    );

    // A repeat is not a replay: the interrupted row is not forceable.
    let remainder = tree_bytes(&dest);
    let error = refuse(
        &fixture.root,
        delete_request("A", &["open-merge", "dirty", "unpreserved-history"]),
    );
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.message);
    assert_eq!(tree_bytes(&dest), remainder, "nothing more was removed");

    // Keep detaches the remainder.
    let response = local(
        &fixture.root,
        family_request(crate::LocalFamilyOp::Dispose, Some("A"), Some(true)),
    );
    assert!(
        response
            .response
            .meta
            .message
            .unwrap()
            .contains("detached local clone `A`")
    );
    assert_eq!(listed_names(&fixture.root), ["root", "B"]);
    assert!(locked.join("held").is_file());
}

/// Design §5.2 step 5: when the contents are already gone, an explicit
/// dispose removes the stale row after validation -- no work or history
/// check, nothing removed. A moved lane looks exactly like this from the
/// root, and its tree is untouched wherever it went.
#[test]
fn a_stale_row_for_an_absent_target_is_removed_after_validation() {
    let fixture = clean_family_workspace("dispose-stale");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let dest = fixture.sibling("A");
    let elsewhere = fixture.tree.path().join("A-elsewhere");
    fs::rename(&dest, &elsewhere).unwrap();
    let before = tree_bytes(&elsewhere);

    let response = local(&fixture.root, delete_request("A", &[]));
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("stale row"), "{message}");
    assert!(message.contains("nothing stood at"), "{message}");
    assert!(message.contains("no file was removed"), "{message}");
    assert_eq!(listed_names(&fixture.root), ["root", "B"]);
    assert_eq!(
        tree_bytes(&elsewhere),
        before,
        "the moved tree is untouched"
    );
    assert!(
        elsewhere.join(".gwz/family-root").is_file(),
        "not even its (now dangling) pointer was touched"
    );
    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_eq!(error.code, ErrorCode::MemberNotFound);
}

/// Design §12 ("Clean intact lane with all protected history elsewhere:
/// explicit dispose deletes without creating an archive") and §5: a clean
/// lane whose every protected root is preserved whole in a surviving
/// family repository is deleted -- the directory is gone, the row with it,
/// every other member and the root are byte-identical, and no archive
/// appears anywhere. The survivor may be a lane, not the root: C's commit
/// lives on in D, cloned from C, so C deletes; D, which holds it alone,
/// refuses.
#[test]
fn a_clean_lane_whose_history_is_preserved_deletes_and_the_index_forgets_it() {
    let fixture = clean_family_workspace("dispose-delete");
    clone(&fixture.root, "A");
    clone(&fixture.root, "B");
    let a = fixture.sibling("A");
    let b = fixture.sibling("B");
    let before_b = tree_bytes(&b);
    let before_root = root_files_except_the_index(&fixture.root);
    let tree_entries_before = fs::read_dir(fixture.tree.path()).unwrap().count();

    let response = local(&fixture.root, delete_request("A", &[]));
    let message = response.response.meta.message.expect("a message");
    assert!(message.contains("deleted local clone `A`"), "{message}");
    assert!(message.contains(&a.display().to_string()), "{message}");
    assert!(message.contains("row removed"), "{message}");
    assert!(!message.contains("forced past"), "{message}");
    assert!(response.members.is_empty() && response.root_path.is_none());

    assert!(!a.exists(), "the directory is gone");
    assert!(
        family(&fixture.root).unwrap().member("A").is_none(),
        "the row is gone"
    );
    assert_eq!(listed_names(&fixture.root), ["root", "B"]);
    assert_eq!(
        listed_names(&b),
        ["root", "B"],
        "B still resolves the family"
    );
    assert_eq!(tree_bytes(&b), before_b, "B is byte-identical");
    assert_eq!(
        root_files_except_the_index(&fixture.root),
        before_root,
        "the root is byte-identical"
    );
    assert_eq!(
        fs::read_dir(fixture.tree.path()).unwrap().count(),
        tree_entries_before - 1,
        "no archive or backup appeared beside the family"
    );
    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_eq!(error.code, ErrorCode::MemberNotFound, "{}", error.message);

    // Preservation in a surviving lane, not the root: C's commit lives on
    // in D, cloned from C, so C deletes; then D holds it alone and refuses.
    clone(&fixture.root, "C");
    let c = fixture.sibling("C");
    let in_c = commit_in(&c.join("app"), "feature.txt", "from C\n", "only in C");
    clone(&c, "D");
    let d = fixture.sibling("D");
    assert_eq!(head_of(&d.join("app")), in_c, "D was cloned from C");
    let response = local(&fixture.root, delete_request("C", &[]));
    assert!(
        response
            .response
            .meta
            .message
            .unwrap()
            .contains("deleted local clone `C`")
    );
    assert!(!c.exists());
    assert_eq!(listed_names(&fixture.root), ["root", "B", "D"]);
    assert_eq!(head_of(&d.join("app")), in_c);
    let before_d = tree_bytes(&d);
    let error = refuse(&fixture.root, delete_request("D", &[]));
    assert_refused_without_effect(
        &fixture,
        &d,
        &before_d,
        &error,
        ErrorCode::UnwaivedHazard,
        &["<unpreserved-history>", in_c.as_str()],
    );
}

/// The disposal ports against real repositories: `observe_target` reads the
/// target's metadata through the store and every included repository
/// through the inspector; `check_history` asks `gwz-history-check` once per
/// witness store and combines; `remove_directory` removes a tree once.
#[test]
fn the_disposal_ports_observe_check_history_per_witness_and_remove() {
    let fixture = family_workspace("dispose-ports");
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let view = family(&fixture.root).unwrap();
    let name = gwz_family_model::MemberName::parse("A").unwrap();
    let mut ports = CoreDisposalPorts::new(
        fixture.root.clone(),
        view.clone(),
        name.clone(),
        open_merge_probe,
    );

    let evidence = ports.observe_target(&dest).expect("observed");
    assert_eq!(
        evidence.target,
        TargetObservation::Present {
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Matches,
        }
    );
    assert!(evidence.unknown.is_empty(), "{:?}", evidence.unknown);
    let keys: Vec<&RepoKey> = evidence
        .repositories
        .iter()
        .map(|repository| &repository.key)
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert_eq!(keys[0], &RepoKey::Root);
    assert!(
        matches!(keys[1], RepoKey::Member { id } if id.starts_with("mem_")),
        "{keys:?}"
    );
    let app = &evidence.repositories[1];
    let work = app.work.known().expect("the member's work is known");
    assert!(
        work.entries
            .iter()
            .any(|entry| entry.kind == WorkKind::Untracked && entry.path == b"notes.txt"),
        "the copied untracked note is observed: {:?}",
        work.entries
    );
    let root_repo = &evidence.repositories[0];
    let root_work = root_repo.work.known().expect("the root's work is known");
    assert!(
        root_work
            .entries
            .iter()
            .any(|entry| entry.kind == WorkKind::Unstaged && entry.path == b"README"),
        "the copied unstaged edit is observed: {:?}",
        root_work.entries
    );
    assert!(
        !root_work
            .entries
            .iter()
            .any(|entry| { entry.path.starts_with(b".gwz") || entry.path.starts_with(b"app") }),
        "GWZ's runtime directory and the separately inspected member are not \
         the root's work: {:?}",
        root_work.entries
    );
    assert!(root_repo.gwz.merge == gwz_work_detector::EvidenceState::None);
    let protected = match &root_repo.history {
        Observation::Known(protected) => protected.clone(),
        Observation::Unknown(reasons) => panic!("history unknown: {reasons:?}"),
    };
    assert!(protected.is_complete() && !protected.roots.is_empty());

    // The source root's repository is the witness paired with @root, and it
    // holds every object A's protected roots reach: preserved.
    let query = HistoryQuery {
        target: RepoKey::Root,
        protected: protected.clone(),
    };
    assert_eq!(ports.check_history(&query), HistoryAnswer::Preserved);

    // A commit made only in A is reached by no surviving witness root.
    let repository = git2::Repository::open(&dest).unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    let tree_id = {
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("README")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repository.find_tree(tree_id).unwrap();
    let parent = repository.head().unwrap().peel_to_commit().unwrap();
    let only_in_a = repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "only in A",
            &tree,
            &[&parent],
        )
        .unwrap();
    drop(tree);
    drop(parent);
    drop(repository);
    let evidence = ports.observe_target(&dest).expect("observed again");
    let protected = evidence.repositories[0]
        .history
        .known()
        .cloned()
        .expect("known");
    match ports.check_history(&HistoryQuery {
        target: RepoKey::Root,
        protected,
    }) {
        HistoryAnswer::Unpreserved { detail } => {
            assert!(detail.contains(&only_in_a.to_string()), "{detail}");
        }
        other => panic!("a commit only in A is unpreserved, got {other:?}"),
    }
    // A nested repository has no paired witness.
    assert!(matches!(
        ports.check_history(&HistoryQuery {
            target: RepoKey::Member {
                id: "nested:vendor/thing".to_owned()
            },
            protected: gwz_repo_contract::ProtectedRoots::default(),
        }),
        HistoryAnswer::Unpreserved { .. }
    ));

    // The remover, on a scratch tree beside the family (never the clone).
    let scratch = fixture.tree.dir("scratch/deep");
    fs::write(scratch.join("file"), b"x").unwrap();
    ports
        .remove_directory(&fixture.tree.join("scratch"))
        .expect("removed");
    assert!(!fixture.tree.join("scratch").exists());
    assert!(
        dest.join(".gwz/family-root").is_file(),
        "the clone is untouched"
    );
    let _ = DisposeEffect::RowDetached;
}

/// Design §5.1 ("including root and unmanaged nested repositories") and
/// §13.8's recorded gap: a nested **bare** repository -- a directory that
/// is itself a Git directory, with no `.git` entry -- is inventoried and
/// inspected like every other repository in the deletion tree, so its
/// unique history refuses ordinary deletion rather than being deleted
/// uninspected.
#[test]
fn a_nested_bare_repository_is_inventoried_and_its_history_protected() {
    let fixture = clean_family_workspace("dispose-nested-bare");
    // A bare repository inside the source tree, copied into the lane.
    let bare = fixture.tree.bare_repo("root/vendor/mirror.git");
    let unique = bare.commit_files("mirror", &[("README", b"mirrored\n")]);
    clone(&fixture.root, "A");
    let dest = fixture.sibling("A");
    let view = family(&fixture.root).unwrap();
    let mut ports = CoreDisposalPorts::new(
        fixture.root.clone(),
        view,
        MemberName::parse("A").unwrap(),
        open_merge_probe,
    );
    let evidence = ports.observe_target(&dest).expect("observed");
    assert!(evidence.unknown.is_empty(), "{:?}", evidence.unknown);
    let nested = evidence
        .repositories
        .iter()
        .find(|repository| {
            matches!(&repository.key, RepoKey::Member { id } if id == "nested:vendor/mirror.git")
        })
        .expect("the nested bare repository is inventoried");
    assert!(nested.info.bare);
    let roots = nested.history.known().expect("its history is known");
    assert!(
        roots.roots.iter().any(|root| root.oid == unique),
        "its commit is a protected root: {:?}",
        roots.roots
    );
    // Through the slot: a nested repository is paired with no survivor, so
    // its history is unpreserved until the operator names the loss.
    let before = tree_bytes(&dest);
    let error = refuse(&fixture.root, delete_request("A", &[]));
    assert_refused_without_effect(
        &fixture,
        &dest,
        &before,
        &error,
        ErrorCode::UnwaivedHazard,
        &["<unpreserved-history>", "nested:vendor/mirror.git"],
    );
    local(&fixture.root, delete_request("A", &["unpreserved-history"]));
    assert!(!dest.exists());
    assert!(
        fixture.root.join("vendor/mirror.git/HEAD").is_file(),
        "the source's own copy stands"
    );
}
