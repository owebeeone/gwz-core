use super::super::capability::{
    AsciiComponent, CanonicalComponent, DurableObjectIdentityV1, PathComponentMode,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, AdmissionHandoffDecisionV1, BarrierIntentV1,
    BarrierOrdinalV1, CanonicalPathIdentityV1, CleanupAliasSetV1, CleanupAliasV1,
    CleanupPhysicalFactV1, CleanupResolutionV1, CleanupRowV1, CleanupWorklistV1,
    DurableLeafFingerprintV1, ManagedBootstrapInputV1, RecordObservationV1, RequestOwnerBindingV1,
    ScratchRecordObservationV1, classify_cleanup_row, classify_fixed_replacement, classify_handoff,
};

fn linux_identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

fn schedule(plan_sizes: &[u8], barriers: usize) -> ActionScheduleV1 {
    let plans = plan_sizes
        .iter()
        .enumerate()
        .map(|(index, size)| {
            ManagedBootstrapInputV1::new([index as u8; 32], usize::from(*size)).unwrap()
        })
        .collect();
    ActionScheduleV1::try_new(barriers, plans, CleanupAliasSetV1::all()).unwrap()
}

#[test]
fn bootstrap_schedule_pins_both_extremes() {
    let deep = schedule(&[8], 64);
    assert_eq!(deep.generation_count(), 17);
    assert_eq!(deep.bootstrap_rows()[0].generation_range(), 0..17);
    assert_eq!(deep.component_count(), 8);

    let wide = schedule(&[1, 1, 1, 1, 1, 1, 1, 1], 64);
    assert_eq!(wide.generation_count(), 24);
    assert_eq!(wide.component_count(), 8);
    for (index, row) in wide.bootstrap_rows().iter().enumerate() {
        assert_eq!(row.generation_range(), (index * 3)..(index * 3 + 3));
        assert_eq!(row.component_range(), index..index + 1);
    }
}

#[test]
fn schedule_rejects_unbounded_or_empty_inputs() {
    assert!(ActionScheduleV1::try_new(65, Vec::new(), CleanupAliasSetV1::all()).is_err());
    assert!(ManagedBootstrapInputV1::new([0; 32], 0).is_err());
    assert!(ManagedBootstrapInputV1::new([0; 32], 9).is_err());
    assert!(
        ActionScheduleV1::try_new(
            0,
            vec![
                ManagedBootstrapInputV1::new([3; 32], 1).unwrap(),
                ManagedBootstrapInputV1::new([3; 32], 1).unwrap(),
            ],
            CleanupAliasSetV1::all(),
        )
        .is_err()
    );
    assert!(
        ActionScheduleV1::try_new(
            0,
            vec![
                ManagedBootstrapInputV1::new([0; 32], 5).unwrap(),
                ManagedBootstrapInputV1::new([1; 32], 4).unwrap(),
            ],
            CleanupAliasSetV1::all(),
        )
        .is_err()
    );
}

