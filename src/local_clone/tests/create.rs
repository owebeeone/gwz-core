//! `local_clone::tests::create`: the first real `gwz clone --local`, on a
//! real workspace built with `gwz-local-testrepo` -- verbatim create, a
//! design §4.0 hazard refused before reservation, and an interrupted create.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Cancellation;
use gwz_family_model::{CloneMode, MemberKind, MemberState, ROOT_PATH};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilySource, FamilyStore};

use super::fixture::{FamilyFixture, family_files_absent, family_workspace, meta};
use crate::artifact::{ConfIntegrityVerdict, inspect_conf_integrity};
use crate::git::Git2Backend;
use crate::local_clone::create;
use crate::local_clone::request::validate_clone_local;
use crate::model::ErrorCode;
use crate::operation::NullSink;
use crate::workspace_ops::{handle_clone_local_workspace, open_merge_probe};

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

fn family_view(root: &Path) -> (PathBuf, gwz_family_model::FamilyView) {
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(root))
        .expect("the family reads")
    {
        FamilyObservation::Family { root, view, .. } => (root, view),
        FamilyObservation::NoFamily => panic!("{} is in no family", root.display()),
    }
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Design §4, §4.1, §8.1: `root -> A` verbatim. The copy is independent, the
/// destination's `.gwz/` holds a regenerated pointer and marker and no
/// index, no `.gwz/merge/`, the manifest is written (last, by construction
/// -- the interrupted case below shows it absent until then) and the row is
/// `ready`.
#[test]
fn root_to_a_verbatim_create_is_independent_installed_and_ready() {
    let fixture = family_workspace("create-a");
    let backend = Git2Backend::without_credential_helpers();
    let response = handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    )
    .expect("the first real local clone");
    let dest = fixture.sibling("A");
    let message = response.response.meta.message.expect("a message");
    eprintln!("{message}");
    assert!(message.contains("created local clone `A`"), "{message}");
    assert!(message.contains(&dest.display().to_string()), "{message}");
    assert!(message.contains("recorded as ../root-A"), "{message}");
    assert!(message.contains("founded"), "{message}");

    // The destination's .gwz/: a regenerated pointer and marker, no index,
    // no lock, no merge store, no copied runtime locks.
    assert!(dest.join(".gwz/family-root").is_file());
    assert!(dest.join(".gwz/local-clone-allocation").is_file());
    assert!(!dest.join(".gwz/local-family.yml").exists());
    assert!(!dest.join(".gwz/local-family.lock").exists());
    assert!(!dest.join(".gwz/merge").exists());
    assert!(!dest.join(".gwz/locks").exists());
    assert!(!dest.join("app/.git/worktrees").exists());
    // The manifest and the regenerated conf-integrity marker.
    assert_eq!(
        read(&dest.join("gwz.conf/gwz.yml")),
        read(&fixture.root.join("gwz.conf/gwz.yml")),
        "the destination manifest is the source's"
    );
    assert_eq!(
        read(&dest.join("gwz.conf/gwz.lock.yml")),
        read(&fixture.root.join("gwz.conf/gwz.lock.yml")),
        "the lock is carried verbatim"
    );
    assert_eq!(
        inspect_conf_integrity(&dest),
        ConfIntegrityVerdict::Verified
    );
    // The dirt came along: verbatim copies the tree as it sits.
    assert_eq!(read(&dest.join("app/notes.txt")), b"scratch\n");
    assert_eq!(read(&dest.join("README")), b"edited\n");
    assert!(dest.join("AGENTS_GWZ.md").is_file());

    // The row: ready, recorded root-relative, sourced from the root.
    let (family_root, view) = family_view(&fixture.root);
    assert_eq!(family_root, fixture.root);
    let (name, row) = view.member("A").expect("row A");
    assert_eq!(name.as_str(), "A");
    assert_eq!(row.state, MemberState::Ready);
    assert_eq!(row.path, "../root-A");
    assert_eq!(row.kind, MemberKind::Checkout);
    assert_eq!(row.mode, CloneMode::Verbatim);
    assert_eq!(row.source_path, ROOT_PATH);
    assert_eq!(row.last_error, None);
    // The pointer names the root; reading the family from A reaches it.
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(&dest))
        .unwrap()
    {
        FamilyObservation::Family {
            root,
            source,
            view: through_pointer,
        } => {
            assert_eq!(root, fixture.root);
            assert_eq!(source, FamilySource::Pointer);
            assert_eq!(through_pointer, view);
        }
        FamilyObservation::NoFamily => panic!("A holds no pointer"),
    }
    // The source is untouched: it holds the index (founded here) and no
    // pointer or marker of its own; its dirt is still there.
    assert!(fixture.root.join(".gwz/local-family.yml").is_file());
    assert!(!fixture.root.join(".gwz/family-root").exists());
    assert!(!fixture.root.join(".gwz/local-clone-allocation").exists());
    assert_eq!(read(&fixture.root.join("app/notes.txt")), b"scratch\n");

    // Independence (design §4.0 dest-complete): every destination
    // repository's metadata lives inside the destination, and a commit made
    // in the clone never reaches the source's object store.
    let canonical_dest = fs::canonicalize(&dest).unwrap();
    for relative in ["", "app"] {
        let repository = git2::Repository::open(dest.join(relative)).unwrap();
        let common = fs::canonicalize(repository.commondir()).unwrap();
        assert!(
            common.starts_with(&canonical_dest),
            "{relative}: {} is inside {}",
            common.display(),
            canonical_dest.display()
        );
    }
    let clone_app = git2::Repository::open(dest.join("app")).unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    let tree_id = {
        let mut index = clone_app.index().unwrap();
        index.add_path(Path::new("notes.txt")).unwrap();
        index.write().unwrap();
        index.write_tree().unwrap()
    };
    let tree = clone_app.find_tree(tree_id).unwrap();
    let parent = clone_app.head().unwrap().peel_to_commit().unwrap();
    let new_commit = clone_app
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "only in A",
            &tree,
            &[&parent],
        )
        .unwrap();
    let source_app = fixture.workspace.member("app").open();
    assert!(
        !source_app.odb().unwrap().exists(new_commit),
        "a commit in the clone does not appear in the source"
    );
    assert_eq!(
        fixture.workspace.member("app").head_id().to_hex(),
        parent.id().to_string(),
        "the source HEAD is where the copy froze it"
    );
}

