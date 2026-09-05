//! The real store's [`StoreFixture`]: temporary directories, real files.
//!
//! `gwz_family_store_contract::contract_tests::run_all` is measured against
//! [`YamlFamilyStore`](crate::YamlFamilyStore) through this fixture, so the
//! pointer/marker half of the suite runs on genuine filesystem effects
//! rather than on the reference fake's maps (LCM1.0c checkpoint §8).
//!
//! Two fixture obligations need explaining.
//!
//! - `fresh_root` hands out `<temp>/fam-N/root`, so the rows the suite
//!   records at `../<name>` land in a fixture-private parent, never in the
//!   shared temporary directory.
//! - `fail_next` makes the target path genuinely unwritable rather than
//!   teaching the store a fault-injection hook. The suite then *retries* the
//!   same call and requires it to succeed, which no durable obstruction can
//!   satisfy, so the obstruction is bracketed around exactly the sabotaged
//!   call by [`BracketedStore`], a delegating wrapper that owns no store
//!   behaviour of its own: it plants a directory at the pointer's path,
//!   calls the real store (whose `rename` onto a directory fails with a real
//!   OS error), and removes the obstruction. The store under test is
//!   unmodified and the failure it reports is a real one.

// Ordinary file I/O in this crate's own tests, outside gwz-core's
// merge-writer boundary (gwz-core/clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gwz_family_model::{
    AllocationId, FamilyChange, FamilyId, FamilyView, INDEX_RELATIVE_PATH, LOCK_RELATIVE_PATH,
    MAX_ENCODED_INDEX_BYTES, MemberName, POINTER_RELATIVE_PATH,
};
use gwz_family_store_contract::contract_tests::StoreFixture;
use gwz_family_store_contract::{
    AppliedChange, FamilyLocation, FamilyObservation, FamilySession, FamilyStore, StoreError,
    StoreOperation,
};

use crate::{LockedFamilySession, YamlFamilyStore};

/// Paths whose next operation the fixture obstructs, keyed by the family
/// root the suite named.
#[derive(Debug, Default)]
struct Armed {
    entries: Vec<(PathBuf, StoreOperation)>,
    roots: Vec<PathBuf>,
}

impl Armed {
    fn take(&mut self, root: &Path, operation: StoreOperation) -> bool {
        let root = canonical(root);
        let found = self
            .entries
            .iter()
            .position(|(armed, op)| *op == operation && *armed == root);
        match found {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// [`YamlFamilyStore`] plus the fixture's one-call obstruction bracket.
#[derive(Clone, Debug, Default)]
pub(crate) struct BracketedStore {
    inner: YamlFamilyStore,
    armed: Rc<RefCell<Armed>>,
}

impl FamilyStore for BracketedStore {
    type Session = BracketedSession;

    fn read_view(&self, location: &FamilyLocation) -> Result<FamilyObservation, StoreError> {
        self.inner.read_view(location)
    }

    fn try_lock(&self, location: &FamilyLocation) -> Result<Self::Session, StoreError> {
        Ok(BracketedSession {
            inner: self.inner.try_lock(location)?,
            armed: Rc::clone(&self.armed),
        })
    }
}

/// A held session that brackets the sabotaged call and delegates the rest.
#[derive(Debug)]
pub(crate) struct BracketedSession {
    inner: LockedFamilySession,
    armed: Rc<RefCell<Armed>>,
}

impl FamilySession for BracketedSession {
    fn root(&self) -> &Path {
        self.inner.root()
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        self.inner.reread()
    }

    fn found(
        &mut self,
        family_id: FamilyId,
        root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        self.inner.found(family_id, root_allocation)
    }

    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        self.inner.apply(change)
    }

    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        let sabotage = self
            .armed
            .borrow_mut()
            .take(self.inner.root(), StoreOperation::WritePointer);
        if !sabotage {
            return self.inner.install_pointer(name, destination);
        }
        // Occupy the pointer's own path with a directory: the marker write
        // that precedes it still succeeds, and publishing the pointer fails
        // on a real `rename` error. Removing it afterwards is what makes the
        // suite's retry a retry rather than a second sabotage.
        let obstruction = destination.join(POINTER_RELATIVE_PATH);
        fs::create_dir_all(obstruction.parent().expect("the pointer has a parent"))
            .expect("fixture: materialise the destination's metadata directory");
        fs::create_dir(&obstruction).expect("fixture: obstruct the pointer path");
        let outcome = self.inner.install_pointer(name, destination);
        fs::remove_dir(&obstruction).expect("fixture: release the obstruction");
        outcome
    }

    fn remove_pointer(&mut self, name: &MemberName) -> Result<AppliedChange, StoreError> {
        self.inner.remove_pointer(name)
    }
}

/// One temporary directory, one root per `fresh_root`, real files.
pub(crate) struct FsFixture {
    temp: tempfile::TempDir,
    store: BracketedStore,
    counter: u32,
}

impl FsFixture {
    pub(crate) fn new() -> Self {
        Self {
            temp: tempfile::tempdir().expect("fixture: temporary directory"),
            store: BracketedStore::default(),
            counter: 0,
        }
    }
}

impl StoreFixture for FsFixture {
    type Store = BracketedStore;

