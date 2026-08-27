use super::super::capability::{
    AsciiComponent, CanonicalComponent, DurableObjectIdentityV1, PathComponentMode,
    RoamingAnchorHomeWitnessV1,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, AdmissionHandoffDecisionV1, BarrierIntentV1,
    BarrierOrdinalV1, CanonicalPathIdentityV1, CleanupAliasSetV1, CleanupAliasV1,
    CleanupPhysicalFactV1, CleanupResolutionV1, CleanupRowV1, CleanupWorklistV1,
    DurableLeafFingerprintV1, ManagedBootstrapInputV1, RecordObservationV1, RequestOwnerBindingV1,
    ScratchRecordObservationV1, classify_fixed_replacement, classify_handoff,
    read_and_bind_barrier_intent, read_and_bind_cleanup_worklist, read_bounded_record,
};

fn linux_identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

/// R2-E Phase E2 (O6). The three roaming-anchor-home facts now reach the intent
/// only through the owner-minted witness, so the protocol-private semantic test
/// binds them through the `cfg(test)` door — the
/// `NamespaceBarrierAuthority::test_only` shape — and still varies each one
/// independently.
fn home_witness(catalog: u8, home: u8, home_name: u8) -> RoamingAnchorHomeWitnessV1 {
    RoamingAnchorHomeWitnessV1::test_only(
        linux_identity(catalog),
        linux_identity(home),
        AsciiComponent::parse(&[home_name]).unwrap(),
    )
}

/// One barrier intent, encoded, for the O6 read-side rows below. Its three
/// identity facts are `home_witness(1, 2, b'h')`.
fn encoded_barrier_intent() -> Vec<u8> {
    BarrierIntentV1::test_issue(
        &ActionCapacityReservationV1::new(
            ActionDigestV1::new([8; 32]),
            RequestOwnerBindingV1::new([9; 32]),
            schedule(&[1], 2),
        ),
        BarrierOrdinalV1::new(0).unwrap(),
        &home_witness(1, 2, b'h'),
        linux_identity(3),
        CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
            AsciiComponent::parse(b"p").unwrap(),
            PathComponentMode::Sensitive,
        )])
        .unwrap(),
        AsciiComponent::parse(b"t").unwrap(),
    )
    .unwrap()
    .encode_canonical()
    .unwrap()
}

fn barrier_reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([8; 32]),
        RequestOwnerBindingV1::new([9; 32]),
        schedule(&[1], 2),
    )
}

/// R2-E Phase E2 (O6, read side). The restatement class survived restart until
/// this refusal existed: `decode_canonical` rebuilds through `from_bound_fields`
/// and bypasses `issue`, so a resume read the resident record's caller-asserted
/// identities and acted on them however tight `issue` became.
///
/// Each of the three facts gets its **own** row (E2 review [P3-7]) rather than a
/// shared loop, so a regression in one cannot be masked by an earlier failure in
/// another, and so the failing test's name says which fact stopped being checked.
fn refuses_disagreeing_witness(disagreeing: RoamingAnchorHomeWitnessV1) {
    assert!(
        read_and_bind_barrier_intent(
            Cursor::new(encoded_barrier_intent()),
            &barrier_reservation(),
            BarrierOrdinalV1::new(0).unwrap(),
            &disagreeing,
        )
        .is_err()
    );
}

#[test]
fn a_resident_barrier_intent_is_refused_when_the_catalog_anchor_identity_disagrees() {
    refuses_disagreeing_witness(home_witness(4, 2, b'h'));
}

#[test]
fn a_resident_barrier_intent_is_refused_when_the_home_parent_identity_disagrees() {
    refuses_disagreeing_witness(home_witness(1, 4, b'h'));
}

#[test]
fn a_resident_barrier_intent_is_refused_when_the_home_name_disagrees() {
    refuses_disagreeing_witness(home_witness(1, 2, b'i'));
}