/// Design §4.0, §12 ("Included ignored nested gitfile/alternates/external
/// metadata: refuse before reservation"): a member borrowing another object
/// store refuses the create with nothing written -- no lock file, no index,
/// no row, no destination.
#[test]
fn a_source_hazard_refuses_before_reservation_and_leaves_nothing() {
    let fixture = family_workspace("create-hazard");
    let borrowed = fixture.tree.dir("borrowed-objects");
    fixture.workspace.member("app").hazard_alternates(&borrowed);
    let backend = Git2Backend::without_credential_helpers();
    let error = handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedOperation,
        "{}",
        error.message
    );
    assert!(error.message.contains("Alternates"), "{}", error.message);
    assert!(
        error.message.contains("nothing was reserved"),
        "{}",
        error.message
    );
    assert!(
        family_files_absent(&fixture.root),
        "no lock file, no index, no pointer, no marker at the root"
    );
    assert!(
        !fixture.sibling("A").exists(),
        "no destination was allocated"
    );
}

/// Cancels once `path` exists: after the pointer is installed and before the
/// manifest is published, the next poll is installation's own checkpoint
/// at `publish_manifest`.
struct CancelWhenExists(PathBuf);

impl Cancellation for CancelWhenExists {
    fn is_cancelled(&self) -> bool {
        self.0.exists()
    }
}

/// Design §3.1, §4 step 4, §12 ("Interrupted create, including
/// manifest-before-ready gap: retain and report"): an install interrupted
/// after the row and before the manifest leaves a `creating` row carrying
/// the diagnostic and an inspectable directory with no manifest -- the
/// manifest really is last -- and nothing is cleaned up or promoted.
#[test]
fn an_interrupted_create_leaves_a_creating_row_and_an_inspectable_directory() {
    let fixture = family_workspace("create-interrupted");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    let error = create::clone_local(
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &CancelWhenExists(dest.join(".gwz/family-root")),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::IoError, "{}", error.message);
    assert!(
        error
            .message
            .contains("publish manifest failed: install cancelled"),
        "{}",
        error.message
    );
    for effect in [
        "RowAllocated",
        "DestinationAllocated",
        "TreeCopied",
        "DestinationGitInstalled",
        "PointerInstalled",
        "ConfigurationInstalled",
        "ErrorRecorded",
    ] {
        assert!(
            error.message.contains(effect),
            "{effect}: {}",
            error.message
        );
    }
    assert!(
        !error.message.contains("ManifestPublished"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("retained for inspection"),
        "{}",
        error.message
    );

    let (_, view) = family_view(&fixture.root);
    let (_, row) = view.member("A").expect("the creating row is retained");
    assert_eq!(row.state, MemberState::Creating);
    assert!(
        row.last_error
            .as_deref()
            .is_some_and(|detail| detail.contains("publish manifest")),
        "{:?}",
        row.last_error
    );
    // The directory: built, pointed, but without the manifest -- not
    // discoverable as a workspace, and not an exchange endpoint.
    assert!(dest.join(".gwz/family-root").is_file());
    assert!(dest.join(".gwz/local-clone-allocation").is_file());
    assert!(dest.join("app/.git").is_dir());
    assert_eq!(read(&dest.join("app/notes.txt")), b"scratch\n");
    assert!(
        !dest.join("gwz.conf/gwz.yml").exists(),
        "the manifest is written last, so an interruption before it leaves none"
    );
    assert!(
        dest.join("gwz.conf/gwz.lock.yml").is_file(),
        "the copied lock is there"
    );
}

/// The fixture itself: a workspace the public handlers accept, so a failure
/// above is the clone's and not the scaffolding's.
#[test]
fn the_family_fixture_is_a_registered_workspace_with_one_member() {
    let fixture = family_workspace("create-fixture");
    let manifest = crate::artifact::read_manifest(&fixture.root).unwrap();
    assert_eq!(manifest.members.len(), 1);
    assert_eq!(manifest.members[0].path, "app");
    assert!(manifest.members[0].active);
    assert_eq!(
        inspect_conf_integrity(&fixture.root),
        ConfIntegrityVerdict::Verified
    );
    assert!(family_files_absent(&fixture.root), "no family yet");
    let _ = FamilyFixture::sibling;
}
