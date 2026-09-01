use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::store::CheckedV1Store;
use crate::checked_artifact::{CheckedArtifactFault, fail_next_checked_artifact_at};
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::tests::TempDir;

/// Row `:273`'s two managed parents, and the leaf it orders after them.
const MERGE_STORE: &str = ".gwz/merge";
const PRESERVATION_BUNDLES: &str = ".gwz/stash/bundles";
const RECORD_LEAF: &str = ".gwz/merge/merge_1.yaml";

/// R2-E Step E4.2, frozen ordering clause one: **both parents durable before
/// record**, through the production creation door. The bootstrap installs BOTH
/// declared prefixes and no record; it installs them under the WORKSPACE root,
/// not the Git directory (the §11.3-item-2(b) answer, recorded at `entry.rs`);
/// and the record that follows is durable when `create_open` returns — proved
/// by an independent re-read, not by the returned handle.
#[test]
fn the_creation_lease_bootstraps_both_managed_parents_before_any_record_exists() {
    let root = TempDir::new_git("merge-v1-e42-bootstrap");
    let model = record();
    let git = git2::Repository::open(&root.path)
        .unwrap()
        .path()
        .to_owned();

    let lease =
        V1MutationLease::acquire_for_merge_start_for_test(&root.path, &model.workspace_id).unwrap();

    assert!(root.path.join(MERGE_STORE).is_dir(), "MergeStore missing");
    let bundles = root.path.join(PRESERVATION_BUNDLES);
    assert!(bundles.is_dir(), "PreservationBundles missing");
    assert!(
        !root.path.join(RECORD_LEAF).exists(),
        "the record was written before its parents were proved"
    );
    assert!(
        !git.join(".gwz").exists(),
        "a managed parent was bound to the Git directory, not the workspace root"
    );

    let created = CheckedV1Store::default()
        .create_open(&lease, &root.path, &model)
        .unwrap();

    let durable = std::fs::read(root.path.join(RECORD_LEAF)).unwrap();
    assert_eq!(
        StoredV1Record::from_open_bytes(&root.path, created.location().path(), &durable)
            .unwrap()
            .source_digest(),
        created.source_digest(),
        "create_open returned before its record was durably readable"
    );
}

/// The decisive proof that the creation path is CONVERTED. Before E4.2 this
/// call created `.gwz/merge` itself, through `create_temporary`'s
/// `create_dir_all`; it cannot now, the publication being a checked artifact
/// action and a checked replacement with no open parent refusing. This is O13's
/// behavioural half; the checker's raw-writer inventory is the structural one.
#[test]
fn the_converted_creation_path_refuses_a_record_whose_parent_was_never_bootstrapped() {
    let root = TempDir::new_git("merge-v1-e42-unbootstrapped");
    let model = record();

    // The PLAIN lease — abort's, capability-free — bootstraps nothing, which is
    // also this row's proof that [P3-C1]'s hazard stays closed.
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    assert!(!root.path.join(MERGE_STORE).exists());

    let Err(refused) = CheckedV1Store::default().create_open(&lease, &root.path, &model) else {
        panic!("a record was published into an unbootstrapped parent");
    };

    assert!(
        refused.message.contains("parent"),
        "the refusal does not name the missing parent: {refused:?}"
    );
    assert!(
        !root.path.join(MERGE_STORE).exists(),
        "the creation path still creates its own managed parent"
    );
}

/// Frozen ordering clause two under interruption: **record durable before
/// Git**. Faulted immediately before the leaf publication, the operation leaves
/// both parents durably installed and NO record, so no Git work can have been
/// ordered behind a record that never came into being. (The forward half is
/// structural: `start.rs` creates the record inside a scoped lease and only then
/// builds its `ForwardRuntime`, the mutator lock not being re-entrant.)
#[test]
fn a_faulted_record_publication_leaves_the_parents_installed_and_no_record() {
    let root = TempDir::new_git("merge-v1-e42-publication-fault");
    let model = record();
    let lease =
        V1MutationLease::acquire_for_merge_start_for_test(&root.path, &model.workspace_id).unwrap();

    fail_next_checked_artifact_at(CheckedArtifactFault::BeforeManagedPublication);
    let Err(refused) = CheckedV1Store::default().create_open(&lease, &root.path, &model) else {
        panic!("the injected publication fault did not refuse");
    };

    assert!(
        refused.message.contains("injected failure"),
        "the fault was not the one this row installed: {refused:?}"
    );
    assert!(root.path.join(MERGE_STORE).is_dir());
    assert!(root.path.join(PRESERVATION_BUNDLES).is_dir());
    assert!(
        !root.path.join(RECORD_LEAF).exists(),
        "a record survived a failed publication"
    );
}

#[test]
fn checked_record_and_lease_are_bound_to_exact_root_and_bytes() {
    let first = TempDir::new_git("merge-v1-checked-first");
    let second = TempDir::new_git("merge-v1-checked-second");
    let checked = StoredV1Record::for_test(&first.path, record()).unwrap();
    let same = StoredV1Record::for_test(&first.path, record()).unwrap();
    let mut changed_record = record();
    changed_record.writer_version = "different".into();
    let changed = StoredV1Record::for_test(&first.path, changed_record).unwrap();

    assert_eq!(checked.source_digest(), same.source_digest());
    assert_ne!(checked.source_digest(), changed.source_digest());
    assert!(checked.raw().is_mapping());
    assert!(checked.unknown_fields().entries().is_empty());

    let lease = V1MutationLease::acquire_for_test(&first.path).unwrap();
    assert!(lease.covers(checked.location()));
    assert!(
        !lease.covers(
            StoredV1Record::for_test(&second.path, record())
                .unwrap()
                .location()
        )
    );
}

/// R2-E E4.1 precondition 6, FIRST arm (E0.2b §5.3 item 6): the activation
/// refusal is proved to occur BEFORE the operation's first durable mutation,
/// rather than asserted.
///
/// The FORWARD prologue takes the catalog before any record is written, so a
/// workspace whose catalog cannot be activated refuses with the merge store
/// untouched and the obstruction unread and unmoved. (The reverse prologue —
/// abort — takes the plain `acquire`, and is capability-free by the E4.1
/// review's [P1-1] cure.) Driven by a real un-bootstrappable catalog target — a foreign FILE where the catalog's own
/// directory belongs — because the catalog's injection keys are
/// `pub(in crate::checked_artifact)` and this partition is outside that tree;
/// the injection-driven arm (an interrupted durable edge converging on restart)
/// lives inside it, at `catalog::bootstrap::tests`.
#[test]
fn the_v1_prologue_refuses_an_unactivatable_catalog_before_any_durable_mutation() {
    let workspace = TempDir::new_git("merge-v1-catalog-refusal");
    let private = workspace.path.join(".gwz");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("catalog-final"), b"foreign").unwrap();

    let Err(refused) = V1MutationLease::acquire_activated_for_test(&workspace.path) else {
        panic!("the prologue activated a catalog over a foreign object");
    };
    // The capability-free lease the abort route takes is unaffected by the same
    // obstruction — the exit every refusal above depends on.
    assert!(V1MutationLease::acquire_for_test(&workspace.path).is_ok());

    assert!(
        refused.to_string().contains("catalog"),
        "the refusal does not name the catalog: {refused}"
    );
    assert!(
        !private.join("merge").exists(),
        "the operation wrote merge-store state before its catalog was proved"
    );
    assert_eq!(
        std::fs::read(private.join("catalog-final")).unwrap(),
        b"foreign",
        "the refused activation adopted or rewrote the foreign object"
    );
}
