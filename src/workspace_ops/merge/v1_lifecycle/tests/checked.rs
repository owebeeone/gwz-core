use super::super::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::tests::TempDir;

#[test]
fn checked_record_and_lease_are_bound_to_exact_root_and_bytes() {
    let first = TempDir::new("merge-v1-checked-first");
    let second = TempDir::new("merge-v1-checked-second");
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
