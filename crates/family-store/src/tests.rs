//! In-crate cases the reference fake cannot express.
//!
//! The conformance suite (`fixture.rs`) measures the contract; these
//! measure the things only a real filesystem has: a destination that does
//! not exist, a symlinked spelling of a recorded path, two OS lock handles,
//! a genuinely failed publication, metadata that is not a regular file, and
//! the litter an interrupted publication leaves behind.

// Ordinary file I/O in this crate's own tests, outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};

use gwz_family_model::{
    ALLOCATION_MARKER_RELATIVE_PATH, AllocationId, CloneMode, FamilyChange, FamilyId,
    INDEX_RELATIVE_PATH, LOCK_RELATIVE_PATH, MAX_ENCODED_INDEX_BYTES, MemberKind, MemberName,
    MemberRow, MemberState, POINTER_RELATIVE_PATH,
};
use gwz_family_store_contract::{
    FamilyLocation, FamilyObservation, FamilySession, FamilySource, FamilyStore, MetadataEffect,
    StoreError, StoreOperation,
};

use crate::{LockedFamilySession, YamlFamilyStore};

struct Family {
    _temp: tempfile::TempDir,
    store: YamlFamilyStore,
    root: PathBuf,
}

impl Family {
    /// A founded family whose root is `<temp>/root`, so rows recorded at
    /// `../<name>` are siblings inside the temporary directory.
    fn founded() -> Self {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        fs::create_dir_all(&root).expect("create the root workspace");
        let store = YamlFamilyStore::new();
        let mut session = store.try_lock(&FamilyLocation::new(&root)).expect("lock");
        session
            .found(
                FamilyId::new("fam_store").unwrap(),
                AllocationId::new("alloc_root").unwrap(),
            )
            .expect("found the family");
        drop(session);
        Self {
            _temp: temp,
            store,
            root,
        }
    }

    fn lock(&self) -> LockedFamilySession {
        self.store
            .try_lock(&FamilyLocation::new(&self.root))
            .expect("the family lock is free")
    }

    /// Allocate a `creating` row at `relative` and materialise its
    /// destination, as the orchestrator does before the store writes.
    fn creating(&self, session: &mut LockedFamilySession, name: &str, relative: &str) -> PathBuf {
        session
            .apply(&FamilyChange::Allocate {
                name: MemberName::parse(name).unwrap(),
                row: row(relative),
            })
            .expect("allocate the row");
        let destination = self.root.join(relative);
        fs::create_dir_all(&destination).expect("allocate the destination");
        destination
    }
}

fn row(relative: &str) -> MemberRow {
    MemberRow {
        path: relative.to_owned(),
        kind: MemberKind::Checkout,
        state: MemberState::Creating,
        allocation_id: AllocationId::new(format!("alloc{relative}")).unwrap(),
        source_path: ".".to_owned(),
        mode: CloneMode::Verbatim,
        last_error: None,
    }
}

fn name(name: &str) -> MemberName {
    MemberName::parse(name).unwrap()
}

