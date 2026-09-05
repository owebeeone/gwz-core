//! Store conformance suite and the in-memory fake.
//!
//! Enabled by the `contract-tests` feature (dev-dependencies only). The
//! real store (`gwz-family-store`) runs [`run_all`] through its own
//! [`StoreFixture`]; consumers (`gwz-workspace-install`,
//! `gwz-local-disposal`) use [`InMemoryFamilyStore`], whose sessions are
//! faithful to the lock, reread/validate and partial-effect contract without
//! touching disk.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use gwz_family_model::{
    AllocationId, FamilyChange, FamilyId, FamilyView, MAX_ENCODED_INDEX_BYTES, MemberName,
    MemberRow, MemberState, Refusal, validate_transition,
};

use crate::{
    AppliedChange, FamilyLocation, FamilyObservation, FamilySession, FamilySource, FamilyStore,
    MetadataEffect, StoreError, StoreOperation,
};

/// What the suite needs from an implementation's fixture: fresh locations,
/// materialised member workspaces, and the ability to observe or corrupt the
/// on-disk (or in-memory) state.
pub trait StoreFixture {
    type Store: FamilyStore;

    /// A store plus a fresh root location holding no family. The suite
    /// records member rows at `../<name>` (siblings of the root), so the
    /// root's parent must be fixture-private.
    fn fresh_root(&mut self) -> (Self::Store, FamilyLocation);
    /// The workspace at `relative` (root-relative, spelled as a member row
    /// spells its path), materialised the way the orchestrator allocates a
    /// destination before the store writes into it (design §3 step 2;
    /// LCM1.0c-fu1, Code C2-P3-1): a filesystem fixture `create_dir_all`s the
    /// directory and returns the join; the in-memory fake returns the join.
    /// The suite hands the returned path to `install_pointer` verbatim, so
    /// the store's own resolution -- not the fixture's -- decides that it is
    /// the row's path.
    fn member_workspace(&mut self, root: &Path, relative: &str) -> PathBuf;
    /// Whether the family lock artifact exists at `root`.
    fn lock_artifact_exists(&self, root: &Path) -> bool;
    /// Make the index at `root` undecodable.
    fn corrupt_index(&mut self, root: &Path);
    /// Make the index at `root` larger than the encoded limit.
    fn oversize_index(&mut self, root: &Path);
    /// Script the next occurrence of `operation` (in the family locked at
    /// `root`) to fail, so the suite can drive [`StoreError::Partial`] and
    /// I/O arms against a real store (LCM1.0c-rem1, S-P3-1 / C-P3-3). A
    /// filesystem fixture makes the target unwritable; the in-memory fake
    /// queues a scripted failure.
    fn fail_next(&mut self, root: &Path, operation: StoreOperation);
    /// Clear every obstruction `fail_next` planted at `root` (LCM1.0c
    /// follow-up 3, lane S proposal S-1). The suite calls this after it has
    /// observed the scripted failure and before it retries the same call, so
    /// a fixture whose obstruction is durable on disk (an unwritable path, a
    /// directory planted where the pointer goes) clears it here and needs no
    /// bracketing wrapper of its own. The in-memory fake's queued failure is
    /// consumed by the failing call, so the default body -- nothing -- is
    /// right for it.
    fn clear_failures(&mut self, root: &Path) {
        let _ = root;
    }
    /// A second spelling of the member workspace at `relative`: `alias` is a
    /// root-relative path, spelled as a row spells its path, that the store's
    /// own resolution must resolve to the same directory as `relative` -- a
    /// symlink on a filesystem fixture. Returns the alias joined to `root`, to
    /// record as another row's path and hand to `install_pointer`, or `None`
    /// when the fixture cannot alias, in which case the path-collision case is
    /// skipped for it (LCM1.0c follow-up 3, lane S proposal S-4). The
    /// reference fixture always can, so the case is never vacuous for the
    /// contract.
    fn alias_workspace(&mut self, root: &Path, relative: &str, alias: &str) -> Option<PathBuf> {
        let _ = (root, relative, alias);
        None
    }
}

/// Run every conformance case through `fixture`.
pub fn run_all<F: StoreFixture>(fixture: &mut F) {
    reading_no_family_creates_no_lock_file(fixture);
    founding_then_reading_round_trips_and_reads_create_no_lock(fixture);
    a_second_try_lock_is_busy_until_the_session_drops(fixture);
    apply_refuses_an_invalid_transition_without_effects(fixture);
    apply_writes_only_the_matching_index_change(fixture);
    malformed_and_oversize_indexes_refuse(fixture);
    // LCM1.0c-rem1: the pointer/marker half. These are the family's only
    // multi-file, cross-workspace durable effects and the sole reason
    // `StoreError::Partial` and the pointer/marker effect variants exist, so
    // the real store must be measured on them, not only the in-memory fake
    // (State P3-1 / Code P3-3).
    installing_a_pointer_writes_the_marker_before_the_pointer(fixture);
    a_failed_pointer_write_reports_partial_with_the_marker_completed(fixture);
    remove_pointer_is_repeatable_and_the_second_call_reports_no_effects(fixture);
    installing_a_pointer_into_a_destination_holding_an_index_conflicts(fixture);
    installing_a_pointer_over_another_familys_pointer_is_pointer_target_invalid(fixture);
    removing_the_row_before_the_pointer_is_refused_and_leaves_no_orphan(fixture);
    // LCM1.0c-fu1 (State S2-P3-1, Code C2-P3-2): the destination is the
    // row's recorded path, resolved by the store itself, in every derivation;
    // and the ordering refusal covers `Disband` as well as `RemoveRow`.
    installing_a_pointer_anywhere_but_the_rows_path_is_refused_without_effects(fixture);
    a_pointer_installed_through_a_non_canonical_spelling_is_the_rows_pointer(fixture);
    disbanding_before_the_pointers_is_refused_and_leaves_no_orphan(fixture);
    // LCM1.0c-fu3 (lane S proposal S-4): two rows whose recorded paths
    // resolve to one directory -- a spelling the pure model cannot see --
    // refuse at `install_pointer`, before the destination's own metadata.
    installing_a_pointer_where_another_rows_path_resolves_is_a_path_collision(fixture);
}

