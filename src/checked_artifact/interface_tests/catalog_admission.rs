use super::super::capability::DurableObjectIdentityV1;
use super::super::protocol::generated;
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1, ActionScheduleV1,
    ActionSlotV1, AdmissionHandoffDecisionV1, CatalogAdmissionOccupancyV1,
    CatalogAdmissionOwnerTestV1, CatalogNameClassificationV1, CatalogNameInvalidReasonV1,
    CatalogOccupancyErrorV1, CatalogOccupancyV1, CleanupAliasSetV1, FixedReplacementDecisionV1,
    RecordObservationV1, RequestOwnerBindingV1, RootEntryNameV1, ScratchRecordObservationV1,
    classify_fixed_replacement, read_bounded_record,
};

fn reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([0xabu8; 32]),
        RequestOwnerBindingV1::new([0xcdu8; 32]),
        ActionScheduleV1::try_new(1, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    )
}

fn linux_identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

#[test]
fn root_grammar_separates_valid_recognized_invalid_and_foreign_names() {
    let action = ActionDigestV1::new([0xabu8; 32]);
    let action_name = RootEntryNameV1::ActiveAction(action).name();
    assert_eq!(
        RootEntryNameV1::parse(action_name.as_bytes()),
        CatalogNameClassificationV1::Valid(RootEntryNameV1::ActiveAction(action))
    );
    assert_eq!(
        RootEntryNameV1::parse(b"catalog-format-v1"),
        CatalogNameClassificationV1::Valid(RootEntryNameV1::Infrastructure(
            super::super::protocol::InfrastructureSlotV1::CatalogFormat,
        ))
    );
    assert_eq!(
        RootEntryNameV1::parse(b"catalog-format-v2"),
        CatalogNameClassificationV1::RecognizedInvalid(
            CatalogNameInvalidReasonV1::UnsupportedVersion,
        )
    );
    assert_eq!(
        RootEntryNameV1::parse(b"action-abc-v1"),
        CatalogNameClassificationV1::RecognizedInvalid(
            CatalogNameInvalidReasonV1::InvalidActionDigestWidth,
        )
    );
    assert_eq!(
        RootEntryNameV1::parse(format!("action-{}-v1", "g".repeat(64)).as_bytes(),),
        CatalogNameClassificationV1::RecognizedInvalid(
            CatalogNameInvalidReasonV1::InvalidActionDigestEncoding,
        )
    );
    assert_eq!(
        RootEntryNameV1::parse(format!("action-{}-v1", "A".repeat(64)).as_bytes(),),
        CatalogNameClassificationV1::RecognizedInvalid(
            CatalogNameInvalidReasonV1::InvalidActionDigestEncoding,
        )
    );
    assert_eq!(
        RootEntryNameV1::parse(b"action-\xff-v1"),
        CatalogNameClassificationV1::RecognizedInvalid(CatalogNameInvalidReasonV1::NonAscii,)
    );
    assert_eq!(
        RootEntryNameV1::parse(format!("action-{}-v1", "a".repeat(256)).as_bytes()),
        CatalogNameClassificationV1::RecognizedInvalid(CatalogNameInvalidReasonV1::NameTooLong,)
    );
    assert_eq!(
        RootEntryNameV1::parse(b"notes.txt"),
        CatalogNameClassificationV1::Foreign,
    );
    assert_eq!(
        RootEntryNameV1::parse(&[0xff, 0xfe]),
        CatalogNameClassificationV1::Foreign,
    );
}

