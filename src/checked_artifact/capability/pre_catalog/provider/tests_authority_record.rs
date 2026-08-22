//! R2-D Step 2.4 acceptance tests — the authority parse / streamed proof split.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.4;
//! `GwzM5-8R2DInterfaceFreeze.md` §3.3 (the seam consumed) and §4.3 (the E9
//! writer-class annotation this binding carries);
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :232-237 (payload size is not
//! protocol-record size) and §12 (the "payloads above one MiB plus
//! protocol-limit-plus-one rejection" matrix row).
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`check_checked_artifact_boundaries.py`) and out of the injection-site
//! rescan (`interface_tests/fault_expected_keys.rs`), exactly as the Step 2.1
//! harness does.

use std::io::Write;
use std::path::Path;

use cap_std::fs::Dir;

use super::authority_record_binding::{
    AuthorityTransactionV1, FOREIGN_AUTHORITY_REFUSAL, FOREIGN_EXACT_DURABLE_IS_WEAKER,
    ObservedLeafWriterClassV1, RetainedActionDirectoryV1, StreamedPayloadProofV1,
    install_authority_record, observe_streamed_payloads, read_resident_authority_record,
    retire_authority_record, validate_terminal_relation,
};
use super::directory_mutation::sync_directory_edge;
use super::platform::HostPlatform;
use super::tests_leaf_observation::{
    BarrierNamespaceV1, ExpectedPayloadV1, LeafFixture, census, open_dir, write_leaf,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, PathEquivalenceProvider,
};
use crate::checked_artifact::namespace::test_support::retained_directory;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, BaseActionSlotV1,
    CheckedAuthorityObservationV1, CheckedAuthorityRecordV1, CleanupAliasSetV1,
    ManagedBootstrapInputV1, ProtocolRecordKindV1, RequestOwnerBindingV1,
    retained_authority_observation_owner,
};

/// One MiB plus a tail, so every source payload in this file is well past both
/// the observer's 8 KiB streaming window and the authority record's 16 KiB
/// bound. ConsumerCheckpoint §12 requires the above-one-MiB row; making it the
/// *default* payload here means every test in the file carries it.
const SOURCE_PAYLOAD_BYTES: usize = 1024 * 1024 + 37;
const GOAL_PAYLOAD_BYTES: usize = 1024 * 1024 + 111;

fn reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([0x2d; 32]),
        RequestOwnerBindingV1::new([0x24; 32]),
        ActionScheduleV1::try_new(
            2,
            vec![ManagedBootstrapInputV1::new([3; 32], 2).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| ((index as u32 % 251) as u8) ^ seed)
        .collect()
}

fn slot_name(expected: &ActionCapacityReservationV1, slot: BaseActionSlotV1) -> String {
    ActionSlotV1::Base(slot).name(expected.action_digest())
}

/// Mints the retained action-directory capability the way the production
/// namespace owner mints it (`namespace_mutation::retain_action_namespace`):
/// a **bound** canonical path component, carrying the live parent's durable
/// identity, invocation identity and rename domain.
///
/// The Step 2.1 harness's `retain` deliberately uses a bare component, which is
/// all a leaf observation needs. An authority observation needs more: it turns
/// the capability's own path profile into the record's `artifact_root`, and
/// `DurablePathV1::from_live` accepts only bound components. Using the
/// production shape here is what makes these tests exercise the real issuing
/// path rather than a synthetic one.
pub(super) fn retain_action(
    parent_of_action: &Path,
    action_leaf: &str,
) -> RetainedActionDirectoryV1 {
    let parent = open_dir(parent_of_action);
    let handle = open_dir(&parent_of_action.join(action_leaf));
    let fact = HostPlatform.dir_identity(&handle).unwrap();
    let parent_fact = HostPlatform.dir_identity(&parent).unwrap();
    let profile = CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(action_leaf.as_bytes()).unwrap(),
            HostPlatform.parent_mode(&parent).unwrap(),
            parent_fact.durable().clone(),
            parent_fact.invocation().clone(),
            HostPlatform.rename_domain(&parent).unwrap(),
        )
        .unwrap(),
    ])
    .unwrap();
    retained_directory(handle, fact.durable().clone(), profile)
}