#[test]
fn reservation_and_admission_are_canonical_and_bound() {
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([1; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        schedule(&[2, 1], 7),
    );
    let bytes = reservation.encode_canonical().unwrap();
    assert_eq!(
        ActionCapacityReservationV1::decode_canonical(&bytes).unwrap(),
        reservation
    );
    let preparing = ActionDirectoryAdmissionV1::preparing(&reservation);
    let bytes = preparing.encode_canonical().unwrap();
    assert_eq!(
        ActionDirectoryAdmissionV1::decode_canonical(&bytes).unwrap(),
        preparing
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert!(ActionDirectoryAdmissionV1::decode_canonical(&trailing).is_err());
}

#[test]
fn fixed_replacement_table_is_closed() {
    let old = ActionDirectoryAdmissionV1::idle();
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([3; 32]),
        RequestOwnerBindingV1::new([4; 32]),
        schedule(&[1], 0),
    );
    let new = ActionDirectoryAdmissionV1::preparing(&reservation);
    use super::super::protocol::FixedReplacementDecisionV1::*;
    assert_eq!(
        classify_fixed_replacement(
            &RecordObservationV1::Exact(old.clone()),
            &ScratchRecordObservationV1::Missing,
            &old,
            &new
        ),
        WriteOrRewriteScratch
    );
    assert_eq!(
        classify_fixed_replacement(
            &RecordObservationV1::Exact(old.clone()),
            &ScratchRecordObservationV1::PartialExpectedPrefix,
            &old,
            &new
        ),
        WriteOrRewriteScratch
    );
    assert_eq!(
        classify_fixed_replacement(
            &RecordObservationV1::Exact(old.clone()),
            &ScratchRecordObservationV1::Exact(new.clone()),
            &old,
            &new
        ),
        ReplaceActiveFromScratch
    );
    assert_eq!(
        classify_fixed_replacement(
            &RecordObservationV1::Exact(new.clone()),
            &ScratchRecordObservationV1::Missing,
            &old,
            &new
        ),
        Complete
    );
    assert_eq!(
        classify_fixed_replacement(
            &RecordObservationV1::Exact(new.clone()),
            &ScratchRecordObservationV1::Other,
            &old,
            &new
        ),
        Ambiguous
    );
}

#[test]
fn admission_directory_handoff_has_one_owner_at_each_step() {
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([5; 32]),
        RequestOwnerBindingV1::new([6; 32]),
        schedule(&[1], 1),
    );
    let preparing = ActionDirectoryAdmissionV1::preparing(&reservation);
    let missing = ActionDirectoryObservationV1::Missing;
    let exact = ActionDirectoryObservationV1::exact(
        linux_identity(9),
        RecordObservationV1::Exact(reservation.clone()),
    );
    let partial = ActionDirectoryObservationV1::exact(
        linux_identity(9),
        RecordObservationV1::PartialExpectedPrefix,
    );
    assert_eq!(
        classify_handoff(&preparing, &reservation, &missing, &missing),
        AdmissionHandoffDecisionV1::CreateStaging
    );
    assert_eq!(
        classify_handoff(&preparing, &reservation, &partial, &missing),
        AdmissionHandoffDecisionV1::WriteOrRewriteReservation
    );
    assert_eq!(
        classify_handoff(&preparing, &reservation, &exact, &missing),
        AdmissionHandoffDecisionV1::PublishStaging
    );
    assert_eq!(
        classify_handoff(&preparing, &reservation, &missing, &exact),
        AdmissionHandoffDecisionV1::ReplacePreparingWithIdle
    );
    assert_eq!(
        classify_handoff(&preparing, &reservation, &exact, &exact),
        AdmissionHandoffDecisionV1::Ambiguous
    );
}

#[test]
fn cleanup_worklist_is_immutable_ordered_and_physical() {
    let fingerprint = |byte| DurableLeafFingerprintV1::new(linux_identity(byte), 10, [byte; 32]);
    let rows = vec![
        CleanupRowV1::new(CleanupAliasV1::Source, fingerprint(1)),
        CleanupRowV1::new(CleanupAliasV1::Goal, fingerprint(2)),
        CleanupRowV1::new(CleanupAliasV1::Authority, fingerprint(3)),
    ];
    let reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([7; 32]),
        RequestOwnerBindingV1::new([8; 32]),
        schedule(&[1], 0),
    );
    let worklist = CleanupWorklistV1::try_new(&reservation, rows).unwrap();
    assert_eq!(worklist.rows().len(), 3);
    assert!(worklist.matches_reservation(&reservation));
    let encoded = worklist.encode_canonical().unwrap();
    assert_eq!(
        CleanupWorklistV1::decode_canonical(&encoded).unwrap(),
        worklist
    );
    let row = &worklist.rows()[0];
    assert_eq!(
        classify_cleanup_row(
            row,
            &CleanupPhysicalFactV1::Exact(row.expected().clone()),
            &CleanupPhysicalFactV1::Missing
        ),
        CleanupResolutionV1::Retire
    );
    assert_eq!(
        classify_cleanup_row(
            row,
            &CleanupPhysicalFactV1::Missing,
            &CleanupPhysicalFactV1::Exact(row.expected().clone())
        ),
        CleanupResolutionV1::Complete
    );
    assert_eq!(
        classify_cleanup_row(
            row,
            &CleanupPhysicalFactV1::Other,
            &CleanupPhysicalFactV1::Missing
        ),
        CleanupResolutionV1::Ambiguous
    );
}