    fn fresh_root(&mut self) -> (Self::Store, FamilyLocation) {
        self.counter += 1;
        // The suite records rows at `../<name>`, so every root gets its own
        // parent directory: siblings of one family never collide with
        // another's.
        let root = self
            .temp
            .path()
            .join(format!("fam-{}", self.counter))
            .join("root");
        fs::create_dir_all(&root).expect("fixture: create the root workspace");
        self.store.armed.borrow_mut().roots.push(canonical(&root));
        (self.store.clone(), FamilyLocation::new(root))
    }

    fn member_workspace(&mut self, root: &Path, relative: &str) -> PathBuf {
        // The orchestrator allocates the destination before the store writes
        // into it (design §3 step 2): materialise the directory and hand back
        // the join, unresolved, so the store's own resolution decides that it
        // is the row's path.
        let workspace = root.join(relative);
        fs::create_dir_all(&workspace).expect("fixture: allocate the member workspace");
        workspace
    }

    fn lock_artifact_exists(&self, root: &Path) -> bool {
        root.join(LOCK_RELATIVE_PATH).exists()
    }

    fn corrupt_index(&mut self, root: &Path) {
        let path = root.join(INDEX_RELATIVE_PATH);
        let mut file = fs::File::create(&path).expect("fixture: open the index");
        file.write_all(b"schema: [gwz.local-family/v1\nfamily_id: \"unterminated\n")
            .expect("fixture: corrupt the index");
    }

    fn oversize_index(&mut self, root: &Path) {
        let path = root.join(INDEX_RELATIVE_PATH);
        let mut encoded = fs::read(&path).expect("fixture: read the index");
        encoded.push(b'\n');
        encoded.extend(std::iter::repeat_n(
            b'#',
            usize::try_from(MAX_ENCODED_INDEX_BYTES).expect("the limit fits a usize") + 1,
        ));
        let mut file = fs::File::create(&path).expect("fixture: open the index");
        file.write_all(&encoded).expect("fixture: pad the index");
    }

    fn fail_next(&mut self, root: &Path, operation: StoreOperation) {
        let root = canonical(root);
        let mut armed = self.store.armed.borrow_mut();
        assert!(
            armed.roots.contains(&root),
            "fail_next: {} is not a root this fixture handed out",
            root.display()
        );
        assert_eq!(
            operation,
            StoreOperation::WritePointer,
            "the fixture obstructs the pointer path; no other operation is scripted"
        );
        armed.entries.push((root, operation));
    }
}

/// The conformance suite, measured on real files.
#[test]
fn the_real_store_satisfies_the_conformance_suite() {
    gwz_family_store_contract::contract_tests::run_all(&mut FsFixture::new());
}

/// The fixture's own obligations, so a failure in the suite is the store's.
#[test]
fn the_fixture_hands_out_private_parents_and_materialises_workspaces() {
    let mut fixture = FsFixture::new();
    let (_, first) = fixture.fresh_root();
    let (_, second) = fixture.fresh_root();
    assert_ne!(
        first.workspace.parent(),
        second.workspace.parent(),
        "each root gets a fixture-private parent"
    );
    assert!(first.workspace.is_dir(), "the root workspace exists");
    let member = fixture.member_workspace(&first.workspace, "../ws-A");
    assert_eq!(
        member,
        first.workspace.join("../ws-A"),
        "the join is returned"
    );
    assert!(
        member.is_dir(),
        "the destination is allocated before the store writes"
    );
    assert!(
        !fixture.lock_artifact_exists(&first.workspace),
        "nothing has taken the lock yet"
    );
}