#[test]
fn a_destination_that_does_not_exist_fails_as_the_marker_write_it_would_have_been() {
    // Contract `install_pointer`: the orchestrator allocates the
    // destination first, so the store creates nothing above `.gwz/` and a
    // missing destination is `Io { operation: WriteMarker }`.
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    fs::remove_dir(&destination).expect("un-allocate the destination");

    match session.install_pointer(&name("A"), &destination) {
        Err(StoreError::Io {
            operation, path, ..
        }) => {
            assert_eq!(operation, StoreOperation::WriteMarker);
            assert_eq!(path, destination.join(ALLOCATION_MARKER_RELATIVE_PATH));
        }
        other => panic!("a missing destination fails as WriteMarker, got {other:?}"),
    }
    assert!(
        !destination.exists(),
        "the store creates nothing above `.gwz/`"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_parent_spelling_of_the_recorded_path_is_the_rows_destination() {
    // One resolution: `install_pointer`'s check, `remove_pointer` and the
    // guard all canonicalise, so a spelling that reaches the row's path
    // through a symlinked parent is the row's own destination.
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    let link = family.root.join("../by-link");
    std::os::unix::fs::symlink(family.root.join(".."), &link).expect("symlink the parent");
    let spelled = link.join("ws-A");
    assert_ne!(
        spelled, destination,
        "a different spelling of one directory"
    );

    let applied = session
        .install_pointer(&name("A"), &spelled)
        .expect("a symlinked spelling of the row's path is accepted");
    assert_eq!(
        applied.effects,
        vec![
            MetadataEffect::MarkerWritten {
                workspace: spelled.clone()
            },
            MetadataEffect::PointerWritten { workspace: spelled },
        ],
        "the effects name the destination as passed"
    );
    // The guard and the removal find that same pointer through the row.
    match session.apply(&FamilyChange::RemoveRow {
        name: name("A"),
        reason: gwz_family_model::RemovalReason::Keep,
    }) {
        Err(StoreError::PointerStillInstalled { member, workspace }) => {
            assert_eq!(member, "A");
            assert_eq!(workspace, destination, "the guard names the recorded path");
        }
        other => panic!("the pointer protects the row, got {other:?}"),
    }
    let removed = session.remove_pointer(&name("A")).expect("removal");
    assert_eq!(
        removed.effects,
        vec![
            MetadataEffect::PointerRemoved {
                workspace: destination.clone()
            },
            MetadataEffect::MarkerRemoved {
                workspace: destination
            },
        ],
        "removal names the recorded path, and finds the other spelling's pointer"
    );
}

#[test]
fn two_handles_on_one_root_prove_the_lock_is_busy_without_waiting() {
    // No sleeping and no second process: `flock`/`LockFileEx` live on the
    // open file description, so a second handle in this same process is the
    // contended case.
    let family = Family::founded();
    let held = family.lock();
    match family.store.try_lock(&FamilyLocation::new(&family.root)) {
        Err(StoreError::Busy { lock_path }) => {
            assert_eq!(lock_path, family.root.join(LOCK_RELATIVE_PATH));
        }
        other => panic!("a second lock must be Busy, got {:?}", other.err()),
    }
    drop(held);
    assert!(
        family
            .store
            .try_lock(&FamilyLocation::new(&family.root))
            .is_ok(),
        "the lock is released with the session"
    );
}

#[test]
fn a_failed_pointer_publication_reports_partial_with_the_marker_completed() {
    // A directory at the pointer's own path makes the publishing rename
    // fail with a real OS error, after the marker has already landed.
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    let obstruction = destination.join(POINTER_RELATIVE_PATH);
    fs::create_dir_all(obstruction.parent().unwrap()).unwrap();
    fs::create_dir(&obstruction).unwrap();

    match session.install_pointer(&name("A"), &destination) {
        Err(StoreError::Partial {
            operation,
            completed,
            path,
            detail,
        }) => {
            assert_eq!(operation, StoreOperation::WritePointer);
            assert_eq!(
                completed,
                vec![MetadataEffect::MarkerWritten {
                    workspace: destination.clone()
                }]
            );
            assert_eq!(path, obstruction);
            assert!(!detail.is_empty(), "the OS error is reported");
        }
        other => panic!("a failed pointer write reports Partial, got {other:?}"),
    }
    assert!(
        destination.join(ALLOCATION_MARKER_RELATIVE_PATH).is_file(),
        "the completed effect really happened"
    );
    let leftovers: Vec<String> = fs::read_dir(destination.join(".gwz"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.contains(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the temporary file is cleaned up: {leftovers:?}"
    );
}

#[test]
fn an_interrupted_publication_leaves_the_previous_index_readable() {
    // The crash shape: a temporary file left behind by a publication that
    // never renamed. The next reread still sees the previous good index,
    // and the litter is left for inspection rather than swept up.
    let family = Family::founded();
    let mut session = family.lock();
    family.creating(&mut session, "A", "../ws-A");
    let before = session.reread().unwrap().expect("the index is there");

    let litter = family
        .root
        .join(".gwz")
        .join(".local-family.yml.tmp.4242.0");
    fs::write(&litter, b"half written").expect("leave a temporary file behind");

    assert_eq!(
        session.reread().unwrap().as_ref(),
        Some(&before),
        "the previous good index is what a reread sees"
    );
    assert!(
        litter.is_file(),
        "malformed leftovers are retained, not swept"
    );
    // And a fresh read through the store agrees.
    let observed = family
        .store
        .read_view(&FamilyLocation::new(&family.root))
        .unwrap();
    assert_eq!(observed.view(), Some(&before));
}

#[test]
fn an_index_in_a_format_this_store_does_not_read_refuses_as_malformed() {
    let family = Family::founded();
    let path = family.root.join(INDEX_RELATIVE_PATH);
    fs::write(
        &path,
        b"schema: gwz.local-family/v2\nfamily_id: fam_store\nroot:\n  allocation_id: alloc_root\n",
    )
    .unwrap();
    match family.store.read_view(&FamilyLocation::new(&family.root)) {
        Err(StoreError::Malformed {
            path: reported,
            detail,
        }) => {
            assert_eq!(reported, path);
            assert!(detail.contains("gwz.local-family/v2"), "{detail}");
        }
        other => panic!("another format version refuses, got {other:?}"),
    }
}

#[test]
fn an_oversize_index_refuses_before_it_is_decoded() {
    let family = Family::founded();
    let path = family.root.join(INDEX_RELATIVE_PATH);
    let mut encoded = fs::read(&path).unwrap();
    encoded.extend(std::iter::repeat_n(
        b'#',
        usize::try_from(MAX_ENCODED_INDEX_BYTES).unwrap() + 1,
    ));
    fs::write(&path, &encoded).unwrap();
    match family.store.read_view(&FamilyLocation::new(&family.root)) {
        Err(StoreError::Oversize { bytes, limit, .. }) => {
            assert_eq!(limit, MAX_ENCODED_INDEX_BYTES);
            assert!(bytes > limit, "{bytes} > {limit}");
        }
        other => panic!("an oversize index refuses, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_where_the_index_belongs_is_refused_rather_than_followed() {
    let family = Family::founded();
    let elsewhere = family.root.join("../elsewhere.yml");
    fs::copy(family.root.join(INDEX_RELATIVE_PATH), &elsewhere).unwrap();
    let path = family.root.join(INDEX_RELATIVE_PATH);
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    match family.store.read_view(&FamilyLocation::new(&family.root)) {
        Err(StoreError::Malformed { path: reported, .. }) => assert_eq!(reported, path),
        other => panic!("a symlinked index is refused, not followed, got {other:?}"),
    }
}

#[test]
fn a_workspace_holding_both_an_index_and_a_pointer_conflicts() {
    let family = Family::founded();
    fs::write(
        family.root.join(POINTER_RELATIVE_PATH),
        b"schema: gwz.family-root/v1\nfamily_id: fam_store\nroot_path: /elsewhere\n",
    )
    .unwrap();
    match family.store.read_view(&FamilyLocation::new(&family.root)) {
        Err(StoreError::ConflictingMetadata { workspace }) => assert_eq!(workspace, family.root),
        other => panic!("both an index and a pointer conflict, got {other:?}"),
    }
    assert!(
        family
            .store
            .try_lock(&FamilyLocation::new(&family.root))
            .is_err(),
        "a conflicted workspace is not lockable either"
    );
}

#[test]
fn a_clone_reads_and_locks_through_its_pointer() {
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    session.install_pointer(&name("A"), &destination).unwrap();
    drop(session);

    let observed = family
        .store
        .read_view(&FamilyLocation::new(&destination))
        .unwrap();
    let FamilyObservation::Family { root, source, view } = observed else {
        panic!("the pointer resolves to the family");
    };
    assert_eq!(source, FamilySource::Pointer);
    assert_eq!(
        root,
        fs::canonicalize(&family.root).unwrap(),
        "the pointer records the registering root"
    );
    assert_eq!(view.family_id.as_str(), "fam_store");

    let session = family
        .store
        .try_lock(&FamilyLocation::new(&destination))
        .expect("a clone locks its root");
    assert_eq!(
        session.root(),
        fs::canonicalize(&family.root).unwrap(),
        "a clone locks its root, not itself"
    );
    assert!(
        !destination.join(LOCK_RELATIVE_PATH).exists(),
        "no second lock file is created at the clone"
    );
}

#[test]
fn a_pointer_whose_root_holds_no_matching_index_is_pointer_target_invalid() {
    let family = Family::founded();
    let clone = family.root.join("../ws-A");
    fs::create_dir_all(clone.join(".gwz")).unwrap();
    let elsewhere = family.root.join("../no-family");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(
        clone.join(POINTER_RELATIVE_PATH),
        format!(
            "schema: gwz.family-root/v1\nfamily_id: fam_store\nroot_path: {}\n",
            elsewhere.display()
        ),
    )
    .unwrap();
    match family.store.read_view(&FamilyLocation::new(&clone)) {
        Err(StoreError::PointerTargetInvalid { pointer, .. }) => {
            assert_eq!(pointer, clone.join(POINTER_RELATIVE_PATH));
        }
        other => panic!("a pointer at no family refuses, got {other:?}"),
    }

    // A root that holds a *different* family's index refuses the same way.
    fs::write(
        clone.join(POINTER_RELATIVE_PATH),
        format!(
            "schema: gwz.family-root/v1\nfamily_id: fam_other\nroot_path: {}\n",
            family.root.display()
        ),
    )
    .unwrap();
    assert!(matches!(
        family.store.read_view(&FamilyLocation::new(&clone)),
        Err(StoreError::PointerTargetInvalid { .. })
    ));
}

#[test]
fn reading_and_locking_an_absent_workspace_create_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("never-allocated");
    let store = YamlFamilyStore::new();
    let location = FamilyLocation::new(&absent);
    assert_eq!(
        store.read_view(&location).unwrap(),
        FamilyObservation::NoFamily,
        "an absent workspace is in no family"
    );
    match store.try_lock(&location) {
        Err(StoreError::Io { operation, .. }) => assert_eq!(operation, StoreOperation::Lock),
        other => panic!("locking an absent workspace is an Io refusal, got {other:?}"),
    }
    assert!(!absent.exists(), "neither call created the workspace");
}

#[test]
fn the_refusal_order_holds_before_any_destination_is_touched() {
    let family = Family::founded();
    let mut session = family.lock();
    // Unknown row, before anything is read at a destination.
    let destination = family.root.join("../ws-A");
    fs::create_dir_all(&destination).unwrap();
    assert!(matches!(
        session.install_pointer(&name("ghost"), &destination),
        Err(StoreError::Refused(
            gwz_family_model::Refusal::NotFound { .. }
        ))
    ));
    // A row that is not `creating`.
    family.creating(&mut session, "A", "../ws-A");
    session
        .apply(&FamilyChange::MarkReady {
            name: name("A"),
            expected_allocation: AllocationId::new("alloc../ws-A").unwrap(),
        })
        .expect("mark ready");
    assert!(matches!(
        session.install_pointer(&name("A"), &destination),
        Err(StoreError::Refused(
            gwz_family_model::Refusal::WrongState { .. }
        ))
    ));
    assert!(
        !destination.join(".gwz").exists(),
        "a refusal writes nothing at the destination"
    );
}

#[test]
fn removing_a_pointer_whose_directory_is_gone_reports_nothing_and_frees_the_row() {
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    session.install_pointer(&name("A"), &destination).unwrap();
    fs::remove_dir_all(&destination).expect("the destination disappears");

    let removed = session.remove_pointer(&name("A")).expect("removal");
    assert!(
        removed.effects.is_empty(),
        "a recorded path that no longer resolves holds nothing: {removed:?}"
    );
    session
        .apply(&FamilyChange::RemoveRow {
            name: name("A"),
            reason: gwz_family_model::RemovalReason::Keep,
        })
        .expect("no pointer stands, so the row may be removed");
}

#[test]
fn another_familys_marker_at_the_rows_path_is_retained_not_removed() {
    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    session.install_pointer(&name("A"), &destination).unwrap();
    // Overwrite the marker with another family's: it is not this row's, so
    // removal leaves it for inspection.
    let marker = destination.join(ALLOCATION_MARKER_RELATIVE_PATH);
    fs::write(
        &marker,
        b"schema: gwz.local-clone-allocation/v1\nfamily_id: fam_other\nallocation_id: alloc_x\n",
    )
    .unwrap();
    let removed = session.remove_pointer(&name("A")).unwrap();
    assert_eq!(
        removed.effects,
        vec![MetadataEffect::PointerRemoved {
            workspace: destination.clone()
        }],
        "the pointer is this family's; the marker is not"
    );
    assert!(marker.is_file(), "another family's marker is retained");
}

#[test]
fn partial_is_reported_only_once_an_effect_has_landed() {
    let error = std::io::Error::other("disk full");
    let path = Path::new("/roots/one/.gwz/local-clone-allocation");
    assert!(matches!(
        crate::partial_or_io(StoreOperation::RemoveMarker, &[], path, &error),
        StoreError::Io { .. }
    ));
    let completed = [MetadataEffect::PointerRemoved {
        workspace: PathBuf::from("/roots/ws-A"),
    }];
    match crate::partial_or_io(StoreOperation::RemoveMarker, &completed, path, &error) {
        StoreError::Partial {
            operation,
            completed: reported,
            ..
        } => {
            assert_eq!(operation, StoreOperation::RemoveMarker);
            assert_eq!(reported, completed);
        }
        other => panic!("a landed effect makes it Partial, got {other:?}"),
    }
}

#[test]
fn a_family_whose_index_is_malformed_is_still_lockable_and_the_reread_refuses() {
    // A store that refused the lock here would leave nothing able to
    // address the family at all: the refusal belongs to `reread` under the
    // lock, exactly where the reference fake puts it.
    let family = Family::founded();
    let path = family.root.join(INDEX_RELATIVE_PATH);
    fs::write(&path, b"schema: [unterminated\n").unwrap();
    let mut session = family
        .store
        .try_lock(&FamilyLocation::new(&family.root))
        .expect("a malformed index does not deny the lock");
    assert_eq!(session.root(), family.root);
    match session.reread() {
        Err(StoreError::Malformed { path: reported, .. }) => assert_eq!(reported, path),
        other => panic!("the reread carries the refusal, got {other:?}"),
    }
    assert!(
        matches!(
            family.store.read_view(&FamilyLocation::new(&family.root)),
            Err(StoreError::Malformed { .. })
        ),
        "and a read refuses rather than pretending there is no family"
    );
}

#[test]
fn founding_at_a_workspace_that_already_holds_a_pointer_refuses() {
    let family = Family::founded();
    let clone = family.root.join("../ws-A");
    fs::create_dir_all(clone.join(".gwz")).unwrap();
    fs::write(
        clone.join(POINTER_RELATIVE_PATH),
        format!(
            "schema: gwz.family-root/v1\nfamily_id: fam_store\nroot_path: {}\n",
            family.root.display()
        ),
    )
    .unwrap();
    let mut session = family
        .store
        .try_lock(&FamilyLocation::new(&clone))
        .expect("the clone locks its root");
    assert_eq!(session.root(), family.root, "a clone locks its root");
    // Founding at the *root* of that family refuses: an index is there.
    match session.found(
        FamilyId::new("fam_second").unwrap(),
        AllocationId::new("alloc_second").unwrap(),
    ) {
        Err(StoreError::ConflictingMetadata { workspace }) => assert_eq!(workspace, family.root),
        other => panic!("founding over an index refuses, got {other:?}"),
    }
    // And a pointer that appears at the root *after* the lock was taken
    // refuses too: `found` rereads under the lock rather than trusting what
    // `try_lock` saw.
    let fresh = family.root.join("../unfounded");
    fs::create_dir_all(&fresh).unwrap();
    let mut session = family
        .store
        .try_lock(&FamilyLocation::new(&fresh))
        .expect("an unfounded workspace locks itself");
    fs::create_dir_all(fresh.join(".gwz")).unwrap();
    fs::write(
        fresh.join(POINTER_RELATIVE_PATH),
        format!(
            "schema: gwz.family-root/v1\nfamily_id: fam_store\nroot_path: {}\n",
            family.root.display()
        ),
    )
    .unwrap();
    match session.found(
        FamilyId::new("fam_second").unwrap(),
        AllocationId::new("alloc_second").unwrap(),
    ) {
        Err(StoreError::ConflictingMetadata { workspace }) => assert_eq!(workspace, fresh),
        other => panic!("founding where a pointer stands refuses, got {other:?}"),
    }
    assert!(
        !fresh.join(INDEX_RELATIVE_PATH).exists(),
        "the refusal wrote no index"
    );
}

#[test]
fn a_clone_whose_pointer_cannot_be_decoded_refuses_as_malformed() {
    let family = Family::founded();
    let clone = family.root.join("../ws-A");
    fs::create_dir_all(clone.join(".gwz")).unwrap();
    let pointer = clone.join(POINTER_RELATIVE_PATH);
    fs::write(&pointer, b"schema: gwz.family-root/v1\nfamily_id: [\n").unwrap();
    match family.store.read_view(&FamilyLocation::new(&clone)) {
        Err(StoreError::Malformed { path, .. }) => assert_eq!(path, pointer),
        other => panic!("an undecodable pointer refuses, got {other:?}"),
    }
    assert!(pointer.is_file(), "the file is retained for inspection");
}

/// LCM1.1 (lane C wiring): `gwz local list` and disposal's fresh evidence
/// read what stands at a row's recorded path -- the index, the pointer and
/// the marker -- through the store, which owns the format, and never write.
/// The observation is keyed by the row so it resolves the recorded path
/// exactly as `install_pointer` and `remove_pointer` do.
#[test]
fn observing_a_members_target_reports_present_missing_and_foreign_metadata() {
    use crate::{WorkspaceMetadata, WorkspaceObservation};
    use gwz_family_model::{MarkerObservation, PointerObservation, TargetObservation};

    let family = Family::founded();
    let mut session = family.lock();
    let destination = family.creating(&mut session, "A", "../ws-A");
    let view = session.reread().unwrap().expect("the index is there");
    let row = view.members.get(&name("A")).unwrap().clone();
    let observe = || {
        family
            .store
            .observe_member_target(&family.root, &view, &row)
    };

    // Allocated but not yet installed: present, holding nothing of the family.
    assert_eq!(
        observe(),
        TargetObservation::Present {
            pointer: PointerObservation::Absent,
            marker: MarkerObservation::Absent,
        }
    );
    session
        .install_pointer(&name("A"), &destination)
        .expect("install");
    assert_eq!(
        observe(),
        TargetObservation::Present {
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Matches,
        }
    );
    // The same reading through the path-keyed form, with the facts unfolded.
    assert_eq!(
        family.store.observe_workspace(
            &destination,
            &view.family_id,
            &family.root,
            &row.allocation_id
        ),
        WorkspaceObservation::Present(WorkspaceMetadata {
            index: false,
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Matches,
        })
    );

    // Another allocation's marker is a mismatch; another family's pointer is
    // foreign; a pointer that does not decode is malformed -- each retained.
    let other_allocation = AllocationId::new("alloc-other").unwrap();
    assert_eq!(
        family.store.observe_workspace(
            &destination,
            &view.family_id,
            &family.root,
            &other_allocation
        ),
        WorkspaceObservation::Present(WorkspaceMetadata {
            index: false,
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Mismatch,
        })
    );
    let other_family = FamilyId::new("fam_other").unwrap();
    assert_eq!(
        family.store.observe_workspace(
            &destination,
            &other_family,
            &family.root,
            &row.allocation_id
        ),
        WorkspaceObservation::Present(WorkspaceMetadata {
            index: false,
            pointer: PointerObservation::OtherFamily,
            marker: MarkerObservation::Mismatch,
        })
    );
    // A pointer naming this family at another root is not this root's pointer
    // (design §11 item 4: fail closed on a family-root mismatch).
    let elsewhere = family.root.join("../elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    assert_eq!(
        family.store.observe_workspace(
            &destination,
            &view.family_id,
            &elsewhere,
            &row.allocation_id
        ),
        WorkspaceObservation::Present(WorkspaceMetadata {
            index: false,
            pointer: PointerObservation::OtherFamily,
            marker: MarkerObservation::Matches,
        })
    );
    fs::write(
        destination.join(POINTER_RELATIVE_PATH),
        b"schema: [broken\n",
    )
    .unwrap();
    assert_eq!(
        observe(),
        TargetObservation::Present {
            pointer: PointerObservation::Malformed,
            marker: MarkerObservation::Matches,
        }
    );

    // An index at the recorded path makes it a root, not a clone.
    fs::copy(
        family.root.join(INDEX_RELATIVE_PATH),
        destination.join(INDEX_RELATIVE_PATH),
    )
    .unwrap();
    assert_eq!(
        observe(),
        TargetObservation::Present {
            pointer: PointerObservation::IsIndex,
            marker: MarkerObservation::Matches,
        }
    );

    // A file where the workspace should be cannot hold metadata; a path that
    // is gone is missing.
    fs::remove_dir_all(&destination).unwrap();
    assert_eq!(observe(), TargetObservation::Missing);
    fs::write(&destination, b"not a directory").unwrap();
    assert!(
        matches!(observe(), TargetObservation::Malformed { .. }),
        "{:?}",
        observe()
    );
}
