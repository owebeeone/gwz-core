use super::super::protocol::{
    ACTION_BUDGET_V1, ActionDigestV1, ActionSlotV1, BaseActionSlotV1, InfrastructureSlotV1,
    MAX_ACTION_SLOTS, MAX_ROOT_ENTRIES, RETIRED_ROOT_BUDGET_V1, ROOT_BUDGET_V1, RootEntryNameV1,
};

#[test]
fn literal_catalog_budgets_are_frozen() {
    assert_eq!(MAX_ROOT_ENTRIES, 74);
    assert_eq!(MAX_ACTION_SLOTS, 261);
    assert_eq!(ROOT_BUDGET_V1.tuple(), (74, 18_870, 18_944));
    assert_eq!(ACTION_BUDGET_V1.tuple(), (261, 66_555, 66_816));
    assert_eq!(RETIRED_ROOT_BUDGET_V1.tuple(), (64, 16_320, 16_384));
}

#[test]
fn infrastructure_and_base_slot_grammars_are_exact() {
    let infrastructure = InfrastructureSlotV1::ALL
        .iter()
        .map(|slot| slot.name())
        .collect::<Vec<_>>();
    assert_eq!(
        infrastructure,
        [
            "catalog-format-v1",
            "catalog-anchor-a-v1",
            "catalog-anchor-b-v1",
            "roaming-anchor-home-v1",
            "retired-actions-v1",
            "retired-actions-descriptor-v1",
            "catalog-bootstrap-retired-v1",
            "action-admission-active-v1",
            "action-admission-scratch-v1",
            "action-admission-staging-v1",
        ]
    );
    let base = BaseActionSlotV1::ALL
        .iter()
        .map(|slot| slot.suffix())
        .collect::<Vec<_>>();
    assert_eq!(
        base,
        [
            "reservation",
            "authority",
            "source-payload",
            "goal-payload",
            "authority-scratch",
            "goal-scratch",
            "record-scratch",
            "barrier-intent-scratch",
            "bootstrap-intent-scratch",
            "retired-source-alias",
            "retired-goal-alias",
            "retired-authority-alias",
            "cleanup-worklist",
        ]
    );
}

#[test]
fn all_261_action_slots_have_unique_deterministic_names() {
    let action = ActionDigestV1::new([0xabu8; 32]);
    let slots = ActionSlotV1::all();
    assert_eq!(slots.len(), 261);
    let mut names = slots
        .iter()
        .map(|slot| slot.name(action))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 261);
    for slot in slots {
        let name = slot.name(action);
        assert_eq!(ActionSlotV1::parse(action, name.as_bytes()), Some(slot));
        assert!(name.len() <= 255);
    }
}

#[test]
fn root_name_parser_is_ascii_canonical_and_versioned() {
    let action = ActionDigestV1::new([7; 32]);
    let active = RootEntryNameV1::ActiveAction(action);
    let name = active.name();
    assert_eq!(RootEntryNameV1::parse(name.as_bytes()), Some(active));
    assert_eq!(
        RootEntryNameV1::parse(b"catalog-format-v1"),
        Some(RootEntryNameV1::Infrastructure(
            InfrastructureSlotV1::CatalogFormat
        ))
    );
    assert_eq!(RootEntryNameV1::parse(name.to_uppercase().as_bytes()), None);
    assert_eq!(RootEntryNameV1::parse(b"action-v2-00"), None);
    assert_eq!(RootEntryNameV1::parse(&[0xff, 0xfe]), None);
}

