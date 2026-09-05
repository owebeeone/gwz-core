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
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gwz_family_model::{
    AllocationId, FamilyChange, FamilyId, FamilyView, MAX_ENCODED_INDEX_BYTES, MemberName,
    MemberState, validate_transition,
};

use crate::{
    AppliedChange, FamilyLocation, FamilyObservation, FamilySession, FamilySource, FamilyStore,
    MetadataEffect, StoreError, StoreOperation,
};

/// What the suite needs from an implementation's fixture: fresh locations
/// and the ability to observe or corrupt the on-disk (or in-memory) state.
pub trait StoreFixture {
    type Store: FamilyStore;

    /// A store plus a fresh root location holding no family.
    fn fresh_root(&mut self) -> (Self::Store, FamilyLocation);
    /// Whether the family lock artifact exists at `root`.
    fn lock_artifact_exists(&self, root: &Path) -> bool;
    /// Make the index at `root` undecodable.
    fn corrupt_index(&mut self, root: &Path);
    /// Make the index at `root` larger than the encoded limit.
    fn oversize_index(&mut self, root: &Path);
}

/// Run every conformance case through `fixture`.
pub fn run_all<F: StoreFixture>(fixture: &mut F) {
    reading_no_family_creates_no_lock_file(fixture);
    founding_then_reading_round_trips_and_reads_create_no_lock(fixture);
    a_second_try_lock_is_busy_until_the_session_drops(fixture);
    apply_refuses_an_invalid_transition_without_effects(fixture);
    apply_writes_only_the_matching_index_change(fixture);
    malformed_and_oversize_indexes_refuse(fixture);
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

/// In-memory family state shared by a store and its sessions.
#[derive(Debug, Default)]
struct Families {
    /// Root workspace -> its index, when founded.
    indexes: BTreeMap<PathBuf, IndexState>,
    /// Clone workspace -> (family id, root workspace).
    pointers: BTreeMap<PathBuf, (FamilyId, PathBuf)>,
    /// Clone workspace -> allocation id.
    markers: BTreeMap<PathBuf, AllocationId>,
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
        self.families
            .borrow()
            .lock_files
            .iter()
            .any(|path| path == root)
    }

    pub fn corrupt_index(&self, root: &Path) {
        self.families
            .borrow_mut()
            .indexes
            .insert(root.to_path_buf(), IndexState::Malformed);
    }

    pub fn oversize_index(&self, root: &Path) {
        self.families.borrow_mut().indexes.insert(
            root.to_path_buf(),
            IndexState::Oversize(MAX_ENCODED_INDEX_BYTES + 1),
        );
    }

    /// Clone pointers currently written (workspace -> root).
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
        let families = self.families.borrow();
        let has_index = families.indexes.contains_key(workspace);
        let pointer = families.pointers.get(workspace).cloned();
        match (has_index, pointer) {
            (true, Some(_)) => Err(StoreError::ConflictingMetadata {
                workspace: workspace.to_path_buf(),
            }),
            (true, None) => Ok(Some((workspace.to_path_buf(), FamilySource::Index))),
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
        match families.indexes.get(root) {
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
            None => location.workspace.clone(),
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
        {
            let families = self.store.families.borrow();
            if families.indexes.contains_key(destination) {
                return Err(StoreError::ConflictingMetadata {
                    workspace: destination.to_path_buf(),
                });
            }
            if let Some((family_id, _)) = families.pointers.get(destination)
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
            .insert(destination.to_path_buf(), row.allocation_id.clone());
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
        self.store.families.borrow_mut().pointers.insert(
            destination.to_path_buf(),
            (view.family_id.clone(), self.root.clone()),
        );
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
        let workspace = self.root.join(&row.path);
        let mut effects = Vec::new();
        let mut families = self.store.families.borrow_mut();
        let matches = families
            .pointers
            .get(&workspace)
            .is_some_and(|(family_id, _)| *family_id == view.family_id);
        if matches {
            families.pointers.remove(&workspace);
            effects.push(MetadataEffect::PointerRemoved {
                workspace: workspace.clone(),
            });
        }
        if families.markers.remove(&workspace).is_some() {
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
    stores: Vec<InMemoryFamilyStore>,
    counter: u32,
}

impl StoreFixture for InMemoryFixture {
    type Store = InMemoryFamilyStore;

    fn fresh_root(&mut self) -> (InMemoryFamilyStore, FamilyLocation) {
        self.counter += 1;
        let store = InMemoryFamilyStore::new();
        self.stores.push(store.clone());
        (
            store,
            FamilyLocation::new(PathBuf::from(format!("/mem/root-{}", self.counter))),
        )
    }

    fn lock_artifact_exists(&self, root: &Path) -> bool {
        self.stores.iter().any(|store| store.lock_file_exists(root))
    }

    fn corrupt_index(&mut self, root: &Path) {
        for store in &self.stores {
            store.corrupt_index(root);
        }
    }

    fn oversize_index(&mut self, root: &Path) {
        for store in &self.stores {
            store.oversize_index(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_satisfies_the_conformance_suite() {
        run_all(&mut InMemoryFixture::default());
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
