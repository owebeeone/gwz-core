//! R2-D Step 2.2 — the host backend's fail-closed seam properties.
//!
//! The matrix in `tests_fault_matrix.rs` proves the three edges cross their
//! boundaries and converge. These cases prove the refusals that keep the edges
//! inside the frozen seam: no source without a §4.4 recheck arm, no edge from a
//! capability this backend did not retain, no edge onto an occupied row, no
//! barrier at an unscheduled ordinal, and no retained namespace over an action
//! directory the handoff does not name.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as the matrix file does.

use std::fs;

use super::tests_fault_matrix::{
    Fixture, TargetVariantV1, handoff, reservation, slot_leaf, with_catalog, write_scratch,
};
use super::{ActionNamespace, HostActionNamespaceV1, retain_action_namespace};
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::protocol::{ActionSlotV1, BaseActionSlotV1, ProtocolRecordKindV1};

const VARIANT: TargetVariantV1 = TargetVariantV1::Workspace;

#[track_caller]
fn refusal<T>(result: Result<T, CheckedFsError>, expected: &str) {
    match result {
        Ok(_) => panic!("expected a refusal mentioning {expected:?}, got success"),
        Err(error) => refused(&error, expected),
    }
}

#[track_caller]
fn refused(error: &CheckedFsError, expected: &str) {
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains(expected),
        "expected a refusal mentioning {expected:?}, got {rendered}"
    );
}

/// §4.3 assigns a §4.4 Class 1 recheck arm to rows E3, E7, E15, E16 and E17 —
/// not to E12/E13. A directory source therefore has no arm to verify its
/// interior with, so this backend refuses it instead of publishing one; the
/// managed directory publish that needs the arm is Phase 2.3/3's.
#[test]
fn a_directory_source_is_refused_because_step_2_2_carries_no_recheck_arm() {
    let fixture = Fixture::new("directory-source");
    let expected = reservation(0xE1, 2);
    let identity = fixture.admit(VARIANT, &expected);
    let action = expected.action_digest();
    let leaf = slot_leaf(ActionSlotV1::Base(BaseActionSlotV1::GoalScratch), action);
    fs::create_dir(
        fixture
            .action_directory(VARIANT, action)
            .join(std::str::from_utf8(leaf.as_bytes()).unwrap()),
    )
    .unwrap();

    let refused_source = with_catalog(&fixture, VARIANT, |catalog| {
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(&expected, &identity))?;
        Ok(namespace.retain_scheduled_source(leaf, ProtocolRecordKindV1::BarrierIntent))
    })
    .unwrap();
    refusal(refused_source, "not a canonical regular file");
}

/// The retained-handle contract: an edge consumes the one source this backend
/// itself retained and still holds. A proof for another role, and a proof whose
/// retention the previous edge already consumed, are both refused before any
/// physical mutation.
#[test]
fn only_the_source_this_backend_still_holds_can_drive_an_edge() {
    let fixture = Fixture::new("retained-source");
    let expected = reservation(0xE2, 2);
    let identity = fixture.admit(VARIANT, &expected);
    let action = expected.action_digest();
    let action_directory = fixture.action_directory(VARIANT, action);
    let scratch = slot_leaf(
        ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
        action,
    );
    let other = slot_leaf(ActionSlotV1::Base(BaseActionSlotV1::GoalScratch), action);
    write_scratch(&action_directory, &scratch, &expected);
    write_scratch(&action_directory, &other, &expected);

    with_catalog(&fixture, VARIANT, |catalog| {
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(&expected, &identity))?;
        let slots = namespace.scheduled_barrier_slots(0, scratch.clone())?;

        // Retaining a second source displaces the first, so the first proof is
        // no longer the capability this backend holds.
        let stale = namespace
            .retain_scheduled_source(scratch.clone(), ProtocolRecordKindV1::BarrierIntent)?;
        namespace.retain_scheduled_source(other, ProtocolRecordKindV1::BarrierIntent)?;
        refused(
            &namespace
                .publish_barrier_intent(&stale, &slots)
                .expect_err("a displaced source proof cannot drive an edge"),
            "not the capability this backend retained",
        );

        // A completed edge clears the retention, so replaying its proof is
        // refused rather than re-published.
        let source =
            namespace.retain_scheduled_source(scratch, ProtocolRecordKindV1::BarrierIntent)?;
        namespace.publish_barrier_intent(&source, &slots)?;
        refused(
            &namespace
                .publish_barrier_intent(&source, &slots)
                .expect_err("a consumed source proof cannot drive a second edge"),
            "no retained source capability",
        );
        Ok(())
    })
    .unwrap();
}

