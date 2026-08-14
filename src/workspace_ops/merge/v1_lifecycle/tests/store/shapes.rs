use std::fs;

use super::{prepare_outcome, seed_open};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::merge::v1_lifecycle::checked::V1MutationLease;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::workspace_ops::merge::{OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

#[test]
fn same_byte_source_replacement_retains_identical_durable_authority() {
    let root = TempDir::new_git("merge-v1-store-same-byte-replacement");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action =
        Some(super::super::fixtures::up_to_date_action());
    let bytes = seed_open(&root, &model, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let rewrite = prepare_outcome(&lease, &current);
    let replacement = current.location().path().with_extension("replacement");
    fs::write(&replacement, &bytes).unwrap();
    fs::remove_file(current.location().path()).unwrap();
    fs::rename(&replacement, current.location().path()).unwrap();

    let next = store.commit(&lease, &current, rewrite).unwrap();
    assert_eq!(
        next.record().participants["mem_a"].state,
        ParticipantState::UpToDate
    );
}

#[test]
fn nonregular_open_source_and_archive_destination_fail_closed() {
    let root = TempDir::new_git("merge-v1-store-nonregular-open");
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action =
        Some(super::super::fixtures::up_to_date_action());
    seed_open(&root, &model, |_| {});
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, "merge_1").unwrap();
    let rewrite = prepare_outcome(&lease, &current);
    fs::remove_file(current.location().path()).unwrap();
    fs::create_dir(current.location().path()).unwrap();
    assert_eq!(
        store.commit(&lease, &current, rewrite).err().unwrap().code,
        ErrorCode::MergeRecordUnreadable
    );
    assert!(current.location().path().is_dir());

    let terminal_root = TempDir::new_git("merge-v1-store-nonregular-archive");
    let mut terminal = record();
    terminal.state = OperationState::Aborted;
    terminal.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let source_bytes = seed_open(&terminal_root, &terminal, |_| {});
    let lease = V1MutationLease::acquire_for_test(&terminal_root.path).unwrap();
    let current = store.load_open(&terminal_root.path, "merge_1").unwrap();
    let destination = current
        .location()
        .path()
        .parent()
        .unwrap()
        .join("done/merge_1.yaml");
    fs::create_dir_all(&destination).unwrap();
    assert_eq!(
        store.archive(&lease, &current).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );
    assert_eq!(fs::read(current.location().path()).unwrap(), source_bytes);
    assert!(destination.is_dir());
}
