use super::super::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::tests::TempDir;

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