/// An action-shaped directory with both payload slots resident and durable.
struct AuthorityFixture {
    fixture: LeafFixture,
    expected: ActionCapacityReservationV1,
    source: Vec<u8>,
    goal: Vec<u8>,
}

impl AuthorityFixture {
    fn new(label: &str) -> Self {
        let fixture = LeafFixture::new(label);
        let expected = reservation();
        let source = payload(SOURCE_PAYLOAD_BYTES, 0x5a);
        let goal = payload(GOAL_PAYLOAD_BYTES, 0xa5);
        let parent = fixture.parent();
        write_leaf(
            &parent,
            &slot_name(&expected, BaseActionSlotV1::SourcePayload),
            &source,
        );
        write_leaf(
            &parent,
            &slot_name(&expected, BaseActionSlotV1::GoalPayload),
            &goal,
        );
        Self {
            fixture,
            expected,
            source,
            goal,
        }
    }

    fn dir(&self) -> Dir {
        open_dir(&self.fixture.parent_path())
    }

    fn retained(&self) -> RetainedActionDirectoryV1 {
        let path = self.fixture.parent_path();
        retain_action(path.parent().expect("the fixture root"), "parent")
    }

    /// The complete production read: stream both payloads through the landed
    /// observer, then issue one coherent observation through the R1 owner.
    fn observe(
        &self,
        parent: &RetainedActionDirectoryV1,
        writer_class: ObservedLeafWriterClassV1,
    ) -> (StreamedPayloadProofV1, CheckedAuthorityObservationV1) {
        let source = ExpectedPayloadV1::new(self.source.clone());
        let goal = ExpectedPayloadV1::new(self.goal.clone());
        let mut namespace = BarrierNamespaceV1::default();
        let proof = observe_streamed_payloads(
            parent,
            self.expected.action_digest(),
            writer_class,
            &source,
            &goal,
            &mut namespace,
            (0, 1),
        )
        .expect("both payload leaves are exact and durable");
        assert_eq!(
            (source.opens(), goal.opens()),
            (2, 2),
            "each payload is opened twice by the observer and never rewound"
        );
        assert_eq!(
            namespace.crossed(),
            2,
            "each payload proof crosses its own scheduled barrier"
        );
        let owner =
            retained_authority_observation_owner(AuthorityTransactionV1::from_streamed_proof(
                self.expected.request_owner_binding(),
                proof.clone(),
            ));
        let observation = owner
            .observe(&self.expected)
            .expect("the streamed transaction issues one coherent observation");
        (proof, observation)
    }
}

/// The step's headline property, proved end to end on real payloads: the
/// record path stays inside its frozen 16 KiB bound while the payload path
/// streams more than a megabyte on each side, and the two are joined only by
/// the terminal relation.
#[test]
fn the_bounded_record_and_the_streamed_payloads_are_separate_budgets() {
    let fixture = AuthorityFixture::new("split");
    let parent = fixture.retained();
    let (proof, observation) = fixture.observe(&parent, ObservedLeafWriterClassV1::GwzWritten);

    let record = CheckedAuthorityRecordV1::issue(&observation).expect("the record issues");
    let encoded = record.encode_canonical();
    assert!(
        encoded.len() <= ProtocolRecordKindV1::Authority.max_bytes(),
        "the authority record must stay inside its frozen protocol-record bound"
    );

    let (source_length, goal_length) = proof.payload_lengths();
    assert!(
        source_length > ProtocolRecordKindV1::Authority.max_bytes() as u64
            && goal_length > ProtocolRecordKindV1::Authority.max_bytes() as u64,
        "both payloads must exceed the record bound, or the separation is untested"
    );
    assert!(
        source_length > 1024 * 1024 && goal_length > 1024 * 1024,
        "ConsumerCheckpoint §12 requires the above-one-MiB payload row"
    );

    install_authority_record(&parent, fixture.expected.action_digest(), &record)
        .expect("the record installs");
    let bound = read_resident_authority_record(
        &parent,
        fixture.expected.action_digest(),
        &fixture.expected,
        &observation,
    )
    .expect("the resident record parses and binds");
    assert_eq!(
        bound.record_bytes(),
        encoded.len(),
        "the bounded read accepted exactly the record that was installed"
    );
    validate_terminal_relation(&parent, fixture.expected.action_digest(), &bound, &proof)
        .expect("the terminal relation holds");
}

