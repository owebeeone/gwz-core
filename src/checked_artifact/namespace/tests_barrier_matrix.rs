//! R2-E Phase E2.3 — the executed `barrier.*` interruption/restart/convergence
//! matrix, all sixteen keys.
//!
//! Controlling text: `GwzM5-8R2E-Plan.md` §3 Phase E2; the `barrier.*`
//! activation record at `GwzM5-8R2DInterfaceFreeze.md` §3.5, bound by
//! `GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §3 as amended by
//! `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §1/§5/§6.3 (addendum
//! controlling); `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 for the repeated-crash
//! rule; `GwzM5-8R4bP1P2-RemPlan-4.md` §4's R2 stop clause (:1089-1092) for the
//! stable deterministic slots the repeated-crash rows prove.
//!
//! Every row drives the real machinery against a real target: a real lease, the
//! sealed catalog owner, the frozen admission seam, the owner-minted
//! roaming-anchor-home witness, and a *fresh* retained action-namespace
//! capability per attempt — so an interruption is a real process stop across a
//! real durable barrier edge.
//!
//! **Why the drive takes two ordinals.** Rows #10/#11 and #12/#13 are two
//! boundaries sharing one helper (DECISION B-4), and exactly one of the two
//! entries runs per ordinal: the drive that created an alias retires it through
//! `OwnDrive`, and a drive that finds an alias it did not create retires it
//! through `Stranded`. One ordinal therefore cannot cross both. So the fixture
//! builds the stranded state before driving the retirement rows — the Step-4.2
//! standard, whose own `[P2-1]` closure was that two announced boundaries were
//! driven by nothing — by pre-placing ordinal 1's alias at its reserved leaf.
//! Ordinal 0 then takes the fresh path (#6-#11) and ordinal 1 the restart path
//! (#12-#13), and one virgin attempt crosses all sixteen.
//!
//! **The two reserved leaves are a fixture choice, and OPEN-B3's answer says
//! why they can be.** The gate at `scheduled_barrier_slots` requires a
//! canonical `ActionSlotV1` name of this action; no frozen slot names the
//! *live* alias, and E2.1 mints none, so the fixture picks two legal
//! action-scoped slots the barrier family does not otherwise use — one per
//! ordinal, because both ordinals of one action share a target parent under
//! OPEN-B2's answer.
//!
//! *That is true of the barrier family and not of the action* (E2 review
//! [P3-1]): `GoalScratch` and `AuthorityScratch` belong to other families, and
//! the gate is action-scoped rather than family-scoped, so it admits them. This
//! fixture drives no other family, so nothing collides here — but a real
//! consumer must not copy the choice. The limit and E4's obligation are recorded
//! on `require_reserved_target_leaf`.
//!
//! **The Git-directory route, stated as §11.3 item 2 requires.** Both variants
//! drive the same target: the retained *action directory*, reached through the
//! permit's own no-follow hop from the completed catalog. Neither takes the
//! Step-2.3 `cfg(test)` managed-parent door nor a managed prefix under a
//! target's own retained root, because OPEN-B2's answer keeps the barrier
//! target action-directory-pinned — so the Git-directory arm has no
//! `cfg(test)` dependency to revisit here, and the door's disposition stays
//! E4.2's.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as `tests_fault_matrix.rs`
//! does.

use std::path::Path;

use super::tests_fault_matrix::{
    Fixture, TargetVariantV1, handoff, reservation, slot_leaf, with_catalog,
};
use super::{ActionNamespace, HostActionNamespaceV1, retain_action_namespace};
use crate::checked_artifact::capability::{
    AliasRetirementEntryV1, AsciiComponent, BarrierIntentRowV1, CheckedFsError,
    DurableObjectIdentityV1, RoamingAnchorHomeWitnessV1, TargetAnchorAliasStateV1,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_barrier_fault, take_armed_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionSlotV1, BaseActionSlotV1,
    ProtocolRecordKindV1,
};