/// A founded family holding one `creating` member `A` at `../ws-A`, under a
/// held session.
struct Founded<S: FamilyStore> {
    store: S,
    session: S::Session,
    root: PathBuf,
    name: MemberName,
    /// `A`'s destination as the fixture materialised it (`<root>/../ws-A`).
    destination: PathBuf,
}

fn founded_with_a_creating_member<F: StoreFixture>(fixture: &mut F) -> Founded<F::Store> {
    let (store, location) = fixture.fresh_root();
    let (family_id, root_allocation) = family();
    let mut session = store.try_lock(&location).expect("lock is free");
    session.found(family_id, root_allocation).unwrap();
    let name = MemberName::parse("A").unwrap();
    session
        .apply(&FamilyChange::Allocate {
            name: name.clone(),
            row: creating_row("../ws-A"),
        })
        .unwrap();
    let root = session.root().to_path_buf();
    let destination = fixture.member_workspace(&root, "../ws-A");
    Founded {
        store,
        session,
        root,
        name,
        destination,
    }
}

fn keep(name: &MemberName) -> FamilyChange {
    FamilyChange::RemoveRow {
        name: name.clone(),
        reason: gwz_family_model::RemovalReason::Keep,
    }
}

pub fn installing_a_pointer_writes_the_marker_before_the_pointer<F: StoreFixture>(fixture: &mut F) {
    let Founded {
        mut session,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    let applied = session.install_pointer(&name, &destination).unwrap();
    assert_eq!(
        applied.effects,
        vec![
            MetadataEffect::MarkerWritten {
                workspace: destination.clone(),
            },
            MetadataEffect::PointerWritten {
                workspace: destination,
            },
        ],
        "the marker is written before the pointer"
    );
}

pub fn a_failed_pointer_write_reports_partial_with_the_marker_completed<F: StoreFixture>(
    fixture: &mut F,
) {
    let Founded {
        mut session,
        root,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    fixture.fail_next(&root, StoreOperation::WritePointer);
    match session.install_pointer(&name, &destination) {
        Err(StoreError::Partial {
            operation,
            completed,
            ..
        }) => {
            assert_eq!(operation, StoreOperation::WritePointer);
            assert_eq!(
                completed,
                vec![MetadataEffect::MarkerWritten {
                    workspace: destination.clone(),
                }],
                "the marker write completed before the pointer write failed"
            );
        }
        other => panic!("a failed pointer write must report Partial, got {other:?}"),
    }
    // A retry after the scripted failure completes both effects. A fixture
    // whose obstruction is durable clears it here (S-1); the fake's queued
    // failure was consumed by the call above.
    fixture.clear_failures(&root);
    let applied = session.install_pointer(&name, &destination).unwrap();
    assert_eq!(applied.effects.len(), 2, "the retry completes both effects");
}

/// S-4 (LCM1.0c-fu3): `B`'s recorded path is a second spelling of `A`'s
/// directory. Installing `B`'s pointer there is `Refused(PathCollision)` and
/// has no effect on `A`'s pointer. The refusal is derived from the rows and
/// symmetric -- while both rows stand, an install for *either* refuses, which
/// is why `A` installs before `B` is allocated here -- and it never blocks
/// `remove_pointer`, so `dispose --keep` still clears the mix-up. Skipped for
/// a fixture that cannot alias a directory (`alias_workspace` is `None`).
pub fn installing_a_pointer_where_another_rows_path_resolves_is_a_path_collision<
    F: StoreFixture,
>(
    fixture: &mut F,
) {
    let Founded {
        mut session,
        root,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    let Some(alias) = fixture.alias_workspace(&root, "../ws-A", "../ws-A-alias") else {
        return;
    };
    session.install_pointer(&name, &destination).unwrap();
    let other = MemberName::parse("B").unwrap();
    session
        .apply(&FamilyChange::Allocate {
            name: other.clone(),
            row: creating_row("../ws-A-alias"),
        })
        .expect("the model sees two distinct normalised paths");
    assert_eq!(
        session.install_pointer(&other, &alias).unwrap_err(),
        StoreError::Refused(Refusal::PathCollision {
            path: "../ws-A-alias".to_owned(),
            holder: "A".to_owned(),
        }),
        "the destination is where A's row resolves"
    );
    assert_eq!(
        session.install_pointer(&name, &destination).unwrap_err(),
        StoreError::Refused(Refusal::PathCollision {
            path: "../ws-A".to_owned(),
            holder: "B".to_owned(),
        }),
        "symmetric: while both rows stand, A cannot re-install either"
    );
    let removed = session.remove_pointer(&name).unwrap();
    assert!(
        removed.effects.contains(&MetadataEffect::PointerRemoved {
            workspace: destination.clone(),
        }),
        "A's pointer stood untouched and is removable: {removed:?}"
    );
    assert!(
        session.remove_pointer(&other).unwrap().effects.is_empty(),
        "B never had a pointer"
    );
}

pub fn remove_pointer_is_repeatable_and_the_second_call_reports_no_effects<F: StoreFixture>(
    fixture: &mut F,
) {
    let Founded {
        mut session,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    session.install_pointer(&name, &destination).unwrap();
    let removed = session.remove_pointer(&name).unwrap();
    assert!(
        removed.effects.contains(&MetadataEffect::PointerRemoved {
            workspace: destination.clone(),
        }),
        "the first removal reports the pointer removal: {removed:?}"
    );
    let again = session.remove_pointer(&name).unwrap();
    assert!(
        again.effects.is_empty(),
        "a repeat removal reports no effects: {again:?}"
    );
}

pub fn installing_a_pointer_into_a_destination_holding_an_index_conflicts<F: StoreFixture>(
    fixture: &mut F,
) {
    // Family one records member `B` at `../root-two`, where family two has
    // been founded: the row's OWN destination holds an index, so installing
    // the pointer would make one workspace hold both an index and a pointer.
    // (LCM1.0c-fu1, State S2-P3-1: the destination must be the row's path,
    // so the conflicting directory is reached through a row, not by handing
    // `install_pointer` the root.)
    let Founded {
        store,
        mut session,
        root,
        ..
    } = founded_with_a_creating_member(fixture);
    let root_two = fixture.member_workspace(&root, "../root-two");
    store
        .try_lock(&FamilyLocation::new(&root_two))
        .unwrap()
        .found(
            FamilyId::new("fam_two").unwrap(),
            AllocationId::new("alloc_two").unwrap(),
        )
        .unwrap();
    let b = MemberName::parse("B").unwrap();
    session
        .apply(&FamilyChange::Allocate {
            name: b.clone(),
            row: creating_row("../root-two"),
        })
        .unwrap();
    match session.install_pointer(&b, &root_two) {
        Err(StoreError::ConflictingMetadata { .. }) => {}
        other => panic!("a destination holding an index must conflict, got {other:?}"),
    }
}

pub fn installing_a_pointer_over_another_familys_pointer_is_pointer_target_invalid<
    F: StoreFixture,
>(
    fixture: &mut F,
) {
    // Family one installs `A`'s pointer at its own recorded destination.
    let Founded {
        store,
        session: mut first,
        root: root_one,
        name: a,
        destination: shared,
    } = founded_with_a_creating_member(fixture);
    first.install_pointer(&a, &shared).unwrap();
    drop(first);

    // Family two at a sibling root records `B` at `../ws-A` as well: the same
    // directory, reached through family two's own root (`<root-two>/../ws-A`)
    // -- `B`'s recorded path, not a divergent destination (LCM1.0c-fu1, State
    // S2-P3-1). That destination already holds family one's pointer.
    let root_two = fixture.member_workspace(&root_one, "../root-two");
    let mut second = store.try_lock(&FamilyLocation::new(&root_two)).unwrap();
    second
        .found(
            FamilyId::new("fam_two").unwrap(),
            AllocationId::new("alloc_two").unwrap(),
        )
        .unwrap();
    let b = MemberName::parse("B").unwrap();
    second
        .apply(&FamilyChange::Allocate {
            name: b.clone(),
            row: creating_row("../ws-A"),
        })
        .unwrap();
    let own = fixture.member_workspace(&root_two, "../ws-A");
    match second.install_pointer(&b, &own) {
        Err(StoreError::PointerTargetInvalid { .. }) => {}
        other => panic!("a destination pointing at another family must refuse, got {other:?}"),
    }
}

pub fn removing_the_row_before_the_pointer_is_refused_and_leaves_no_orphan<F: StoreFixture>(
    fixture: &mut F,
) {
    // State P2-2: the orphaning order (remove the row before its pointer) is
    // refused by the store, so a pointer whose row is gone is never produced;
    // and the required order (pointer first) leaves no orphan.
    let Founded {
        mut session,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    session.install_pointer(&name, &destination).unwrap();

    // Removing the row while its pointer stands is refused.
    match session.apply(&keep(&name)) {
        Err(StoreError::PointerStillInstalled { member, .. }) => {
            assert_eq!(
                member, "A",
                "the refusal names the row whose pointer stands"
            );
        }
        other => panic!("removing a row before its pointer must be refused, got {other:?}"),
    }
    // The row and its pointer both still stand: nothing was orphaned.
    let view = session
        .reread()
        .unwrap()
        .expect("the family index is intact");
    assert!(view.members.contains_key(&name), "the row is untouched");

    // The required order leaves no orphan: pointer first, then the row.
    let removed = session.remove_pointer(&name).unwrap();
    assert!(
        removed.effects.contains(&MetadataEffect::PointerRemoved {
            workspace: destination,
        }),
        "the pointer is removed first"
    );
    session
        .apply(&keep(&name))
        .expect("removing the row after its pointer succeeds");
    assert!(
        !session
            .reread()
            .unwrap()
            .unwrap()
            .members
            .contains_key(&name),
        "the row is gone and no pointer was stranded"
    );
}

pub fn installing_a_pointer_anywhere_but_the_rows_path_is_refused_without_effects<
    F: StoreFixture,
>(
    fixture: &mut F,
) {
    // LCM1.0c-fu1 (State S2-P3-1): the destination is not a free argument. A
    // pointer installed anywhere but the store's own resolution of the row's
    // recorded path would be unreachable by `remove_pointer` and invisible to
    // the RemoveRow/Disband guard -- the S-P2-2 orphan by another door -- so
    // the store refuses before any effect.
    let Founded {
        store,
        mut session,
        root,
        name,
        destination,
    } = founded_with_a_creating_member(fixture);
    let elsewhere = fixture.member_workspace(&root, "../ws-elsewhere");
    match session.install_pointer(&name, &elsewhere) {
        Err(StoreError::PathMismatch { member, .. }) => {
            assert_eq!(member, "A", "the refusal names the member");
        }
        other => panic!(
            "a destination other than the row's recorded path must be refused as \
             PathMismatch, got {other:?}"
        ),
    }
    // Nothing was written at either path...
    assert_eq!(
        store.read_view(&FamilyLocation::new(&elsewhere)).unwrap(),
        FamilyObservation::NoFamily,
        "nothing was written at the divergent destination"
    );
    assert_eq!(
        store.read_view(&FamilyLocation::new(&destination)).unwrap(),
        FamilyObservation::NoFamily,
        "nothing was written at the row's path either"
    );
    let removed = session.remove_pointer(&name).unwrap();
    assert!(
        removed.effects.is_empty(),
        "there is no pointer to remove: {removed:?}"
    );
    // ...so the row is not protected by a pointer and may be removed.
    session
        .apply(&keep(&name))
        .expect("no pointer stands, so the row may be removed");
}

pub fn a_pointer_installed_through_a_non_canonical_spelling_is_the_rows_pointer<F: StoreFixture>(
    fixture: &mut F,
) {
    // LCM1.0c-fu1 (State S2-P3-1) closure: `install_pointer`'s destination
    // check, `remove_pointer` and the RemoveRow/Disband guard resolve the
    // row's path the same way, so a pointer installed through ANY spelling of
    // that path is the one `remove_pointer` finds and the one that protects
    // the row. A store whose three derivations disagree strands the pointer
    // here, in the suite, instead of in a user's family.
    let Founded {
        mut session,
        root,
        name,
        ..
    } = founded_with_a_creating_member(fixture);
    let spelled = root.join("./../ws-A");
    let applied = session.install_pointer(&name, &spelled).unwrap();
    assert_eq!(
        applied.effects.len(),
        2,
        "the non-canonical spelling of the row's path is accepted: {applied:?}"
    );
    match session.apply(&keep(&name)) {
        Err(StoreError::PointerStillInstalled { member, .. }) => {
            assert_eq!(member, "A", "the guard sees the pointer through the row");
        }
        other => panic!("the row is protected by the pointer it reaches, got {other:?}"),
    }
    let removed = session.remove_pointer(&name).unwrap();
    assert!(
        removed
            .effects
            .iter()
            .any(|effect| matches!(effect, MetadataEffect::PointerRemoved { .. })),
        "remove_pointer finds the pointer installed through the other spelling: {removed:?}"
    );
    session
        .apply(&keep(&name))
        .expect("with the pointer gone the row may be removed");
}

pub fn disbanding_before_the_pointers_is_refused_and_leaves_no_orphan<F: StoreFixture>(
    fixture: &mut F,
) {
    // LCM1.0c-fu1 (Code C2-P3-2): the second path of the State P2-2 hazard.
    // Disbanding removes the index, so a pointer still installed at that
    // moment would have no row to reach it through; the store refuses until
    // every pointer of the family is gone, then disbands.
    let Founded {
        mut session,
        name,
        destination,
        ..
    } = founded_with_a_creating_member(fixture);
    session.install_pointer(&name, &destination).unwrap();
    match session.apply(&FamilyChange::Disband) {
        Err(StoreError::PointerStillInstalled { member, .. }) => {
            assert_eq!(
                member, "A",
                "the refusal names the member whose pointer stands"
            );
        }
        other => panic!("disbanding before the pointers must be refused, got {other:?}"),
    }
    let view = session.reread().unwrap().expect("the index is intact");
    assert!(view.members.contains_key(&name), "the row is untouched");

    session.remove_pointer(&name).unwrap();
    let disbanded = session
        .apply(&FamilyChange::Disband)
        .expect("disband succeeds once the pointers are gone");
    assert_eq!(disbanded.effects, vec![MetadataEffect::IndexRemoved]);
    assert_eq!(session.reread().unwrap(), None, "the index is gone");
}

fn family() -> (FamilyId, AllocationId) {
    (
        FamilyId::new("fam_conformance").unwrap(),
        AllocationId::new("alloc_root").unwrap(),
    )
}

fn creating_row(path: &str) -> gwz_family_model::MemberRow {
    gwz_family_model::MemberRow {
        path: path.to_owned(),
        kind: gwz_family_model::MemberKind::Checkout,
        state: MemberState::Creating,
        allocation_id: AllocationId::new(format!("alloc{path}")).unwrap(),
        source_path: ".".to_owned(),
        mode: gwz_family_model::CloneMode::Verbatim,
        last_error: None,
    }
}

pub fn reading_no_family_creates_no_lock_file<F: StoreFixture>(fixture: &mut F) {
    let (store, location) = fixture.fresh_root();
    let observation = store.read_view(&location).expect("read succeeds");
    assert_eq!(observation, FamilyObservation::NoFamily);
    assert!(
        !fixture.lock_artifact_exists(&location.workspace),
        "a read never creates the lock file"
    );
}

pub fn founding_then_reading_round_trips_and_reads_create_no_lock<F: StoreFixture>(
    fixture: &mut F,
) {
    let (store, location) = fixture.fresh_root();
    let (family_id, root_allocation) = family();
    {
        let mut session = store.try_lock(&location).expect("lock is free");
        assert_eq!(session.reread().unwrap(), None, "no index before founding");
        let applied = session
            .found(family_id.clone(), root_allocation.clone())
            .unwrap();
        assert_eq!(applied.effects, vec![MetadataEffect::IndexWritten]);
        assert!(
            session
                .found(family_id.clone(), root_allocation.clone())
                .is_err(),
            "founding twice refuses"
        );
    }
    let observation = store.read_view(&location).unwrap();
    let FamilyObservation::Family { source, view, root } = observation else {
        panic!("a founded family is observed");
    };
    assert_eq!(source, FamilySource::Index);
    assert_eq!(root, location.workspace);
    assert_eq!(view, FamilyView::founded(family_id, root_allocation));
    let _ = store.read_view(&location).unwrap();
}

pub fn a_second_try_lock_is_busy_until_the_session_drops<F: StoreFixture>(fixture: &mut F) {
    let (store, location) = fixture.fresh_root();
    let session = store.try_lock(&location).expect("first lock");
    match store.try_lock(&location) {
        Err(StoreError::Busy { .. }) => {}
        other => panic!("second lock must be Busy, got {:?}", other.err()),
    }
    drop(session);
    let again = store.try_lock(&location);
    assert!(again.is_ok(), "the lock is released with the session");
}

pub fn apply_refuses_an_invalid_transition_without_effects<F: StoreFixture>(fixture: &mut F) {
    let (store, location) = fixture.fresh_root();
    let (family_id, root_allocation) = family();
    let mut session = store.try_lock(&location).unwrap();
    session.found(family_id, root_allocation).unwrap();
    let before = session.reread().unwrap();
    let error = session
        .apply(&FamilyChange::MarkReady {
            name: MemberName::parse("ghost").unwrap(),
            expected_allocation: AllocationId::new("x").unwrap(),
        })
        .expect_err("an unknown row refuses");
    assert!(matches!(error, StoreError::Refused(_)), "{error:?}");
    assert_eq!(
        session.reread().unwrap(),
        before,
        "a refusal writes nothing"
    );
}

pub fn apply_writes_only_the_matching_index_change<F: StoreFixture>(fixture: &mut F) {
    let (store, location) = fixture.fresh_root();
    let (family_id, root_allocation) = family();
    let mut session = store.try_lock(&location).unwrap();
    session.found(family_id, root_allocation).unwrap();
    let name = MemberName::parse("A").unwrap();
    let applied = session
        .apply(&FamilyChange::Allocate {
            name: name.clone(),
            row: creating_row("../ws-A"),
        })
        .unwrap();
    assert_eq!(applied.effects, vec![MetadataEffect::IndexWritten]);
    let view = applied.view.expect("index remains");
    assert_eq!(view.members[&name].state, MemberState::Creating);
    let reread = session.reread().unwrap().unwrap();
    assert_eq!(reread, view, "reread observes exactly the written index");
    let disband = session.apply(&FamilyChange::Disband).unwrap();
    assert_eq!(
        disband.effects,
        vec![MetadataEffect::IndexRemoved],
        "disband removes the index and nothing else"
    );
    assert_eq!(session.reread().unwrap(), None);
    let again = session.apply(&FamilyChange::Disband);
    assert!(again.is_ok(), "disband is repeatable: {again:?}");
}

pub fn malformed_and_oversize_indexes_refuse<F: StoreFixture>(fixture: &mut F) {
    let (store, location) = fixture.fresh_root();
    let (family_id, root_allocation) = family();
    store
        .try_lock(&location)
        .unwrap()
        .found(family_id.clone(), root_allocation.clone())
        .unwrap();
    fixture.corrupt_index(&location.workspace);
    match store.read_view(&location) {
        Err(StoreError::Malformed { .. }) => {}
        other => panic!("a malformed index refuses, got {other:?}"),
    }

    let (store, location) = fixture.fresh_root();
    store
        .try_lock(&location)
        .unwrap()
        .found(family_id, root_allocation)
        .unwrap();
    fixture.oversize_index(&location.workspace);
    match store.read_view(&location) {
        Err(StoreError::Oversize { limit, .. }) => assert_eq!(limit, MAX_ENCODED_INDEX_BYTES),
        other => panic!("an oversize index refuses, got {other:?}"),
    }
}

/// The fake's own resolution of a workspace path (the contract's "one
/// canonical resolution", call-order clause): lexical, so `<root>/../ws-A`,
/// `<root>/./../ws-A` and `<root>/../root-two/../ws-A` are one key. A
/// filesystem store canonicalises through the filesystem instead; what the
/// contract requires is that one resolution serves `install_pointer`'s
/// destination check, `remove_pointer` and the `RemoveRow`/`Disband` guard.
fn resolve(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match resolved.components().next_back() {
                Some(Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => resolved.push(".."),
            },
            other => resolved.push(other),
        }
    }
    resolved
}

/// In-memory family state shared by a store and its sessions. Every key is
/// a [`resolve`]d workspace path.
#[derive(Debug, Default)]
struct Families {
    /// Root workspace -> its index, when founded.
    indexes: BTreeMap<PathBuf, IndexState>,
    /// Clone workspace -> (family id, root workspace).
    pointers: BTreeMap<PathBuf, (FamilyId, PathBuf)>,
    /// Clone workspace -> allocation id.
    markers: BTreeMap<PathBuf, AllocationId>,
    /// A second lexical spelling of a workspace -> the spelling every map is
    /// keyed by (S-4; a filesystem store sees the same through a symlink).
    aliases: BTreeMap<PathBuf, PathBuf>,
    /// Roots whose lock is currently held.
    locked: Vec<PathBuf>,
    /// Roots whose lock file has ever been created.
    lock_files: Vec<PathBuf>,
    /// Operations scripted to fail next (first match wins, consumed).
    failures: Vec<StoreOperation>,
}

#[derive(Clone, Debug)]
enum IndexState {
    Valid(FamilyView),
    Malformed,
    Oversize(u64),
}

/// A contract-faithful in-memory store.
#[derive(Clone, Debug, Default)]
pub struct InMemoryFamilyStore {
    families: Rc<RefCell<Families>>,
}

impl InMemoryFamilyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next occurrence of `operation` with an I/O error (or a
    /// `Partial` error when the operation has an effect that precedes the
    /// failing write).
    pub fn fail_next(&self, operation: StoreOperation) {
        self.families.borrow_mut().failures.push(operation);
    }

    pub fn lock_file_exists(&self, root: &Path) -> bool {
        self.families.borrow().lock_files.contains(&resolve(root))
    }

    /// Make `alias` a second spelling of the workspace `target`: both key the
    /// same maps from now on (S-4).
    pub fn alias(&self, alias: &Path, target: &Path) {
        self.families
            .borrow_mut()
            .aliases
            .insert(resolve(alias), resolve(target));
    }

    /// The store's own resolution of a workspace path: [`resolve`] and then
    /// the alias table, so two spellings of one directory are one key.
    fn key(&self, workspace: &Path) -> PathBuf {
        let lexical = resolve(workspace);
        self.families
            .borrow()
            .aliases
            .get(&lexical)
            .cloned()
            .unwrap_or(lexical)
    }

    pub fn corrupt_index(&self, root: &Path) {
        self.families
            .borrow_mut()
            .indexes
            .insert(resolve(root), IndexState::Malformed);
    }

    pub fn oversize_index(&self, root: &Path) {
        self.families.borrow_mut().indexes.insert(
            resolve(root),
            IndexState::Oversize(MAX_ENCODED_INDEX_BYTES + 1),
        );
    }

    /// Clone pointers currently written (resolved workspace -> root).
    pub fn pointers(&self) -> BTreeMap<PathBuf, PathBuf> {
        self.families
            .borrow()
            .pointers
            .iter()
            .map(|(workspace, (_, root))| (workspace.clone(), root.clone()))
            .collect()
    }

    fn take_failure(&self, operation: StoreOperation) -> bool {
        let mut families = self.families.borrow_mut();
        if let Some(index) = families.failures.iter().position(|op| *op == operation) {
            families.failures.remove(index);
            return true;
        }
        false
    }

    fn resolve_root(
        &self,
        workspace: &Path,
    ) -> Result<Option<(PathBuf, FamilySource)>, StoreError> {
        let workspace = self.key(workspace);
        let families = self.families.borrow();
        let has_index = families.indexes.contains_key(&workspace);
        let pointer = families.pointers.get(&workspace).cloned();
        match (has_index, pointer) {
            (true, Some(_)) => Err(StoreError::ConflictingMetadata { workspace }),
            (true, None) => Ok(Some((workspace, FamilySource::Index))),
            (false, Some((family_id, root))) => match families.indexes.get(&root) {
                Some(IndexState::Valid(view)) if view.family_id == family_id => {
                    Ok(Some((root, FamilySource::Pointer)))
                }
                Some(IndexState::Valid(_)) | None => Err(StoreError::PointerTargetInvalid {
                    pointer: workspace.join(gwz_family_model::POINTER_RELATIVE_PATH),
                    root,
                    detail: "no index for this family at the pointed root".to_owned(),
                }),
                Some(_) => Ok(Some((root, FamilySource::Pointer))),
            },
            (false, None) => Ok(None),
        }
    }

    fn read_index(&self, root: &Path) -> Result<Option<FamilyView>, StoreError> {
        if self.take_failure(StoreOperation::ReadIndex) {
            return Err(StoreError::Io {
                operation: StoreOperation::ReadIndex,
                path: root.join(gwz_family_model::INDEX_RELATIVE_PATH),
                detail: "scripted read failure".to_owned(),
            });
        }
        let families = self.families.borrow();
        match families.indexes.get(&resolve(root)) {
            None => Ok(None),
            Some(IndexState::Valid(view)) => Ok(Some(view.clone())),
            Some(IndexState::Malformed) => Err(StoreError::Malformed {
                path: root.join(gwz_family_model::INDEX_RELATIVE_PATH),
                detail: "scripted malformed index".to_owned(),
            }),
            Some(IndexState::Oversize(bytes)) => Err(StoreError::Oversize {
                path: root.join(gwz_family_model::INDEX_RELATIVE_PATH),
                bytes: *bytes,
                limit: MAX_ENCODED_INDEX_BYTES,
            }),
        }
    }
}

impl FamilyStore for InMemoryFamilyStore {
    type Session = InMemorySession;

    fn read_view(&self, location: &FamilyLocation) -> Result<FamilyObservation, StoreError> {
        let Some((root, source)) = self.resolve_root(&location.workspace)? else {
            return Ok(FamilyObservation::NoFamily);
        };
        let view = self
            .read_index(&root)?
            .ok_or_else(|| StoreError::NoFamily {
                workspace: location.workspace.clone(),
            })?;
        Ok(FamilyObservation::Family { root, source, view })
    }

    fn try_lock(&self, location: &FamilyLocation) -> Result<Self::Session, StoreError> {
        let root = match self.resolve_root(&location.workspace)? {
            Some((root, _)) => root,
            None => resolve(&location.workspace),
        };
        if self.take_failure(StoreOperation::Lock) {
            return Err(StoreError::Io {
                operation: StoreOperation::Lock,
                path: root.join(gwz_family_model::LOCK_RELATIVE_PATH),
                detail: "scripted lock failure".to_owned(),
            });
        }
        let mut families = self.families.borrow_mut();
        if families.locked.contains(&root) {
            return Err(StoreError::Busy {
                lock_path: root.join(gwz_family_model::LOCK_RELATIVE_PATH),
            });
        }
        families.locked.push(root.clone());
        if !families.lock_files.contains(&root) {
            families.lock_files.push(root.clone());
        }
        drop(families);
        Ok(InMemorySession {
            store: self.clone(),
            root,
        })
    }
}

/// A held in-memory lock; dropping it releases the root.
#[derive(Debug)]
pub struct InMemorySession {
    store: InMemoryFamilyStore,
    root: PathBuf,
}

impl InMemorySession {
    /// A row's recorded path resolved against the root, as the effects and
    /// refusals spell it.
    fn recorded_workspace(&self, row: &MemberRow) -> PathBuf {
        self.root.join(&row.path)
    }

    /// The one derivation `install_pointer`'s destination check,
    /// `remove_pointer` and the `RemoveRow`/`Disband` guard share: the row's
    /// recorded path, when this family's pointer stands there.
    fn installed_pointer_of(&self, view: &FamilyView, row: &MemberRow) -> Option<PathBuf> {
        let workspace = self.recorded_workspace(row);
        let stands = self
            .store
            .families
            .borrow()
            .pointers
            .get(&self.store.key(&workspace))
            .is_some_and(|(family_id, _)| *family_id == view.family_id);
        stands.then_some(workspace)
    }

    fn write_index(&self, view: Option<FamilyView>) -> Result<MetadataEffect, StoreError> {
        let (operation, effect) = match &view {
            Some(_) => (StoreOperation::WriteIndex, MetadataEffect::IndexWritten),
            None => (StoreOperation::RemoveIndex, MetadataEffect::IndexRemoved),
        };
        if self.store.take_failure(operation) {
            return Err(StoreError::Io {
                operation,
                path: self.root.join(gwz_family_model::INDEX_RELATIVE_PATH),
                detail: "scripted write failure".to_owned(),
            });
        }
        let mut families = self.store.families.borrow_mut();
        match view {
            Some(view) => {
                families
                    .indexes
                    .insert(self.root.clone(), IndexState::Valid(view));
            }
            None => {
                families.indexes.remove(&self.root);
            }
        }
        Ok(effect)
    }
}

impl Drop for InMemorySession {
    fn drop(&mut self) {
        self.store
            .families
            .borrow_mut()
            .locked
            .retain(|root| root != &self.root);
    }
}

impl FamilySession for InMemorySession {
    fn root(&self) -> &Path {
        &self.root
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        self.store.read_index(&self.root)
    }

    fn found(
        &mut self,
        family_id: FamilyId,
        root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        if self.reread()?.is_some() {
            return Err(StoreError::ConflictingMetadata {
                workspace: self.root.clone(),
            });
        }
        if self
            .store
            .families
            .borrow()
            .pointers
            .contains_key(&self.root)
        {
            return Err(StoreError::ConflictingMetadata {
                workspace: self.root.clone(),
            });
        }
        let view = FamilyView::founded(family_id, root_allocation);
        let effect = self.write_index(Some(view.clone()))?;
        Ok(AppliedChange {
            effects: vec![effect],
            view: Some(view),
        })
    }

    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        let current = self.reread()?;
        let Some(current) = current else {
            if matches!(change, FamilyChange::Disband) {
                return Ok(AppliedChange {
                    effects: Vec::new(),
                    view: None,
                });
            }
            return Err(StoreError::NoFamily {
                workspace: self.root.clone(),
            });
        };
        // LCM1.0c-rem1 (State P2-2): a row may not be removed, and the family
        // may not be disbanded, while a clone pointer this family installed is
        // still present -- that order strands a pointer with no row to reach
        // it through. Refuse it, so the store never silently accepts the
        // orphaning order. Both arms derive the pointer from the ROW (its
        // recorded path, resolved as `install_pointer` resolved it), which is
        // the only derivation a filesystem store has (LCM1.0c-fu1, State
        // S2-P3-1 / Code C2-P3-2).
        let stranded = match change {
            FamilyChange::RemoveRow { name, .. } => current
                .members
                .get(name)
                .and_then(|row| self.installed_pointer_of(&current, row))
                .map(|workspace| (name.clone(), workspace)),
            FamilyChange::Disband => current.members.iter().find_map(|(name, row)| {
                self.installed_pointer_of(&current, row)
                    .map(|workspace| (name.clone(), workspace))
            }),
            _ => None,
        };
        if let Some((member, workspace)) = stranded {
            return Err(StoreError::PointerStillInstalled {
                member: member.as_str().to_owned(),
                workspace,
            });
        }
        let validated = validate_transition(&current, change)?;
        let next = match change {
            FamilyChange::Disband => None,
            _ => Some(validated.next),
        };
        let effect = self.write_index(next.clone())?;
        Ok(AppliedChange {
            effects: vec![effect],
            view: next,
        })
    }

    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        let view = self.reread()?.ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        let row = view
            .members
            .get(name)
            .ok_or_else(|| gwz_family_model::Refusal::NotFound { name: name.clone() })?;
        if row.state != MemberState::Creating {
            return Err(gwz_family_model::Refusal::WrongState {
                name: name.clone(),
                expected: MemberState::Creating,
                actual: row.state,
            }
            .into());
        }
        // LCM1.0c-fu1 (State S2-P3-1): the destination must resolve to the
        // row's recorded path -- any spelling of it, nothing else -- so the
        // pointer written here is the one `remove_pointer` and the guard find.
        let recorded = self.recorded_workspace(row);
        let key = self.store.key(destination);
        if key != self.store.key(&recorded) {
            return Err(StoreError::PathMismatch {
                member: name.as_str().to_owned(),
                recorded,
                requested: destination.to_path_buf(),
            });
        }
        // LCM1.0c-fu3 (S-4): another row whose recorded path resolves to the
        // same directory, refused before the destination's own metadata.
        if let Some((holder, _)) = view.members.iter().find(|(other, other_row)| {
            *other != name && self.store.key(&self.recorded_workspace(other_row)) == key
        }) {
            return Err(gwz_family_model::Refusal::PathCollision {
                path: row.path.clone(),
                holder: holder.as_str().to_owned(),
            }
            .into());
        }
        {
            let families = self.store.families.borrow();
            if families.indexes.contains_key(&key) {
                return Err(StoreError::ConflictingMetadata {
                    workspace: destination.to_path_buf(),
                });
            }
            if let Some((family_id, _)) = families.pointers.get(&key)
                && *family_id != view.family_id
            {
                return Err(StoreError::PointerTargetInvalid {
                    pointer: destination.join(gwz_family_model::POINTER_RELATIVE_PATH),
                    root: self.root.clone(),
                    detail: "destination already points at another family".to_owned(),
                });
            }
        }
        let mut effects = Vec::new();
        if self.store.take_failure(StoreOperation::WriteMarker) {
            return Err(StoreError::Io {
                operation: StoreOperation::WriteMarker,
                path: destination.join(gwz_family_model::ALLOCATION_MARKER_RELATIVE_PATH),
                detail: "scripted marker failure".to_owned(),
            });
        }
        self.store
            .families
            .borrow_mut()
            .markers
            .insert(key.clone(), row.allocation_id.clone());
        effects.push(MetadataEffect::MarkerWritten {
            workspace: destination.to_path_buf(),
        });
        if self.store.take_failure(StoreOperation::WritePointer) {
            return Err(StoreError::Partial {
                operation: StoreOperation::WritePointer,
                completed: effects,
                path: destination.join(gwz_family_model::POINTER_RELATIVE_PATH),
                detail: "scripted pointer failure".to_owned(),
            });
        }
        self.store
            .families
            .borrow_mut()
            .pointers
            .insert(key, (view.family_id.clone(), self.root.clone()));
        effects.push(MetadataEffect::PointerWritten {
            workspace: destination.to_path_buf(),
        });
        Ok(AppliedChange {
            effects,
            view: Some(view),
        })
    }

    fn remove_pointer(&mut self, name: &MemberName) -> Result<AppliedChange, StoreError> {
        let view = self.reread()?.ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        let row = view
            .members
            .get(name)
            .ok_or_else(|| gwz_family_model::Refusal::NotFound { name: name.clone() })?;
        let workspace = self.recorded_workspace(row);
        let key = self.store.key(&workspace);
        let mut effects = Vec::new();
        let mut families = self.store.families.borrow_mut();
        let matches = families
            .pointers
            .get(&key)
            .is_some_and(|(family_id, _)| *family_id == view.family_id);
        if matches {
            families.pointers.remove(&key);
            effects.push(MetadataEffect::PointerRemoved {
                workspace: workspace.clone(),
            });
        }
        if families.markers.remove(&key).is_some() {
            effects.push(MetadataEffect::MarkerRemoved { workspace });
        }
        drop(families);
        Ok(AppliedChange {
            effects,
            view: Some(view),
        })
    }
}

/// The in-memory store's own fixture, so the suite is exercised here.
#[derive(Default)]
pub struct InMemoryFixture {
    /// One store per `fresh_root`, keyed by the root it was handed, so
    /// `fail_next` routes a scripted failure to the family that owns `root`.
    roots: Vec<(PathBuf, InMemoryFamilyStore)>,
    counter: u32,
    /// How often the suite asked for obstructions to be cleared (S-1) and for
    /// an alias (S-4): proof the corresponding cases ran against this fixture.
    pub clear_failures_calls: u32,
    pub alias_calls: u32,
}

impl InMemoryFixture {
    fn stores(&self) -> impl Iterator<Item = &InMemoryFamilyStore> {
        self.roots.iter().map(|(_, store)| store)
    }
}

impl StoreFixture for InMemoryFixture {
    type Store = InMemoryFamilyStore;

    fn fresh_root(&mut self) -> (InMemoryFamilyStore, FamilyLocation) {
        self.counter += 1;
        let store = InMemoryFamilyStore::new();
        let root = PathBuf::from(format!("/mem/root-{}", self.counter));
        self.roots.push((root.clone(), store.clone()));
        (store, FamilyLocation::new(root))
    }

    fn member_workspace(&mut self, root: &Path, relative: &str) -> PathBuf {
        // Nothing to materialise: the fake's workspaces are map keys.
        root.join(relative)
    }

    fn lock_artifact_exists(&self, root: &Path) -> bool {
        self.stores().any(|store| store.lock_file_exists(root))
    }

    fn corrupt_index(&mut self, root: &Path) {
        for store in self.stores() {
            store.corrupt_index(root);
        }
    }

    fn oversize_index(&mut self, root: &Path) {
        for store in self.stores() {
            store.oversize_index(root);
        }
    }

    fn fail_next(&mut self, root: &Path, operation: StoreOperation) {
        // Route to the store handed this root; the store queues the failure
        // for the next matching operation (a filesystem fixture would instead
        // make the target path unwritable). An unknown root is a suite bug,
        // not a reason to script some other family's failure (LCM1.0c-fu1,
        // Code round-2 residual).
        let root = resolve(root);
        let (_, store) = self
            .roots
            .iter()
            .find(|(handed, _)| resolve(handed) == root)
            .unwrap_or_else(|| {
                panic!(
                    "fail_next: {} is not a root this fixture handed out",
                    root.display()
                )
            });
        store.fail_next(operation);
    }

    fn clear_failures(&mut self, root: &Path) {
        // The queued failure was consumed by the call that failed; there is
        // nothing durable to clear. Counted so the suite's call is observable.
        self.store_for(root);
        self.clear_failures_calls += 1;
    }

    fn alias_workspace(&mut self, root: &Path, relative: &str, alias: &str) -> Option<PathBuf> {
        let aliased = root.join(alias);
        self.store_for(root).alias(&aliased, &root.join(relative));
        self.alias_calls += 1;
        Some(aliased)
    }
}

impl InMemoryFixture {
    /// The store handed `root`; an unknown root is a suite bug.
    fn store_for(&self, root: &Path) -> &InMemoryFamilyStore {
        let root = resolve(root);
        self.roots
            .iter()
            .find(|(handed, _)| resolve(handed) == root)
            .map(|(_, store)| store)
            .unwrap_or_else(|| panic!("{} is not a root this fixture handed out", root.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_satisfies_the_conformance_suite() {
        let mut fixture = InMemoryFixture::default();
        run_all(&mut fixture);
        // LCM1.0c-fu3: the S-1 clearing hook and the S-4 alias hook were each
        // exercised exactly once, so neither case was vacuous for the fake.
        assert_eq!(fixture.clear_failures_calls, 1);
        assert_eq!(fixture.alias_calls, 1);
    }

    #[test]
    fn the_fakes_resolution_is_lexical_and_root_bounded() {
        assert_eq!(
            resolve(Path::new("/mem/root-1/../ws-A")),
            Path::new("/mem/ws-A")
        );
        assert_eq!(
            resolve(Path::new("/mem/root-1/./../ws-A")),
            Path::new("/mem/ws-A")
        );
        assert_eq!(
            resolve(Path::new("/mem/root-1/../root-two/../ws-A")),
            Path::new("/mem/ws-A")
        );
        assert_eq!(resolve(Path::new("/a/../../b")), Path::new("/b"));
        assert_eq!(resolve(Path::new("../x/../y")), Path::new("../y"));
    }

    #[test]
    fn fail_next_on_a_root_the_fixture_never_handed_out_panics() {
        let mut fixture = InMemoryFixture::default();
        let _ = fixture.fresh_root();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fixture.fail_next(Path::new("/mem/never"), StoreOperation::WritePointer);
        }));
        assert!(outcome.is_err(), "an unknown root is refused, not rerouted");
    }

    #[test]
    fn pointer_installation_reports_partial_effects_and_follows_pointers() {
        let store = InMemoryFamilyStore::new();
        let root = FamilyLocation::new("/mem/root");
        let mut session = store.try_lock(&root).unwrap();
        session
            .found(
                FamilyId::new("fam").unwrap(),
                AllocationId::new("alloc_root").unwrap(),
            )
            .unwrap();
        let name = MemberName::parse("A").unwrap();
        session
            .apply(&FamilyChange::Allocate {
                name: name.clone(),
                row: creating_row("../ws-A"),
            })
            .unwrap();
        store.fail_next(StoreOperation::WritePointer);
        let destination = PathBuf::from("/mem/root/../ws-A");
        let error = session.install_pointer(&name, &destination).unwrap_err();
        match error {
            StoreError::Partial {
                operation,
                completed,
                ..
            } => {
                assert_eq!(operation, StoreOperation::WritePointer);
                assert_eq!(
                    completed,
                    vec![MetadataEffect::MarkerWritten {
                        workspace: destination.clone()
                    }]
                );
            }
            other => panic!("expected Partial, got {other:?}"),
        }
        let applied = session.install_pointer(&name, &destination).unwrap();
        assert_eq!(applied.effects.len(), 2);
        drop(session);

        let via_pointer = store.read_view(&FamilyLocation::new(&destination)).unwrap();
        let FamilyObservation::Family {
            source,
            root: observed_root,
            ..
        } = via_pointer
        else {
            panic!("pointer resolves to the family");
        };
        assert_eq!(source, FamilySource::Pointer);
        assert_eq!(observed_root, PathBuf::from("/mem/root"));

        let mut session = store.try_lock(&FamilyLocation::new(&destination)).unwrap();
        assert_eq!(
            session.root(),
            Path::new("/mem/root"),
            "a clone locks its root"
        );
        let removed = session.remove_pointer(&name).unwrap();
        assert_eq!(removed.effects.len(), 2);
        let again = session.remove_pointer(&name).unwrap();
        assert!(again.effects.is_empty(), "pointer removal is repeatable");
    }
}