/// The parse bound belongs to the record kind. A resident authority slot one
/// byte past the frozen bound is refused, and the refusal is a *parse* refusal
/// — no payload length was ever consulted to produce it.
#[test]
fn a_resident_record_one_byte_past_the_frozen_bound_is_refused() {
    let fixture = AuthorityFixture::new("oversize-record");
    let parent = fixture.retained();
    let (_, observation) = fixture.observe(&parent, ObservedLeafWriterClassV1::GwzWritten);

    let directory = fixture.dir();
    let name = slot_name(&fixture.expected, BaseActionSlotV1::Authority);
    let oversize = vec![0_u8; ProtocolRecordKindV1::Authority.max_bytes() + 1];
    let mut file = directory.create(name.as_str()).unwrap();
    file.write_all(&oversize).unwrap();
    file.sync_all().unwrap();
    drop(file);
    sync_directory_edge(&directory, "test oversize record").unwrap();

    read_resident_authority_record(
        &parent,
        fixture.expected.action_digest(),
        &fixture.expected,
        &observation,
    )
    .expect_err("a record past its frozen bound must not parse");
}

/// The terminal relation is the only join, and it refuses a record that binds
/// different payloads than the ones streamed.
#[test]
fn the_terminal_relation_refuses_a_record_that_describes_other_payloads() {
    let fixture = AuthorityFixture::new("relation");
    let parent = fixture.retained();
    let (proof, observation) = fixture.observe(&parent, ObservedLeafWriterClassV1::GwzWritten);
    let record = CheckedAuthorityRecordV1::issue(&observation).expect("the record issues");
    install_authority_record(&parent, fixture.expected.action_digest(), &record)
        .expect("the record installs");
    let bound = read_resident_authority_record(
        &parent,
        fixture.expected.action_digest(),
        &fixture.expected,
        &observation,
    )
    .expect("the resident record binds");

    let action = fixture.expected.action_digest();

    // Re-stream with the goal and source payloads swapped: the same two leaves,
    // the same directory, but a different terminal relation.
    let swapped_source = ExpectedPayloadV1::new(fixture.goal.clone());
    let swapped_goal = ExpectedPayloadV1::new(fixture.source.clone());
    let mut namespace = BarrierNamespaceV1::default();
    observe_streamed_payloads(
        &parent,
        fixture.expected.action_digest(),
        ObservedLeafWriterClassV1::GwzWritten,
        &swapped_source,
        &swapped_goal,
        &mut namespace,
        (0, 1),
    )
    .expect_err("a payload leaf that is not the expected content is not an exact durable proof");

    // And a relation taken against a proof of a *different* action's payloads
    // is refused rather than accepted as "close enough".
    let other = AuthorityFixture::new("relation-other");
    let other_parent = other.retained();
    let (other_proof, _) = other.observe(&other_parent, ObservedLeafWriterClassV1::GwzWritten);
    validate_terminal_relation(&parent, action, &bound, &other_proof)
        .expect_err("a proof taken under another retained directory is refused");
    validate_terminal_relation(&parent, action, &bound, &proof)
        .expect("the true relation still holds");
}