/// Every `barrier.*` boundary, in the order one virgin drive crosses them.
///
/// Ordinal 0 crosses the intent record's five, then the fresh alias path's six,
/// then the retirement's three; ordinal 1 re-crosses the intent five and takes
/// the stranded path's two instead of the fresh six.
const BARRIER_MATRIX: [Fault; 16] = [
    Fault::BarrierIntentScratchCreate,
    Fault::BarrierIntentScratchWrite,
    Fault::BarrierIntentScratchFlush,
    Fault::BarrierIntentPublish,
    Fault::BarrierIntentReobserve,
    Fault::BarrierAnchorOutbound,
    Fault::BarrierAnchorOutboundReobserve,
    Fault::BarrierTargetBarrier,
    Fault::BarrierTargetReobserve,
    Fault::BarrierAnchorReturn,
    Fault::BarrierAnchorReturnReobserve,
    Fault::BarrierTargetAliasRetire,
    Fault::BarrierTargetAliasReobserve,
    Fault::BarrierIntentRetire,
    Fault::BarrierIntentRetiredReobserve,
    Fault::BarrierCompletionReobserve,
];

/// **The inclusion criterion, unchanged from `namespace/tests_managed_matrix.rs`
/// and `bootstrap/managed/tests_intent_matrix.rs`.** A boundary is
/// *single-crossing* when the durable state its crash leaves routes the next
/// drive **past** it; every other boundary is repeatable, because a restart
/// re-enters it on the same durable state.
///
/// Six of the sixteen are repeatable. The three scratch boundaries are, because
/// the scratch slot is one deterministic row that a resume rewrites in place.
/// The two intent-publish boundaries are, for the reason the managed intent
/// lifecycle's are: the edge is guarded by residency and then observed
/// unconditionally, so a restart past the guarded rename still re-enters both of
/// its observation boundaries — that is what makes the lifecycle idempotent.
/// And the completion observation is, because it names a settled ordinal, which
/// every later drive re-observes and never mutates.
const REPEATED_BOUNDARIES: [Fault; 6] = [
    Fault::BarrierIntentScratchCreate,
    Fault::BarrierIntentScratchWrite,
    Fault::BarrierIntentScratchFlush,
    Fault::BarrierIntentPublish,
    Fault::BarrierIntentReobserve,
    Fault::BarrierCompletionReobserve,
];

/// The single-crossing complement, each with the routing that skips it.
///
/// The four fresh-alias boundaries are single-crossing for a reason specific to
/// DECISION B-4: a crash anywhere between the alias's creation and its
/// retirement leaves an alias the next drive cannot prove is its own, so the
/// restart takes the **stranded** entry and never re-enters the fresh path. The
/// four retirement boundaries are single-crossing in the ordinary way — the
/// rename they follow is exactly the state that advances the sequence — and so
/// are the two intent-retirement boundaries, after which the ordinal is settled
/// and the whole drive short-circuits to the completion observation.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 10] = [
    Fault::BarrierAnchorOutbound,
    Fault::BarrierAnchorOutboundReobserve,
    Fault::BarrierTargetBarrier,
    Fault::BarrierTargetReobserve,
    Fault::BarrierAnchorReturn,
    Fault::BarrierAnchorReturnReobserve,
    Fault::BarrierTargetAliasRetire,
    Fault::BarrierTargetAliasReobserve,
    Fault::BarrierIntentRetire,
    Fault::BarrierIntentRetiredReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin barrier drive settles two ordinals in six durable edges and leaves
/// five rows in the action directory, so twelve crashes cross the nominal
/// capacity several times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

/// The reserved target leaf of each ordinal. Legal under OPEN-B3's gate — both
/// are canonical `ActionSlotV1` names of this action — and used by nothing else
/// in the barrier sequence; see the module header for why the two ordinals
/// cannot share one.
const RESERVED_LEAVES: [BaseActionSlotV1; 2] = [
    BaseActionSlotV1::GoalScratch,
    BaseActionSlotV1::AuthorityScratch,
];

