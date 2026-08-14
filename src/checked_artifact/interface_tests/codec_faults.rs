use std::io::Cursor;

use super::super::fault_v1::CheckedArtifactFaultKeyV1;
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, CatalogOccupancyV1,
    CleanupAliasSetV1, ProtocolRecordKindV1, RequestOwnerBindingV1, ScratchBytesV1,
    classify_expected_prefix, read_bounded_bytes, read_bounded_record,
};

#[test]
fn codec_limit_table_is_literal_and_complete() {
    let table = ProtocolRecordKindV1::ALL
        .iter()
        .map(|kind| (kind.stable_name(), kind.max_bytes()))
        .collect::<Vec<_>>();
    assert_eq!(
        table,
        vec![
            ("authority", 16 * 1024),
            ("capacity", 16 * 1024),
            ("admission", 16 * 1024),
            ("barrier_intent", 16 * 1024),
            ("bootstrap_intent", 16 * 1024),
            ("catalog_bootstrap", 16 * 1024),
            ("infrastructure", 8 * 1024),
            ("marker", 4 * 1024),
            ("cleanup_worklist", 16 * 1024),
            ("durable_path", 4 * 1024),
        ]
    );
}

#[test]
fn bounded_reader_rejects_limit_plus_one() {
    assert_eq!(
        read_bounded_bytes(Cursor::new(vec![0; 8]), 8)
            .unwrap()
            .len(),
        8
    );
    assert!(read_bounded_bytes(Cursor::new(vec![0; 9]), 8).is_err());
}

#[test]
fn typed_record_reader_enforces_record_limit_before_decode() {
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([1; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(0, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    );
    let bytes = reservation.encode_canonical().unwrap();
    assert_eq!(
        read_bounded_record::<ActionCapacityReservationV1>(Cursor::new(bytes)).unwrap(),
        reservation
    );
    assert!(
        read_bounded_record::<ActionCapacityReservationV1>(Cursor::new(vec![0; 16 * 1024 + 1]))
            .is_err()
    );
}

#[test]
fn partial_scratch_must_be_an_expected_prefix() {
    assert_eq!(
        classify_expected_prefix(b"abc", b"abcdef"),
        ScratchBytesV1::PartialExpectedPrefix
    );
    assert_eq!(
        classify_expected_prefix(b"abcdef", b"abcdef"),
        ScratchBytesV1::Exact
    );
    assert_eq!(
        classify_expected_prefix(b"abX", b"abcdef"),
        ScratchBytesV1::Other
    );
    assert_eq!(
        classify_expected_prefix(b"abcdefg", b"abcdef"),
        ScratchBytesV1::Other
    );
}

#[test]
fn retired_capacity_is_reserved_before_admission() {
    assert!(
        CatalogOccupancyV1::new(0, 63, false)
            .unwrap()
            .can_admit_new()
    );
    assert!(
        !CatalogOccupancyV1::new(0, 64, false)
            .unwrap()
            .can_admit_new()
    );
    assert!(
        !CatalogOccupancyV1::new(1, 63, false)
            .unwrap()
            .can_admit_new()
    );
    assert!(CatalogOccupancyV1::new(1, 63, false).unwrap().can_resume());
    assert!(CatalogOccupancyV1::new(0, 63, true).unwrap().can_resume());
}

#[test]
fn fault_vocabulary_is_platform_neutral_unique_and_iterable() {
    let keys = CheckedArtifactFaultKeyV1::all();
    assert!(keys.len() > 100);
    let mut names = keys
        .iter()
        .map(CheckedArtifactFaultKeyV1::stable_key)
        .collect::<Vec<_>>();
    let original = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), original);
    assert!(names.contains(&"barrier.intent_publish".to_owned()));
    assert!(names.contains(&"barrier.anchor_outbound".to_owned()));
    assert!(names.contains(&"managed_bootstrap.successor_publish".to_owned()));
    assert!(names.contains(&"terminal.action_directory_retire".to_owned()));
}