/// The same read binds when the owner's re-minted witness agrees, so the three
/// refusals above are proved to be about the *disagreement* and not about the
/// witness being required at all.
#[test]
fn a_resident_barrier_intent_binds_against_the_witness_the_owner_re_minted() {
    assert!(
        read_and_bind_barrier_intent(
            Cursor::new(encoded_barrier_intent()),
            &barrier_reservation(),
            BarrierOrdinalV1::new(0).unwrap(),
            &home_witness(1, 2, b'h'),
        )
        .is_ok()
    );
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
        read_bounded_record::<ActionCapacityReservationV1>(Cursor::new(bytes)).unwrap(),
        reservation
    );
    let preparing = ActionDirectoryAdmissionV1::preparing(&reservation);
    let bytes = preparing.encode_canonical().unwrap();
    assert_eq!(
        read_bounded_record::<ActionDirectoryAdmissionV1>(Cursor::new(bytes.clone())).unwrap(),
        preparing
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert!(read_bounded_record::<ActionDirectoryAdmissionV1>(Cursor::new(trailing)).is_err());
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
    let bound = read_and_bind_cleanup_worklist(Cursor::new(encoded.clone()), &reservation).unwrap();
    assert_eq!(bound.value(), &worklist);
    assert_eq!(bound.len(), 3);
    assert!(!bound.is_empty());
    let row = bound.row(0).unwrap();
    assert_eq!(row.alias(), CleanupAliasV1::Source);
    let expected = row.expected().clone();
    assert_eq!(
        bound.classify(
            0,
            &CleanupPhysicalFactV1::Exact(expected.clone()),
            &CleanupPhysicalFactV1::Missing
        ),
        Some(CleanupResolutionV1::Retire)
    );
    assert_eq!(
        bound.classify(
            0,
            &CleanupPhysicalFactV1::Missing,
            &CleanupPhysicalFactV1::Exact(expected)
        ),
        Some(CleanupResolutionV1::Complete)
    );
    assert_eq!(
        bound.classify(
            0,
            &CleanupPhysicalFactV1::Other,
            &CleanupPhysicalFactV1::Missing
        ),
        Some(CleanupResolutionV1::Ambiguous)
    );
    assert_eq!(
        bound.classify(
            0,
            &CleanupPhysicalFactV1::Exact(bound.row(1).unwrap().expected().clone()),
            &CleanupPhysicalFactV1::Missing,
        ),
        Some(CleanupResolutionV1::Ambiguous)
    );
    assert_eq!(
        bound.classify(
            3,
            &CleanupPhysicalFactV1::Missing,
            &CleanupPhysicalFactV1::Missing,
        ),
        None
    );
    let wrong_reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([9; 32]),
        RequestOwnerBindingV1::new([8; 32]),
        schedule(&[1], 0),
    );
    assert!(read_and_bind_cleanup_worklist(Cursor::new(encoded), &wrong_reservation).is_err());
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
        BarrierIntentV1::test_issue(
            &reservation,
            BarrierOrdinalV1::new(ordinal).unwrap(),
            &home_witness(catalog, home, home_name),
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
    let bound_reservation = || {
        ActionCapacityReservationV1::new(
            ActionDigestV1::new([8; 32]),
            RequestOwnerBindingV1::new([9; 32]),
            schedule(&[1], 2),
        )
    };
    assert_eq!(
        read_bounded_record::<BarrierIntentV1>(Cursor::new(bytes.clone())).unwrap(),
        first
    );
    assert_eq!(
        read_and_bind_barrier_intent(
            Cursor::new(bytes.clone()),
            &bound_reservation(),
            BarrierOrdinalV1::new(0).unwrap(),
            &home_witness(1, 2, b'h'),
        )
        .unwrap()
        .value(),
        &first
    );
    assert!(
        read_and_bind_barrier_intent(
            Cursor::new(bytes.clone()),
            &ActionCapacityReservationV1::new(
                ActionDigestV1::new([7; 32]),
                RequestOwnerBindingV1::new([9; 32]),
                schedule(&[1], 2),
            ),
            BarrierOrdinalV1::new(0).unwrap(),
            &home_witness(1, 2, b'h'),
        )
        .is_err()
    );
    assert!(
        read_and_bind_barrier_intent(
            Cursor::new(bytes.clone()),
            &bound_reservation(),
            BarrierOrdinalV1::new(1).unwrap(),
            &home_witness(1, 2, b'h'),
        )
        .is_err()
    );

    let too_short = ActionCapacityReservationV1::new(
        ActionDigestV1::new([8; 32]),
        RequestOwnerBindingV1::new([9; 32]),
        schedule(&[1], 1),
    );
    assert!(
        BarrierIntentV1::test_issue(
            &too_short,
            BarrierOrdinalV1::new(1).unwrap(),
            &home_witness(1, 2, b'h'),
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
use std::io::Cursor;