/// The ordinals this matrix drives. Ordinal 0 takes the fresh alias path
/// (`OwnDrive`), ordinal 1 the stranded one (`Stranded`), because the fixture
/// pre-places its alias.
const SCHEDULED_ORDINALS: [usize; 2] = [0, 1];

/// Which single ordinal crosses a given single-crossing boundary, for the probe.
///
/// The stranded retirement's two boundaries belong to ordinal 1, whose alias the
/// fixture placed; every other single-crossing boundary is crossed by ordinal 0,
/// which takes the fresh path and then retires its own intent.
const fn probe_ordinal(key: Fault) -> usize {
    match key {
        Fault::BarrierTargetAliasRetire | Fault::BarrierTargetAliasReobserve => 1,
        _ => 0,
    }
}

const ROAMING_ANCHOR_BYTES: &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";

fn reserved_leaf(index: usize, expected: &ActionCapacityReservationV1) -> AsciiComponent {
    slot_leaf(
        ActionSlotV1::Base(RESERVED_LEAVES[index]),
        expected.action_digest(),
    )
}

/// Builds the stranded state ordinal 1's restart path needs: an alias at its
/// reserved leaf that no drive of this fixture created.
///
/// The fixture places it rather than a drive, which is exactly the state key #12
/// names — "found at the target by a drive that did **not** create it" — and is
/// the Step-4.2 matrix's own technique for its two retirement boundaries.
fn strand_ordinal_one_alias(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) {
    let leaf = reserved_leaf(1, expected);
    let name = std::str::from_utf8(leaf.as_bytes()).expect("a scheduled slot name is ASCII");
    let directory = fixture.action_directory(variant, expected.action_digest());
    std::fs::write(directory.join(name), ROAMING_ANCHOR_BYTES)
        .expect("the action directory is writable");
}

/// One fresh-process barrier attempt: re-mint the roaming-anchor-home witness,
/// re-retain the action namespace through the permit, and drive both scheduled
/// ordinals forward by whichever edges the resident durable state has not yet
/// crossed.
///
/// The witness is re-minted here rather than carried, because that is O6's read
/// side: every resume re-observes the home from the owner's own retained
/// capabilities and refuses a resident intent that disagrees.
fn attempt(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
) -> Result<(), CheckedFsError> {
    attempt_ordinals(fixture, variant, expected, identity, &SCHEDULED_ORDINALS)
}

/// The same fresh-process attempt over a declared subset of the schedule's
/// ordinals.
///
/// The subset exists for the single-crossing probe alone. A barrier **ordinal**
/// is an independent scheduled sequence with its own intent record, its own
/// alias and its own retirement rows — the analogue of one bootstrap row in the
/// managed matrix — so "the durable state this crash leaves routes the next
/// drive past this boundary" is a per-ordinal property. A drive over two
/// ordinals legitimately crosses `intent_retire` twice, once per ordinal, and
/// probing it over the whole drive would be measuring the schedule's width
/// rather than the boundary's routing.
fn attempt_ordinals(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
    ordinals: &[usize],
) -> Result<(), CheckedFsError> {
    with_catalog(fixture, variant, |catalog| {
        let home = catalog.observe_roaming_anchor_home()?;
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(expected, identity))?;
        for index in ordinals {
            drive_ordinal(&mut namespace, *index, expected, &home)?;
        }
        Ok(())
    })
}