/// The E9 annotation's negative space: `MissingDurable` is a two-sided absence
/// proof that does not assert continuous absence, and an authority record has
/// no encoding for an absent payload. An absent slot is therefore a refusal.
#[test]
fn an_absent_payload_leaf_cannot_carry_authority() {
    let fixture = LeafFixture::new("absent-payload");
    let expected = reservation();
    let path = fixture.parent_path();
    let parent = retain_action(path.parent().expect("the fixture root"), "parent");
    let source = ExpectedPayloadV1::new(payload(4096, 1));
    let goal = ExpectedPayloadV1::new(payload(4096, 2));
    let mut namespace = BarrierNamespaceV1::default();
    observe_streamed_payloads(
        &parent,
        expected.action_digest(),
        ObservedLeafWriterClassV1::GwzWritten,
        &source,
        &goal,
        &mut namespace,
        (0, 1),
    )
    .expect_err("an absent payload slot cannot produce an authority-strength proof");
}

/// The E9 writer-class condition, **executed** on this host.
///
/// The condition reduces to one platform bit,
/// `FOREIGN_EXACT_DURABLE_IS_WEAKER`, and the gate that reads it is compiled on
/// every platform. So this test runs the real decision here instead of
/// asserting one arm and leaving the other to a platform this suite never
/// executes on: flipping that one constant in a scratch tree flips both the
/// production behaviour and this expectation, which is how the Windows
/// behaviour is exercised from a unix host.
#[test]
fn the_e9_writer_class_condition_is_carried_on_this_platform() {
    let fixture = AuthorityFixture::new("writer-class");
    let parent = fixture.retained();
    let source = ExpectedPayloadV1::new(fixture.source.clone());
    let goal = ExpectedPayloadV1::new(fixture.goal.clone());
    let mut namespace = BarrierNamespaceV1::default();
    let foreign = observe_streamed_payloads(
        &parent,
        fixture.expected.action_digest(),
        ObservedLeafWriterClassV1::Foreign,
        &source,
        &goal,
        &mut namespace,
        (0, 1),
    );

    if FOREIGN_EXACT_DURABLE_IS_WEAKER {
        match foreign {
            Err(CheckedFsError::Ambiguous { detail, .. }) => assert_eq!(
                detail, FOREIGN_AUTHORITY_REFUSAL,
                "the refusal must be the E9 one, not an unrelated ambiguity"
            ),
            other => panic!(
                "no handle flush is available here and E10's exact-interior barrier is the \
                 documented no-op, so a foreign leaf's ExactDurable carries no durable proof \
                 and cannot carry authority: {other:?}"
            ),
        }
    } else {
        foreign.expect(
            "the observation handle really was flushed here, so a foreign writer does not \
             weaken it",
        );
    }
}

/// The platform mapping the E9 condition rests on. Kept as its own test so the
/// behaviour test above can be re-pointed at the other platform's semantics in
/// a scratch tree without this pin silently following it.
#[test]
fn the_platform_with_the_weaker_foreign_durability_is_windows() {
    assert_eq!(
        FOREIGN_EXACT_DURABLE_IS_WEAKER,
        cfg!(windows),
        "the E9 annotation names Windows as the platform whose read-only observation handle \
         cannot flush; no other platform may claim the weaker semantics"
    );
}

/// [P1-1] The provenance guard: a proof carries the retained directory it was
/// streamed under, and the join refuses one taken under a different capability.
///
/// The observation side of this class is unrepresentable —
/// `AuthorityTransactionV1::from_streamed_proof` takes no parent, so its root
/// and parent identity can only be the streamed proof's. What remains
/// representable is the join, which still receives `parent` and `action`
/// alongside a proof, and that is what this test drives.
#[test]
fn a_proof_streamed_under_another_retained_directory_is_refused_at_the_join() {
    let left = AuthorityFixture::new("provenance-left");
    let left_parent = left.retained();
    let (left_proof, left_observation) =
        left.observe(&left_parent, ObservedLeafWriterClassV1::GwzWritten);
    let record = CheckedAuthorityRecordV1::issue(&left_observation).expect("the record issues");
    let action = left.expected.action_digest();
    install_authority_record(&left_parent, action, &record).expect("the record installs");
    let bound =
        read_resident_authority_record(&left_parent, action, &left.expected, &left_observation)
            .expect("the resident record binds");

    // A second action directory, same action digest, different durable identity.
    let right = AuthorityFixture::new("provenance-right");
    let right_parent = right.retained();
    let (right_proof, _) = right.observe(&right_parent, ObservedLeafWriterClassV1::GwzWritten);
    assert_ne!(
        left_proof.retained_parent_identity(),
        right_proof.retained_parent_identity(),
        "the two fixtures must be genuinely different durable directories"
    );

    validate_terminal_relation(&left_parent, action, &bound, &right_proof).expect_err(
        "a proof streamed under the right-hand directory cannot join a record read from the left",
    );
    validate_terminal_relation(&right_parent, action, &bound, &left_proof)
        .expect_err("nor may the left-hand proof be joined under the right-hand capability");
    validate_terminal_relation(&left_parent, action, &bound, &left_proof)
        .expect("the coherent join still holds");
}

