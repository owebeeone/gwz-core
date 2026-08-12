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