fn drive_ordinal(
    namespace: &mut ActionNamespace<HostActionNamespaceV1>,
    index: usize,
    expected: &ActionCapacityReservationV1,
    home: &RoamingAnchorHomeWitnessV1,
) -> Result<(), CheckedFsError> {
    let action = expected.action_digest();
    let scratch = slot_leaf(
        ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
        action,
    );
    let reserved = reserved_leaf(index, expected);
    let slots = namespace.scheduled_barrier_slots(index, reserved.clone())?;
    let active = slots.active_leaf().clone();
    let retired = slots.retired_leaf().clone();
    let retired_alias = slots.retired_anchor_alias_leaf().clone();

    // A settled ordinal is re-observed and never re-driven.
    if namespace.scheduled_row_is_resident(&retired) {
        return namespace.observe_barrier_completion(&slots);
    }

    if !namespace.scheduled_row_is_resident(&active) {
        let intent = namespace.barrier_intent(&slots, home).map_err(|_| {
            CheckedFsError::ambiguous("action barrier", "barrier intent is not issuable")
        })?;
        let bytes = intent.encode_canonical().map_err(|_| {
            CheckedFsError::ambiguous(
                "action barrier",
                "barrier intent is not canonically encodable",
            )
        })?;
        namespace.write_barrier_intent_scratch(&slots, &bytes)?;
        let source =
            namespace.retain_scheduled_source(scratch, ProtocolRecordKindV1::BarrierIntent)?;
        namespace.publish_barrier_intent(&source, &slots)?;
    }
    let bound = namespace.observe_barrier_intent_row(&slots, BarrierIntentRowV1::Active, home)?;

    if !namespace.scheduled_row_is_resident(&retired_alias) {
        // OPEN-B8, answered where it is decided: the two entries are partitioned
        // by the in-memory fact that *this* drive created the alias, never by
        // durable state — `BarrierIntentActive(ordinal)` is resident on both
        // paths, because the intent retires only at key #14.
        //
        // The residency question is asked of the owner, never of the reserved
        // leaf directly. Asking the leaf was the E2 review's [P2-1]: the roaming
        // barrier can leave the alias under an outbound name this consumer
        // cannot derive, so a leaf-only test answered "absent" after a
        // mid-round-trip crash and manufactured a second object.
        let entry = match namespace.converge_target_anchor_alias(&slots)? {
            TargetAnchorAliasStateV1::Stranded => AliasRetirementEntryV1::Stranded,
            TargetAnchorAliasStateV1::Absent => {
                namespace.create_target_anchor_alias(&slots)?;
                namespace.barrier_target_parent(&slots, &bound)?;
                AliasRetirementEntryV1::OwnDrive
            }
        };
        let source =
            namespace.retain_scheduled_source(reserved, ProtocolRecordKindV1::Infrastructure)?;
        namespace.retire_barrier_target_alias(&source, &slots)?;
        namespace.observe_retired_target_anchor_alias(&slots, entry)?;
    }

    let source = namespace.retain_scheduled_source(active, ProtocolRecordKindV1::BarrierIntent)?;
    namespace.retire_barrier_intent(&source, &slots)?;
    namespace.observe_barrier_intent_row(&slots, BarrierIntentRowV1::Retired, home)?;
    namespace.observe_barrier_completion(&slots)
}

/// The executed key list must reconcile against the vocabulary's own
/// `barrier.*` inventory, so a key added to the family without a matrix row
/// fails here rather than silently escaping
/// (`interface_tests/fault_expected_keys.rs`).
fn reconcile_executed_keys() {
    let mut actual = BARRIER_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("barrier.").then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 16, "the barrier family is sixteen keys");
}

/// Every boundary is classified exactly once, so a key can neither escape the
/// single-crossing probe nor be claimed by both partitions.
fn assert_boundary_partition() {
    for key in BARRIER_MATRIX {
        let repeated = REPEATED_BOUNDARIES.contains(&key);
        let single = SINGLE_CROSSING_BOUNDARIES.contains(&key);
        assert!(
            repeated ^ single,
            "{} is in neither or both classifications",
            key.stable_key()
        );
    }
    assert_eq!(
        REPEATED_BOUNDARIES.len() + SINGLE_CROSSING_BOUNDARIES.len(),
        BARRIER_MATRIX.len(),
        "the classification does not cover the whole family"
    );
}