/// Install and retire are complete durable transitions over the scheduled
/// slots, and the retirement reserve refuses a resident alias rather than
/// discovering it at the rename.
#[test]
fn the_record_installs_onto_its_slot_and_retires_onto_its_scheduled_alias() {
    let fixture = AuthorityFixture::new("lifecycle");
    let parent = fixture.retained();
    let (_, observation) = fixture.observe(&parent, ObservedLeafWriterClassV1::GwzWritten);
    let record = CheckedAuthorityRecordV1::issue(&observation).expect("the record issues");
    let action = fixture.expected.action_digest();

    install_authority_record(&parent, action, &record).expect("the record installs");
    let rows = census(&fixture.fixture.parent_path());
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert!(
        names.contains(&slot_name(&fixture.expected, BaseActionSlotV1::Authority).as_str()),
        "the active authority slot is resident after install: {names:?}"
    );
    assert!(
        !names.contains(&slot_name(&fixture.expected, BaseActionSlotV1::AuthorityScratch).as_str()),
        "the write-ahead scratch is consumed by the publish: {names:?}"
    );

    retire_authority_record(&parent, action).expect("the record retires");
    let rows = census(&fixture.fixture.parent_path());
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert!(
        names.contains(
            &slot_name(&fixture.expected, BaseActionSlotV1::RetiredAuthorityAlias).as_str()
        ),
        "the retired alias is resident after retirement: {names:?}"
    );
    assert!(
        !names.contains(&slot_name(&fixture.expected, BaseActionSlotV1::Authority).as_str()),
        "the active slot is free after retirement: {names:?}"
    );

    // A second install plus retire must refuse at the reserve, because the
    // deterministic alias is already resident.
    install_authority_record(&parent, action, &record).expect("the record reinstalls");
    retire_authority_record(&parent, action)
        .expect_err("a resident retired alias is a typed refusal at the reserve");
}

/// An installed record binds to the observation it was issued from, and to no
/// other. This is the `record.binding_validate` stage, exercised through the
/// production read rather than through a synthetic record.
#[test]
fn a_resident_record_binds_only_to_its_own_reservation_and_observation() {
    let fixture = AuthorityFixture::new("binding");
    let parent = fixture.retained();
    let (_, observation) = fixture.observe(&parent, ObservedLeafWriterClassV1::GwzWritten);
    let record = CheckedAuthorityRecordV1::issue(&observation).expect("the record issues");
    install_authority_record(&parent, fixture.expected.action_digest(), &record)
        .expect("the record installs");

    let other = AuthorityFixture::new("binding-other");
    let other_parent = other.retained();
    let (_, other_observation) =
        other.observe(&other_parent, ObservedLeafWriterClassV1::GwzWritten);

    read_resident_authority_record(
        &parent,
        fixture.expected.action_digest(),
        &fixture.expected,
        &other_observation,
    )
    .expect_err("a record must not bind to an observation taken over other payloads");

    read_resident_authority_record(
        &parent,
        fixture.expected.action_digest(),
        &fixture.expected,
        &observation,
    )
    .expect("the record binds to its own observation");
}