#[test]
fn action_grammar_validates_digest_role_width_encoding_range_and_version() {
    let action = ActionDigestV1::new([0xabu8; 32]);
    for slot in ActionSlotV1::all() {
        assert_eq!(
            ActionSlotV1::parse(action, slot.name(action).as_bytes()),
            CatalogNameClassificationV1::Valid(slot),
        );
    }

    let another = ActionDigestV1::new([0xacu8; 32]);
    assert_eq!(
        ActionSlotV1::parse(action, ActionSlotV1::all()[0].name(another).as_bytes(),),
        CatalogNameClassificationV1::RecognizedInvalid(
            CatalogNameInvalidReasonV1::ActionDigestMismatch,
        )
    );
    for (name, reason) in [
        (
            format!("action-{}-reservation-v1", "a".repeat(63)),
            CatalogNameInvalidReasonV1::InvalidActionDigestWidth,
        ),
        (
            format!("action-{}-reservation-v1", "g".repeat(64)),
            CatalogNameInvalidReasonV1::InvalidActionDigestEncoding,
        ),
        (
            format!("action-{}-barrier-intent-active-1-v1", action.hex()),
            CatalogNameInvalidReasonV1::InvalidOrdinalWidth,
        ),
        (
            format!("action-{}-barrier-intent-active-aa-v1", action.hex()),
            CatalogNameInvalidReasonV1::InvalidOrdinalEncoding,
        ),
        (
            format!("action-{}-barrier-intent-active-64-v1", action.hex()),
            CatalogNameInvalidReasonV1::OrdinalOutOfRange,
        ),
        (
            format!("action-{}-bootstrap-intent-active-24-v1", action.hex()),
            CatalogNameInvalidReasonV1::OrdinalOutOfRange,
        ),
        (
            format!("action-{}-retired-bootstrap-marker-08-v1", action.hex()),
            CatalogNameInvalidReasonV1::OrdinalOutOfRange,
        ),
        (
            format!("action-{}-reservation-v2", action.hex()),
            CatalogNameInvalidReasonV1::UnsupportedVersion,
        ),
        (
            format!("action-{}-unknown-role-v1", action.hex()),
            CatalogNameInvalidReasonV1::UnknownSlotRole,
        ),
        (
            format!("action-{}-v1", action.hex()),
            CatalogNameInvalidReasonV1::UnknownSlotRole,
        ),
        (
            format!("action-{}-{}-v1", action.hex(), "x".repeat(256)),
            CatalogNameInvalidReasonV1::NameTooLong,
        ),
    ] {
        assert_eq!(
            ActionSlotV1::parse(action, name.as_bytes()),
            CatalogNameClassificationV1::RecognizedInvalid(reason),
            "{name}",
        );
    }
    assert_eq!(
        ActionSlotV1::parse(action, b"notes.txt"),
        CatalogNameClassificationV1::Foreign,
    );
}

#[test]
fn occupancy_is_closed_and_preparing_never_admits_a_second_action() {
    let idle = CatalogOccupancyV1::new(0, 63, CatalogAdmissionOccupancyV1::Idle).unwrap();
    assert!(idle.can_admit_new());

    let without_final =
        CatalogOccupancyV1::new(0, 63, CatalogAdmissionOccupancyV1::PreparingWithoutFinal).unwrap();
    assert!(!without_final.can_admit_new());
    assert!(without_final.can_resume());

    let with_final =
        CatalogOccupancyV1::new(1, 63, CatalogAdmissionOccupancyV1::PreparingWithFinal).unwrap();
    assert!(!with_final.can_admit_new());
    assert!(with_final.can_resume());

    assert_eq!(
        CatalogOccupancyV1::new(0, 64, CatalogAdmissionOccupancyV1::PreparingWithoutFinal,),
        Err(CatalogOccupancyErrorV1::RetirementCreditsExceeded),
    );
    assert_eq!(
        CatalogOccupancyV1::new(0, 63, CatalogAdmissionOccupancyV1::PreparingWithFinal,),
        Err(CatalogOccupancyErrorV1::PreparingFinalMissing),
    );
}

#[test]
fn admission_record_digest_is_derived_and_verified_for_both_states() {
    let idle = ActionDirectoryAdmissionV1::idle();
    let preparing = ActionDirectoryAdmissionV1::preparing(&reservation());
    assert_ne!(idle.record_digest(), preparing.record_digest());
    for value in [idle, preparing] {
        let bytes = value.encode_canonical().unwrap();
        assert_eq!(
            read_bounded_record::<ActionDirectoryAdmissionV1>(Cursor::new(bytes.clone())).unwrap(),
            value,
        );
        let cbor = crate::cbor::try_decode(&bytes).unwrap();
        let mut wire = generated::CheckedActionDirectoryAdmissionV1::from_cbor(&cbor).unwrap();
        wire.record_digest[0] ^= 1;
        assert!(
            read_bounded_record::<ActionDirectoryAdmissionV1>(Cursor::new(crate::cbor::encode(
                &wire.to_cbor()
            )))
            .is_err()
        );
    }
}

#[test]
fn production_shaped_owner_issues_only_idle_exact_final_admission() {
    let reservation = reservation();
    let owner = CatalogAdmissionOwnerTestV1::new();
    let missing = owner.observe_missing();
    let exact = owner.observe_exact(
        linux_identity(1),
        RecordObservationV1::Exact(reservation.clone()),
        0,
    );
    let extra_child = owner.observe_exact(
        linux_identity(1),
        RecordObservationV1::Exact(reservation.clone()),
        1,
    );

    assert_eq!(
        owner.classify_handoff(
            &ActionDirectoryAdmissionV1::preparing(&reservation),
            &reservation,
            &missing,
            &exact,
        ),
        AdmissionHandoffDecisionV1::ReplacePreparingWithIdle,
    );
    assert!(
        owner
            .admit(
                &ActionDirectoryAdmissionV1::preparing(&reservation),
                &reservation,
                &missing,
                &exact,
            )
            .is_none()
    );
    assert!(
        owner
            .admit(
                &ActionDirectoryAdmissionV1::idle(),
                &reservation,
                &missing,
                &extra_child,
            )
            .is_none()
    );
    let admitted = owner
        .admit(
            &ActionDirectoryAdmissionV1::idle(),
            &reservation,
            &missing,
            &exact,
        )
        .unwrap();
    assert_eq!(admitted.reservation(), &reservation);
    assert_eq!(admitted.directory_identity(), &linux_identity(1));
}