fn suffix(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .1
}

fn settle(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
    context: &str,
) -> Vec<String> {
    attempt(fixture, variant, expected, identity)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    fixture.action_children(variant, expected.action_digest())
}

/// A fresh fixture with its admitted action and ordinal 1's stranded alias in
/// place, ready to be interrupted.
fn prepared(
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    label: &str,
) -> (Fixture, DurableObjectIdentityV1) {
    let fixture = Fixture::new(label);
    let identity = fixture.admit(variant, expected);
    strand_ordinal_one_alias(&fixture, variant, expected);
    (fixture, identity)
}

fn interrupt(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
    key: Fault,
    context: &str,
) {
    interrupt_ordinals(
        fixture,
        variant,
        expected,
        identity,
        key,
        context,
        &SCHEDULED_ORDINALS,
    );
}

/// One interruption binds its fixture, target, reservation, handoff, key, label
/// and ordinal subset — every one of them a fact the row under test needs.
fn interrupt_ordinals(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
    key: Fault,
    context: &str,
    ordinals: &[usize],
) {
    run_next_barrier_fault(key, || panic!("simulated barrier process stop"));
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = attempt_ordinals(fixture, variant, expected, identity, ordinals);
    }));
    assert!(
        interrupted.is_err(),
        "fault point was not reached: {context}"
    );
}

/// Interrupt at every `barrier.*` boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    assert_boundary_partition();
    let expected = reservation(0xB1, 2);
    let settled_rows = {
        let (fixture, identity) = prepared(
            variant,
            &expected,
            &format!("barrier-{}-settled", variant.label()),
        );
        settle(&fixture, variant, &expected, &identity, "baseline")
    };

    for key in BARRIER_MATRIX {
        let stable = key.stable_key();
        let (fixture, identity) = prepared(
            variant,
            &expected,
            &format!("barrier-{}-{}", variant.label(), suffix(&stable)),
        );

        interrupt(&fixture, variant, &expected, &identity, key, &stable);

        let resumed_rows = settle(&fixture, variant, &expected, &identity, &stable);
        assert_eq!(
            resumed_rows, settled_rows,
            "{stable}: the restart did not converge to the settled action directory"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-retains the same namespace, re-mints the same witness, and mutates
        // nothing.
        let again_rows = settle(&fixture, variant, &expected, &identity, &stable);
        assert_eq!(
            again_rows, settled_rows,
            "{stable}: the resume mutated the settled action directory"
        );

        println!(
            "{stable} | {} | interrupted=yes | restart=settled | rows={} | resume=no-mutation",
            variant.label(),
            resumed_rows.len()
        );
    }
}

