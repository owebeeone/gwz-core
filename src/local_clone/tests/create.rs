//! `local_clone::tests::create`: the first real `gwz clone --local`, on a
//! real workspace built with `gwz-local-testrepo` -- verbatim create, a
//! design §4.0 hazard refused before reservation, and an interrupted create.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::Cancellation;
use gwz_family_model::{CloneMode, MemberKind, MemberState, ROOT_PATH};
use gwz_family_store_contract::{FamilyLocation, FamilyObservation, FamilySource, FamilyStore};

use super::fixture::{
    FamilyFixture, TempDir, family_files_absent, family_workspace, meta,
    uncommitted_configuration_workspace, workspace,
};
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
    // LCM1.1 fix 1: a §4.0 hazard is its own code, not "not built yet".
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedSourceLayout,
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

#[test]
fn an_uncommitted_workspace_bootstrap_refuses_before_family_allocation() {
    let fixture = uncommitted_configuration_workspace("create-uncommitted-bootstrap");
    let destination = fixture.sibling("A");
    let error = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error}");
    assert!(
        error.message.contains("not ready for a local family"),
        "{error}"
    );
    assert!(error.message.contains("gwz.conf/gwz.yml"), "{error}");
    assert!(error.message.contains("gwz.conf/gwz.lock.yml"), "{error}");
    assert!(
        family_files_absent(&fixture.root),
        "the incomplete source must not found a family"
    );
    assert!(
        !destination.exists(),
        "the incomplete source must not allocate a destination"
    );
}

#[test]
fn an_unborn_workspace_root_refuses_before_family_allocation() {
    let temp = TempDir::new("create-unborn-root");
    let root = workspace(&temp);
    let destination = temp.path().join("must-not-exist");
    let mut request = clone_request("A");
    request.dest = Some(destination.to_string_lossy().into_owned());
    let error = handle_clone_local_workspace(
        &Git2Backend::without_credential_helpers(),
        &root,
        request,
        "op-clone",
        &NullSink,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error}");
    assert!(
        error.message.contains("root has no committed HEAD"),
        "{error}"
    );
    assert!(
        family_files_absent(&root),
        "the unborn source must not found a family"
    );
    assert!(
        !destination.exists(),
        "the unborn source must not allocate a destination"
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
/// manifest really is last -- and nothing is cleaned up or promoted. The
/// code is `destination_incomplete` (LCM1.1 fix 1): an interruption leaves
/// exactly the shape a failed completion rule leaves, the one `gwz local
/// list` shows as `creating/incomplete`, and the message says it was
/// cancelled.
#[test]
fn an_interrupted_create_leaves_a_creating_row_and_an_inspectable_directory() {
    let fixture = family_workspace("create-interrupted");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    let error = create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &CancelWhenExists(dest.join(".gwz/family-root")),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::DestinationIncomplete,
        "{}",
        error.message
    );
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

/// Plan LCM1.1 exit ("root -> A -> B has one namespace and independent
/// data"), design §3 and §8.1: a clone made *from* A registers on the root
/// with A as its source, points at the root, and lists beside A; a taken
/// name and an occupied destination refuse typed, before any effect.
#[test]
fn a_clone_of_a_clone_registers_on_the_root_and_collisions_refuse() {
    let fixture = family_workspace("create-chain");
    let backend = Git2Backend::without_credential_helpers();
    handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone-a",
        &NullSink,
    )
    .expect("clone A");
    let a = fixture.sibling("A");
    // From A: `gwz clone --local --name B` with the default sibling dest.
    handle_clone_local_workspace(&backend, &a, clone_request("B"), "op-clone-b", &NullSink)
        .expect("clone B from A");
    let b = fixture.sibling("B");
    let (family_root, view) = family_view(&fixture.root);
    assert_eq!(family_root, fixture.root);
    let (_, row_b) = view.member("B").expect("row B on the root");
    assert_eq!(row_b.state, MemberState::Ready);
    assert_eq!(row_b.path, "../root-B");
    assert_eq!(row_b.source_path, "../root-A", "B was copied from A");
    assert_eq!(
        read(&b.join("app/notes.txt")),
        b"scratch\n",
        "A's tree is B's"
    );
    assert!(
        !b.join(".gwz/local-family.yml").exists(),
        "B holds no index: A's pointer was not copied and B got its own"
    );
    match gwz_family_store::YamlFamilyStore::new()
        .read_view(&FamilyLocation::new(&b))
        .unwrap()
    {
        FamilyObservation::Family { root, .. } => assert_eq!(root, fixture.root),
        FamilyObservation::NoFamily => panic!("B holds no pointer"),
    }
    let listed = crate::workspace_ops::handle_local_family(
        &backend,
        &b,
        crate::LocalFamilyRequest {
            meta: meta("req-list"),
            op: crate::LocalFamilyOp::List,
            name: None,
            keep: None,
            force_hazards: Vec::new(),
        },
        "op-list",
        &NullSink,
    )
    .unwrap();
    assert_eq!(
        listed
            .members
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "A", "B"]
    );

    // A taken name refuses as a collision naming the holder; an occupied
    // destination refuses as a collision too; neither writes a row.
    let taken = handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone-again",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(taken.code, ErrorCode::PathCollision, "{}", taken.message);
    assert!(taken.message.contains("../root-A"), "{}", taken.message);
    let mut occupied = clone_request("C");
    occupied.dest = Some(a.to_string_lossy().into_owned());
    let occupied = handle_clone_local_workspace(
        &backend,
        &fixture.root,
        occupied,
        "op-clone-occupied",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(
        occupied.code,
        ErrorCode::PathCollision,
        "{}",
        occupied.message
    );
    assert!(
        occupied.message.contains("nothing was reserved"),
        "{}",
        occupied.message
    );
    let (_, view) = family_view(&fixture.root);
    assert_eq!(view.members.len(), 2, "A and B only");
    assert!(!fixture.sibling("C").exists());
}

/// A cancellation port with a side effect: on its first poll it moves the
/// source (a new branch in the root repository) and never cancels. The
/// first poll is installation's `Reserve` checkpoint, after the snapshot,
/// so the copy carries a source the frozen snapshot no longer describes.
struct DriftSourceOnce {
    repository: PathBuf,
    done: std::sync::atomic::AtomicBool,
}

impl Cancellation for DriftSourceOnce {
    fn is_cancelled(&self) -> bool {
        if !self.done.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let repository = git2::Repository::open(&self.repository).unwrap();
            let head = repository.head().unwrap().peel_to_commit().unwrap();
            repository.branch("drifted", &head, false).unwrap();
        }
        false
    }
}