/// The sealed primitive's hardcoded `replace=false` is restated as a pre-edge
/// expectation, so an occupied deterministic row is a typed refusal and both
/// rows are left exactly as they were.
#[test]
fn an_occupied_destination_row_is_refused_and_leaves_both_rows_untouched() {
    let fixture = Fixture::new("occupied-destination");
    let expected = reservation(0xE3, 2);
    let identity = fixture.admit(VARIANT, &expected);
    let action = expected.action_digest();
    let action_directory = fixture.action_directory(VARIANT, action);
    let scratch = slot_leaf(
        ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
        action,
    );

    with_catalog(&fixture, VARIANT, |catalog| {
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(&expected, &identity))?;
        let slots = namespace.scheduled_barrier_slots(0, scratch.clone())?;
        let active = slots.active_leaf().clone();
        write_scratch(&action_directory, &scratch, &expected);
        write_scratch(&action_directory, &active, &expected);

        let source = namespace
            .retain_scheduled_source(scratch.clone(), ProtocolRecordKindV1::BarrierIntent)?;
        refused(
            &namespace
                .publish_barrier_intent(&source, &slots)
                .expect_err("an occupied destination row is refused before the edge"),
            "already occupied",
        );
        for leaf in [&scratch, &active] {
            assert!(
                namespace.scheduled_row_is_resident(leaf),
                "the refused edge mutated the namespace"
            );
        }
        Ok(())
    })
    .unwrap();
}

/// Barrier ordinals are schedule-derived: the reservation reserves two, so the
/// third is not a namespace capability this action can obtain.
#[test]
fn an_unscheduled_barrier_ordinal_is_refused() {
    let fixture = Fixture::new("unscheduled-barrier");
    let expected = reservation(0xE4, 2);
    let identity = fixture.admit(VARIANT, &expected);
    let action = expected.action_digest();
    let scratch = slot_leaf(
        ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
        action,
    );

    let unscheduled = with_catalog(&fixture, VARIANT, |catalog| {
        let namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(&expected, &identity))?;
        assert!(
            namespace
                .scheduled_barrier_slots(1, scratch.clone())
                .is_ok()
        );
        Ok(namespace.scheduled_barrier_slots(2, scratch))
    })
    .unwrap();
    refusal(unscheduled, "not scheduled");
}

/// Provenance is proved, not assumed: the retained action directory must be the
/// one the handoff names, so a handoff carrying another action directory's
/// identity is refused at the single no-follow hop.
#[test]
fn a_handoff_naming_another_action_directory_is_refused_at_retain() {
    let expected = reservation(0xE5, 2);
    let foreign = Fixture::new("foreign-action");
    let foreign_identity = foreign.admit(VARIANT, &expected);
    let fixture = Fixture::new("own-action");
    let own_identity = fixture.admit(VARIANT, &expected);
    assert_ne!(own_identity, foreign_identity);

    let error = with_catalog(&fixture, VARIANT, |catalog| {
        Ok(retain_action_namespace(&catalog, handoff(&expected, &foreign_identity)).err())
    })
    .unwrap()
    .expect("a foreign action identity must be refused");
    refused(&error, "identity changed");
}