/// A single-crossing boundary must not be re-crossed by the drive that recovers
/// from its own crash. Armed a second time, the probe survives the settling
/// drive and is taken back unfired.
///
/// Driven over the **one ordinal** that crosses each boundary, for the reason
/// `attempt_ordinals` states: an ordinal is its own scheduled sequence, so a
/// two-ordinal drive crosses `intent_retire` twice by design and probing it
/// over the whole drive would measure the schedule's width instead of the
/// boundary's routing.
fn run_single_crossing_probe(variant: TargetVariantV1) {
    let expected = reservation(0xB2, 2);
    for key in SINGLE_CROSSING_BOUNDARIES {
        let stable = key.stable_key();
        let ordinals = [probe_ordinal(key)];
        let (fixture, identity) = prepared(
            variant,
            &expected,
            &format!("barrier-s-{}-{}", variant.label(), suffix(&stable)),
        );

        interrupt_ordinals(
            &fixture, variant, &expected, &identity, key, &stable, &ordinals,
        );

        run_next_barrier_fault(key, || {
            panic!("a single-crossing boundary was re-crossed by the drive after its crash")
        });
        attempt_ordinals(&fixture, variant, &expected, &identity, &ordinals)
            .unwrap_or_else(|error| panic!("{stable}: the restart must settle: {error:?}"));
        assert!(
            take_armed_fault(),
            "{stable}: the probe's arm vanished without firing"
        );

        // The ordinal really did settle, not merely stop crossing the boundary.
        attempt(&fixture, variant, &expected, &identity)
            .unwrap_or_else(|error| panic!("{stable}: the whole drive must settle: {error:?}"));

        println!(
            "{stable} | {} | ordinal={} | single-crossing=yes | restart=settled | re-crossed=no",
            variant.label(),
            ordinals[0]
        );
    }
}

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
///
/// Every repeatable boundary is driven, not a sub-selection of them: the family
/// has only six, and each one is a row a retry could be tempted to re-name — the
/// three scratch boundaries share one deterministic slot across both ordinals,
/// the two publish boundaries re-enter an already-published row, and the
/// completion observation re-reads a settled ordinal.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let expected = reservation(0xB3, 2);
    let settled_rows = {
        let (fixture, identity) = prepared(
            variant,
            &expected,
            &format!("barrier-r-{}-settled", variant.label()),
        );
        settle(&fixture, variant, &expected, &identity, "baseline")
    };

    for key in REPEATED_BOUNDARIES {
        let stable = key.stable_key();
        let (fixture, identity) = prepared(
            variant,
            &expected,
            &format!("barrier-r-{}-{}", variant.label(), suffix(&stable)),
        );
        let mut census: Option<Vec<String>> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            interrupt(
                &fixture,
                variant,
                &expected,
                &identity,
                key,
                &format!("{stable}: round {round}"),
            );

            let observed = fixture.action_children(variant, expected.action_digest());
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let rows = census.expect("the boundary is crashed at least once");
        let converged_rows = settle(&fixture, variant, &expected, &identity, &stable);
        assert_eq!(
            converged_rows, settled_rows,
            "{stable}: the action directory did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | slots-stable=yes | rows={rows:?} | converged=yes",
            variant.label()
        );
    }
}

/// The rows a fully settled two-ordinal drive leaves, by name rather than by
/// count: the admitted action's own reservation, and per ordinal its retired
/// intent row and its retired roaming-anchor alias row. Nothing at either
/// reserved leaf, and no scratch — the whole point of DECISION B-5's disposal.
fn settled_census(action: ActionDigestV1) -> Vec<String> {
    let mut rows = vec![
        ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(action),
        ActionSlotV1::BarrierIntentRetired(0).name(action),
        ActionSlotV1::BarrierIntentRetired(1).name(action),
        ActionSlotV1::RetiredRoamingAnchorAlias(0).name(action),
        ActionSlotV1::RetiredRoamingAnchorAlias(1).name(action),
    ];
    rows.sort();
    rows
}

/// The settled census, pinned by name rather than by count: two ordinals, each
/// leaving its retired intent row and its retired roaming-anchor alias row, plus
/// the admitted action's own reservation. Nothing is left at either reserved
/// leaf, and no scratch survives — the whole point of DECISION B-5's disposal.
fn assert_settled_census(variant: TargetVariantV1) {
    let expected = reservation(0xB4, 2);
    let (fixture, identity) = prepared(
        variant,
        &expected,
        &format!("barrier-census-{}", variant.label()),
    );
    let rows = settle(&fixture, variant, &expected, &identity, "census");
    let action = expected.action_digest();
    assert_eq!(
        rows,
        settled_census(action),
        "{}: settled census",
        variant.label()
    );

    // The retired alias rows carry the frozen roaming-anchor bytes: what was
    // retired is the derived copy, and the catalog's own home row never moved.
    let directory = fixture.action_directory(variant, action);
    for ordinal in 0..2_u8 {
        let name = ActionSlotV1::RetiredRoamingAnchorAlias(ordinal).name(action);
        assert_eq!(
            std::fs::read(directory.join(name)).unwrap(),
            ROAMING_ANCHOR_BYTES,
            "{}: retired alias {ordinal}",
            variant.label()
        );
    }
    assert_catalog_home_untouched(&fixture, variant);
}