/// Design §4 step 3 ("recheck the source observations"), §12 ("Observed
/// source drift before publication: fail without marking ready"; LCM1.1
/// fix 1): a source that moved between the snapshot and publication is
/// `source_drift` -- not an I/O error -- with the row and the copied
/// destination retained and the drift named.
#[test]
fn source_drift_before_publication_is_source_drift_with_the_row_retained() {
    let fixture = family_workspace("create-drift");
    let dest = fixture.sibling("A");
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    let error = create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &DriftSourceOnce {
            repository: fixture.root.clone(),
            done: std::sync::atomic::AtomicBool::new(false),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SourceDrift, "{}", error.message);
    assert!(error.message.contains("source drift"), "{}", error.message);
    assert!(
        error.message.contains("repository @root changed"),
        "{}",
        error.message
    );
    for effect in [
        "RowAllocated",
        "TreeCopied",
        "PointerInstalled",
        "ErrorRecorded",
    ] {
        assert!(
            error.message.contains(effect),
            "{effect}: {}",
            error.message
        );
    }
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
            .is_some_and(|detail| detail.contains("source drift")),
        "{:?}",
        row.last_error
    );
    assert!(dest.join(".gwz/family-root").is_file());
    assert!(!dest.join("gwz.conf/gwz.yml").exists(), "never published");
}

/// A cancellation port with a side effect: once the copier has reproduced
/// `object` at the destination it removes it again and never cancels, so
/// the destination's own store is missing one object the copy delivered.
struct RemoveObjectOnceCopied {
    object: PathBuf,
    done: std::sync::atomic::AtomicBool,
}

impl Cancellation for RemoveObjectOnceCopied {
    fn is_cancelled(&self) -> bool {
        if self.object.exists() && !self.done.swap(true, std::sync::atomic::Ordering::SeqCst) {
            fs::remove_file(&self.object).unwrap();
        }
        false
    }
}

/// Design §4.0 dest-complete ("fails on missing objects"; LCM1.1 fix 1 and
/// fix 2): an object missing from the destination's own store is found by
/// the connectivity walk and refused as `destination_incomplete`, naming the
/// repository and the missing object, with the row and directory retained
/// and no manifest published.
#[test]
fn an_object_missing_from_the_destination_store_is_destination_incomplete() {
    let fixture = family_workspace("create-missing-object");
    let dest = fixture.sibling("A");
    let tree = fixture
        .workspace
        .member("app")
        .open()
        .head()
        .unwrap()
        .peel_to_tree()
        .unwrap()
        .id()
        .to_string();
    let object = dest
        .join("app/.git/objects")
        .join(&tree[..2])
        .join(&tree[2..]);
    let validated = validate_clone_local(&clone_request("A")).unwrap();
    let error = create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &RemoveObjectOnceCopied {
            object,
            done: std::sync::atomic::AtomicBool::new(false),
        },
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::DestinationIncomplete,
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("objects missing from the destination store"),
        "{}",
        error.message
    );
    assert!(error.message.contains(&tree), "{}", error.message);
    assert!(error.message.contains("app"), "{}", error.message);
    assert!(
        !error.message.contains("ManifestPublished"),
        "{}",
        error.message
    );
    let (_, view) = family_view(&fixture.root);
    let (_, row) = view.member("A").expect("the creating row is retained");
    assert_eq!(row.state, MemberState::Creating);
    assert!(!dest.join("gwz.conf/gwz.yml").exists(), "never published");
}