#[test]
fn fixed_replacement_matrix_has_only_the_four_documented_progress_edges() {
    let old = ActionDirectoryAdmissionV1::idle();
    let new = ActionDirectoryAdmissionV1::preparing(&reservation());
    let other = ActionDirectoryAdmissionV1::preparing(&ActionCapacityReservationV1::new(
        ActionDigestV1::new([0xee; 32]),
        RequestOwnerBindingV1::new([0xff; 32]),
        ActionScheduleV1::try_new(0, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    ));
    let active = [
        RecordObservationV1::Missing,
        RecordObservationV1::PartialExpectedPrefix,
        RecordObservationV1::Exact(old.clone()),
        RecordObservationV1::Exact(new.clone()),
        RecordObservationV1::Exact(other.clone()),
        RecordObservationV1::Other,
    ];
    let scratch = [
        ScratchRecordObservationV1::Missing,
        ScratchRecordObservationV1::PartialExpectedPrefix,
        ScratchRecordObservationV1::Exact(old.clone()),
        ScratchRecordObservationV1::Exact(new.clone()),
        ScratchRecordObservationV1::Exact(other),
        ScratchRecordObservationV1::Other,
    ];

    for (active_index, active) in active.iter().enumerate() {
        for (scratch_index, scratch) in scratch.iter().enumerate() {
            let expected = match (active_index, scratch_index) {
                (2, 0 | 1) => FixedReplacementDecisionV1::WriteOrRewriteScratch,
                (2, 3) => FixedReplacementDecisionV1::ReplaceActiveFromScratch,
                (3, 0) => FixedReplacementDecisionV1::Complete,
                _ => FixedReplacementDecisionV1::Ambiguous,
            };
            assert_eq!(
                classify_fixed_replacement(active, scratch, &old, &new),
                expected,
                "active={active_index} scratch={scratch_index}",
            );
        }
    }
}

#[test]
fn owner_handoff_matrix_rejects_every_unlisted_directory_pair() {
    let expected = reservation();
    let other = ActionCapacityReservationV1::new(
        ActionDigestV1::new([0xee; 32]),
        RequestOwnerBindingV1::new([0xff; 32]),
        ActionScheduleV1::try_new(0, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    );
    let admission = ActionDirectoryAdmissionV1::preparing(&expected);
    let owner = CatalogAdmissionOwnerTestV1::new();
    let observations = [
        owner.observe_missing(),
        owner.observe_exact(
            linux_identity(1),
            RecordObservationV1::PartialExpectedPrefix,
            0,
        ),
        owner.observe_exact(
            linux_identity(1),
            RecordObservationV1::Exact(expected.clone()),
            0,
        ),
        owner.observe_exact(linux_identity(1), RecordObservationV1::Exact(other), 0),
        owner.observe_exact(
            linux_identity(1),
            RecordObservationV1::Exact(expected.clone()),
            1,
        ),
        owner.observe_other(),
    ];

    for (staging_index, staging) in observations.iter().enumerate() {
        for (final_index, final_directory) in observations.iter().enumerate() {
            let decision = match (staging_index, final_index) {
                (0, 0) => AdmissionHandoffDecisionV1::CreateStaging,
                (1, 0) => AdmissionHandoffDecisionV1::WriteOrRewriteReservation,
                (2, 0) => AdmissionHandoffDecisionV1::PublishStaging,
                (0, 2) => AdmissionHandoffDecisionV1::ReplacePreparingWithIdle,
                _ => AdmissionHandoffDecisionV1::Ambiguous,
            };
            assert_eq!(
                owner.classify_handoff(&admission, &expected, staging, final_directory),
                decision,
                "staging={staging_index} final={final_index}",
            );
        }
    }
    assert_eq!(
        owner.classify_handoff(
            &ActionDirectoryAdmissionV1::preparing(&expected),
            &ActionCapacityReservationV1::new(
                ActionDigestV1::new([0x11; 32]),
                RequestOwnerBindingV1::new([0x22; 32]),
                ActionScheduleV1::try_new(0, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
            ),
            &owner.observe_missing(),
            &owner.observe_missing(),
        ),
        AdmissionHandoffDecisionV1::Ambiguous,
    );
}
use std::io::Cursor;