/// DECISION B-5's own claim, executed rather than asserted in prose: the
/// catalog's `roaming-anchor-home-v1` row is still exactly where it was, with
/// exactly the frozen bytes, after two complete barriers.
fn assert_catalog_home_untouched(fixture: &Fixture, variant: TargetVariantV1) {
    let home = fixture.catalog_root(variant).join("roaming-anchor-home-v1");
    assert_eq!(
        std::fs::read(&home).unwrap(),
        ROAMING_ANCHOR_BYTES,
        "the catalog's roaming anchor home was disturbed by a barrier"
    );
    // And the catalog still retains: the strongest available statement that no
    // completion predicate was broken, because `recover_or_create` refuses a
    // catalog whose exact retired layout no longer holds.
    with_catalog(fixture, variant, |catalog: OpaqueRetainedCatalogV1<'_>| {
        catalog.observe_roaming_anchor_home().map(|_| ())
    })
    .expect("the completed catalog must still retain after the barriers");
    let _: &Path = fixture.path();
}

/// The outbound name the roaming barrier's Windows arm renames an alias to
/// mid-round-trip. Derived here the way `platform::roundtrip_name` derives it,
/// because the two rows below have to *build* that state on disk — there is no
/// fault key inside the round trip, so the crash cannot be simulated by arming
/// one. That is the Step-4.2 technique: its own two retirement boundaries were
/// only reachable from a state the matrix built directly, and the review that
/// closed them graded "announced, injected, and driven by nothing" as the defect.
fn outbound_name(leaf: &AsciiComponent) -> String {
    format!(
        "{}.roundtrip",
        std::str::from_utf8(leaf.as_bytes()).expect("a scheduled slot name is ASCII")
    )
}

fn place_bytes(fixture: &Fixture, variant: TargetVariantV1, action: ActionDigestV1, name: &str) {
    std::fs::write(
        fixture.action_directory(variant, action).join(name),
        ROAMING_ANCHOR_BYTES,
    )
    .expect("the action directory is writable");
}

/// **The E2 review's [P2-1], driven.** A crash between the roaming round trip's
/// two renames leaves the alias out under `<reserved leaf>.roundtrip` with the
/// reserved leaf empty.
///
/// Before the remediation the drive branched on the reserved leaf alone: it
/// answered "absent", created a *second* object, tripped the barrier's own
/// both-names guard, refused the whole attempt, and left the outbound name
/// permanently — a name no later drive returned, because the following attempt
/// settled the ordinal and every attempt after that short-circuits.
///
/// Now the entry decision is `converge_target_anchor_alias`, which asks
/// `platform` — the owner of both names — and returns the object before it
/// answers. This asserts all three properties the old shape failed: **one**
/// attempt settles (no refusal), the census is exactly the ordinary settled one,
/// and **nothing** is left under the outbound name.
fn assert_mid_round_trip_residue_converges(variant: TargetVariantV1) {
    let expected = reservation(0xB5, 2);
    let (fixture, identity) = prepared(
        variant,
        &expected,
        &format!("barrier-converge-{}", variant.label()),
    );
    let action = expected.action_digest();
    let outbound = outbound_name(&reserved_leaf(0, &expected));
    place_bytes(&fixture, variant, action, &outbound);

    // One attempt, and it must not refuse.
    attempt(&fixture, variant, &expected, &identity).unwrap_or_else(|error| {
        panic!("the mid-round-trip residue must converge in one attempt: {error:?}")
    });

    let rows = fixture.action_children(variant, action);
    assert_eq!(
        rows,
        settled_census(action),
        "{}: the converged drive did not settle to the ordinary census",
        variant.label()
    );
    assert!(
        !rows.contains(&outbound),
        "{}: the outbound name survived the converge: {rows:?}",
        variant.label()
    );
}