/// Design §4 ("Permission, space, I/O and metadata failures are errors, not
/// 'unsupported'"), §12 ("Copy permission/space/I/O error: fail, retain
/// partial destination, source unchanged"; LCM1.1 fix 1): a file the copier
/// cannot read stops the copy as `copy_failed`, with the row and the partial
/// destination retained and the source untouched.
#[cfg(unix)]
#[test]
fn an_unreadable_source_file_is_copy_failed_with_the_partial_destination_retained() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = family_workspace("create-copy-failed");
    let locked = fixture.root.join("app/locked.txt");
    fs::write(&locked, b"cannot be read\n").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let dest = fixture.sibling("A");
    let backend = Git2Backend::without_credential_helpers();
    let error = handle_clone_local_workspace(
        &backend,
        &fixture.root,
        clone_request("A"),
        "op-clone",
        &NullSink,
    )
    .unwrap_err();
    // Readable again before any assertion can fail, so the fixture cleans up.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(error.code, ErrorCode::CopyFailed, "{}", error.message);
    assert!(
        error.message.contains("copy failed at"),
        "{}",
        error.message
    );
    assert!(error.message.contains("locked.txt"), "{}", error.message);
    for effect in ["RowAllocated", "DestinationAllocated", "ErrorRecorded"] {
        assert!(
            error.message.contains(effect),
            "{effect}: {}",
            error.message
        );
    }
    assert!(!error.message.contains("TreeCopied"), "{}", error.message);
    assert!(
        error.message.contains("retained for inspection"),
        "{}",
        error.message
    );
    let (_, view) = family_view(&fixture.root);
    let (_, row) = view.member("A").expect("the creating row is retained");
    assert_eq!(row.state, MemberState::Creating);
    assert!(dest.is_dir(), "the partial destination is retained");
    assert!(
        !dest.join(".gwz/family-root").exists(),
        "the pointer is installed after the copy, so a failed copy has none"
    );
    assert_eq!(
        read(&fixture.root.join("app/notes.txt")),
        b"scratch\n",
        "the source is unchanged"
    );
}

/// LCM1.1 fix 2: dest-complete is a connectivity walk from every protected
/// root of the destination, not a preservation proof with the destination
/// as its own witness. Before the fix a source whose reflog or stash named
/// a commit no ref reached -- the state of every real repository measured
/// (gwz-core: 30 of 374 roots, gwz-cli: 12 of 220, the gwz-dev root: 3 of
/// 473) -- was refused after the copy as "objects missing from the
/// destination store". Now it creates, every object once, and the report
/// says what the walk read, what bounded it and what it cost.
#[test]
fn a_source_with_reflog_only_history_creates_and_reports_the_verified_objects() {
    let fixture = family_workspace("create-reflog-only");
    // Amend `app`'s only commit: the original survives in HEAD's reflog and
    // nowhere else.
    let mut app = fixture.workspace.member("app").open();
    let original = app.head().unwrap().peel_to_commit().unwrap();
    let tree = original.tree().unwrap();
    let amended = original
        .amend(Some("HEAD"), None, None, None, Some("amended"), Some(&tree))
        .unwrap();
    assert_ne!(amended, original.id());
    drop(tree);
    drop(original);
    // Stash a tracked edit in `app`: `stash@{0}` names two commits no ref
    // does (the stash commit and its index commit), plus a tree and a blob.
    fs::write(fixture.root.join("app/README"), b"stash me\n").unwrap();
    let signature = gwz_local_testrepo::fixture_signature();
    app.stash_save(&signature, "wip", None).unwrap();

    let validated = validate_clone_local(&clone_request("A")).unwrap();
    let report = create::clone_local(
        &Git2Backend::without_credential_helpers(),
        &fixture.root,
        &fixture.root,
        &validated,
        open_merge_probe,
        &gwz_copy_contract::NeverCancelled,
    )
    .expect("a reflog-only commit and a stash are connected, not missing");
    let (_, view) = family_view(&fixture.root);
    assert_eq!(view.member("A").unwrap().1.state, MemberState::Ready);

    // The report: both repositories walked, the amended-away commit and the
    // stash commits read, the census the walk was bounded by at least what
    // it read.
    assert_eq!(report.verification.len(), 2, "{:?}", report.verification);
    let keys: Vec<String> = report
        .verification
        .iter()
        .map(|entry| entry.key.to_string())
        .collect();
    assert_eq!(
        keys,
        ["@root", "mem_app"],
        "the manifest member id, not its path"
    );
    for entry in &report.verification {
        assert!(entry.roots >= 2, "{entry:?}");
        assert!(entry.objects_visited >= 3, "{entry:?}");
        assert!(
            entry.census.total() >= entry.objects_visited,
            "the census bounds the walk: {entry:?}"
        );
        assert_eq!(entry.census.packs, 0, "the fixture is loose: {entry:?}");
    }
    let app_entry = &report.verification[1];
    // `app`: the amended commit, the original, their shared tree and blob;
    // the stash commit, its index commit, the stashed tree and blob.
    assert_eq!(app_entry.objects_visited, 8, "{app_entry:?}");
    let message = report.message(&validated.name);
    assert!(
        message.contains("dest-complete: 2 repositories,"),
        "{message}"
    );
    assert!(message.contains("objects verified of"), "{message}");
}