#[test]
fn barrier_intent_id_binds_every_persisted_field() {
    let make = |ordinal,
                action,
                owner,
                barrier_count,
                catalog,
                home,
                home_name,
                target,
                path_leaf,
                target_leaf| {
        let reservation = ActionCapacityReservationV1::new(
            ActionDigestV1::new([action; 32]),
            RequestOwnerBindingV1::new([owner; 32]),
            schedule(&[1], barrier_count),
        );
        BarrierIntentV1::try_new(
            &reservation,
            BarrierOrdinalV1::new(ordinal).unwrap(),
            linux_identity(catalog),
            linux_identity(home),
            AsciiComponent::parse(&[home_name]).unwrap(),
            linux_identity(target),
            CanonicalPathIdentityV1::new(vec![
                CanonicalComponent::new(
                    AsciiComponent::parse(b"a").unwrap(),
                    PathComponentMode::Sensitive,
                ),
                CanonicalComponent::new(
                    AsciiComponent::parse(&[path_leaf]).unwrap(),
                    PathComponentMode::Sensitive,
                ),
            ])
            .unwrap(),
            AsciiComponent::parse(&[target_leaf]).unwrap(),
        )
        .unwrap()
    };
    let first = make(0, 8, 9, 2, 1, 2, b'h', 3, b'b', b't');
    for changed in [
        make(1, 8, 9, 2, 1, 2, b'h', 3, b'b', b't'),
        make(0, 7, 9, 2, 1, 2, b'h', 3, b'b', b't'),
        make(0, 8, 7, 2, 1, 2, b'h', 3, b'b', b't'),
        make(0, 8, 9, 1, 1, 2, b'h', 3, b'b', b't'),
        make(0, 8, 9, 2, 4, 2, b'h', 3, b'b', b't'),
        make(0, 8, 9, 2, 1, 4, b'h', 3, b'b', b't'),
        make(0, 8, 9, 2, 1, 2, b'i', 3, b'b', b't'),
        make(0, 8, 9, 2, 1, 2, b'h', 4, b'b', b't'),
        make(0, 8, 9, 2, 1, 2, b'h', 3, b'c', b't'),
        make(0, 8, 9, 2, 1, 2, b'h', 3, b'b', b'u'),
    ] {
        assert_ne!(first.intent_id(), changed.intent_id());
    }
    let bytes = first.encode_canonical().unwrap();
    assert_eq!(BarrierIntentV1::decode_canonical(&bytes).unwrap(), first);

    let too_short = ActionCapacityReservationV1::new(
        ActionDigestV1::new([8; 32]),
        RequestOwnerBindingV1::new([9; 32]),
        schedule(&[1], 1),
    );
    assert!(
        BarrierIntentV1::try_new(
            &too_short,
            BarrierOrdinalV1::new(1).unwrap(),
            linux_identity(1),
            linux_identity(2),
            AsciiComponent::parse(b"h").unwrap(),
            linux_identity(3),
            CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
                AsciiComponent::parse(b"p").unwrap(),
                PathComponentMode::Sensitive,
            )])
            .unwrap(),
            AsciiComponent::parse(b"t").unwrap(),
        )
        .is_err()
    );
}