/// The legacy shape a pre-remediation binary could leave: an alias resident at
/// the reserved leaf **and** a second object under the outbound name.
///
/// It is refused nowhere and wedges nothing. The ordinal settles through the
/// `Stranded` entry exactly as it would without the residue, and the outbound
/// object is left as a tolerated orphan — the disposition
/// `prepare_roaming_target` states and the same one the legacy nonce orphans
/// have. Refusing instead would be a permanent typed refusal on a reachable
/// state whose only convergence is a removal, which Step 4.2 replaced with
/// durable retirement and for which there is one slot per ordinal.
fn assert_legacy_both_names_settle_with_a_tolerated_orphan(variant: TargetVariantV1) {
    let expected = reservation(0xB6, 2);
    let (fixture, identity) = prepared(
        variant,
        &expected,
        &format!("barrier-legacy-{}", variant.label()),
    );
    let action = expected.action_digest();
    let reserved = reserved_leaf(0, &expected);
    let outbound = outbound_name(&reserved);
    place_bytes(
        &fixture,
        variant,
        action,
        std::str::from_utf8(reserved.as_bytes()).unwrap(),
    );
    place_bytes(&fixture, variant, action, &outbound);

    attempt(&fixture, variant, &expected, &identity).unwrap_or_else(|error| {
        panic!("a legacy both-names tree must still settle its ordinal: {error:?}")
    });

    let mut want = settled_census(action);
    want.push(outbound.clone());
    want.sort();
    let rows = fixture.action_children(variant, action);
    assert_eq!(
        rows,
        want,
        "{}: the ordinal must settle with the outbound object tolerated, not wedged",
        variant.label()
    );

    // Tolerated means tolerated: a later drive re-observes the settled ordinal
    // and neither removes the orphan nor trips over it.
    let again = settle(&fixture, variant, &expected, &identity, "legacy-resume");
    assert_eq!(
        rows,
        again,
        "{}: the resume mutated a settled ordinal",
        variant.label()
    );
}

#[test]
fn a_mid_round_trip_roaming_residue_converges_on_a_workspace_target() {
    assert_mid_round_trip_residue_converges(TargetVariantV1::Workspace);
}

#[test]
fn a_mid_round_trip_roaming_residue_converges_on_a_git_directory_target() {
    assert_mid_round_trip_residue_converges(TargetVariantV1::GitDirectory);
}

#[test]
fn a_legacy_both_names_tree_settles_with_a_tolerated_orphan_on_a_workspace_target() {
    assert_legacy_both_names_settle_with_a_tolerated_orphan(TargetVariantV1::Workspace);
}

#[test]
fn a_legacy_both_names_tree_settles_with_a_tolerated_orphan_on_a_git_directory_target() {
    assert_legacy_both_names_settle_with_a_tolerated_orphan(TargetVariantV1::GitDirectory);
}

#[test]
fn barrier_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn barrier_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn single_crossing_barrier_boundaries_are_not_re_crossed_on_a_workspace_target() {
    run_single_crossing_probe(TargetVariantV1::Workspace);
}

#[test]
fn single_crossing_barrier_boundaries_are_not_re_crossed_on_a_git_directory_target() {
    run_single_crossing_probe(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_barrier_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_barrier_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}

#[test]
fn the_settled_barrier_census_is_the_scheduled_rows_on_a_workspace_target() {
    assert_settled_census(TargetVariantV1::Workspace);
}

#[test]
fn the_settled_barrier_census_is_the_scheduled_rows_on_a_git_directory_target() {
    assert_settled_census(TargetVariantV1::GitDirectory);
}