/// R2-D Phase 1 (C-3, `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2). The
/// pre-catalog interior observer now reads the catalog root through this
/// classification instead of walking `InfrastructureSlotV1::ALL` alone. Every
/// arm is derived from the already-frozen `RootEntryNameV1` grammar, so the
/// widening mints no name; this pins the mapping onto the six
/// `GwzM5-8R4bR2ConsumerCheckpoint.md` §6 (:199-201) classes.
#[test]
fn catalog_root_rows_classify_into_the_six_global_enumeration_classes() {
    use super::super::protocol::{CatalogNameInvalidReasonV1, CatalogRootRowClassV1};
    use CatalogRootRowClassV1 as Class;

    let action = ActionDigestV1::new([0x5a; 32]);
    let cases: [(&[u8], Class); 8] = [
        (
            b"catalog-format-v1",
            Class::Infrastructure(InfrastructureSlotV1::CatalogFormat),
        ),
        (
            b"action-admission-active-v1",
            Class::Infrastructure(InfrastructureSlotV1::ActionAdmissionActive),
        ),
        (
            b"action-admission-scratch-v1",
            Class::ScheduledScratch(InfrastructureSlotV1::ActionAdmissionScratch),
        ),
        (
            b"action-admission-staging-v1",
            Class::ScheduledScratch(InfrastructureSlotV1::ActionAdmissionStaging),
        ),
        (
            b"retired-actions-v1",
            Class::Retired(InfrastructureSlotV1::RetiredActions),
        ),
        (
            b"catalog-bootstrap-retired-v1",
            Class::Retired(InfrastructureSlotV1::CatalogBootstrapRetired),
        ),
        (
            b"action-5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a-v1",
            Class::ActiveAction(action),
        ),
        (b"README.md", Class::Foreign),
    ];
    for (name, expected) in cases {
        assert_eq!(
            CatalogRootRowClassV1::classify(name),
            expected,
            "catalog root row misclassified: {}",
            String::from_utf8_lossy(name)
        );
    }
    // A recognized-but-invalid row is its own class and never collapses into
    // `Foreign`: the action prefix is owned, so the row is ours and malformed.
    assert_eq!(
        CatalogRootRowClassV1::classify(b"action-00-v1"),
        Class::MalformedRecognized(CatalogNameInvalidReasonV1::InvalidActionDigestWidth)
    );
    assert_eq!(
        CatalogRootRowClassV1::classify(b"catalog-format-v2"),
        Class::MalformedRecognized(CatalogNameInvalidReasonV1::UnsupportedVersion)
    );
    // Only the three infrastructure-bearing arms carry a slot.
    assert_eq!(
        CatalogRootRowClassV1::classify(
            b"action-5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a-v1"
        )
        .infrastructure_slot(),
        None
    );
    assert_eq!(
        CatalogRootRowClassV1::classify(b"retired-actions-v1").infrastructure_slot(),
        Some(InfrastructureSlotV1::RetiredActions)
    );
}

/// C-3's third fact: the ten-slot interior cap and the root grammar had to move
/// together, and both move only onto vocabulary R1+C0 already froze. The
/// observer's bound is the frozen `MAX_ROOT_ENTRIES`, and the two per-family
/// caps below it partition that budget exactly.
#[test]
fn the_widened_interior_bound_is_the_frozen_root_entry_budget() {
    use super::super::protocol::{MAX_ACTIVE_ACTION_DIRS, MAX_INFRASTRUCTURE_ENTRIES};

    assert_eq!(
        MAX_ROOT_ENTRIES,
        MAX_INFRASTRUCTURE_ENTRIES + MAX_ACTIVE_ACTION_DIRS
    );
    assert_eq!(MAX_INFRASTRUCTURE_ENTRIES, InfrastructureSlotV1::ALL.len());

    let interior = include_str!("../capability/pre_catalog/provider/interior.rs");
    assert!(
        interior.contains("const MAX_INTERIOR_ENTRIES: usize = MAX_ROOT_ENTRIES;"),
        "the interior observer no longer bounds the catalog root by the frozen root budget"
    );
    for required in [
        "MAX_INFRASTRUCTURE_ENTRIES",
        "MAX_ACTIVE_ACTION_DIRS",
        "CatalogRootRowClassV1::ActiveAction",
    ] {
        assert!(
            interior.contains(required),
            "the C-3 interior widening lost {required}"
        );
    }
}
