//! R2-D Phase 3 Step 3.3 — the coordinator execution glue, driven against a
//! real target.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 3.3 ("schedule +
//! `AdmittedActionV1` binding so that replacement/removal executes only after an
//! admitted action and an owner-private coherent authority observation; writers
//! receive only the opaque retained-parent proof");
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §8 (:239-240) and §9 (:264-266).
//!
//! Every row runs the production sequence — schedule, admit through the Phase-1
//! owner in its own lease session, then execute in the next — against a real
//! lease, the sealed catalog owner and a real workspace. The one thing no test
//! here does is convert a consumer: plan §4 Step 3.3 wires machinery and leaves
//! conversion to R2-E, so `entry.rs` is untouched and every assertion is about
//! the seam.

use std::fs;

use super::execution::{
    AdmittedCheckedActionV1, CheckedExecutionPlanV1, ScheduledCheckedActionV1,
    schedule_checked_action,
};
use super::{
    CheckedActionOperationV1, CheckedActionOwnerV1, CheckedActionRequestV1, CheckedLeafFactV1,
    CheckedManagedActionV1, synthetic_leaf_request,
};
use crate::checked_artifact::bootstrap::tests_provider::{Fixture, TargetVariantV1, with_catalog};
use crate::checked_artifact::bootstrap::{
    ManagedParentBootstrapOwnerV1, ManagedParentPlanV1, ManagedParentPurpose,
    RetainedManagedParentProviderV1,
};
use crate::checked_artifact::capability::{AsciiComponent, CheckedFsError, PreCatalogRootKindV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, CheckedAuthorityObservationV1,
    CleanupAliasSetV1, DurableLeafFingerprintV1, RequestOwnerBindingV1,
    synthetic_authority_observation,
};

const WORKSPACE: &str = "ws_r2d_step_33";

fn managed_action() -> CheckedManagedActionV1 {
    CheckedManagedActionV1::for_merge_start(WORKSPACE).expect("the merge-start owner is valid")
}

fn owner() -> CheckedActionOwnerV1 {
    CheckedActionOwnerV1::for_merge_start(WORKSPACE).expect("the merge-start owner is valid")
}

fn component(bytes: &[u8]) -> AsciiComponent {
    AsciiComponent::parse(bytes).expect("a fixed test component is ASCII")
}

/// A leaf request of the given operation, over a fixed workspace-relative path.
/// The expected and goal digests [`leaf_request`] uses for each operation, so an
/// authority observation built for that request binds the same leaf facts the
/// action digest was derived over.
fn leaf_digests(operation: CheckedActionOperationV1) -> ([u8; 32], [u8; 32]) {
    match operation {
        CheckedActionOperationV1::Observe => ([5; 32], [5; 32]),
        CheckedActionOperationV1::Remove => ([7; 32], [7; 32]),
        _ => ([9; 32], [9; 32]),
    }
}

fn leaf_request(operation: CheckedActionOperationV1) -> CheckedActionRequestV1 {
    // Each operation's legal expected/goal shape, per `CheckedActionRequestV1`'s
    // own leaf validator: an observation asserts one fact, a replacement names an
    // exact goal, and a removal names an exact expected and a missing goal.
    let (expected, goal) = match operation {
        CheckedActionOperationV1::Observe => (
            CheckedLeafFactV1::Exact {
                sha256: [5; 32],
                length: 7,
            },
            CheckedLeafFactV1::Exact {
                sha256: [5; 32],
                length: 7,
            },
        ),
        CheckedActionOperationV1::Remove => (
            CheckedLeafFactV1::Exact {
                sha256: [7; 32],
                length: 11,
            },
            CheckedLeafFactV1::Missing,
        ),
        _ => (
            CheckedLeafFactV1::Missing,
            CheckedLeafFactV1::Exact {
                sha256: [9; 32],
                length: 13,
            },
        ),
    };
    synthetic_leaf_request(
        &owner(),
        operation,
        PreCatalogRootKindV1::Workspace,
        vec![component(b"gwz.conf"), component(b"lock")],
        expected,
        goal,
        0,
    )
    .expect("the synthetic leaf request is valid")
}

fn schedule_of(request: &CheckedActionRequestV1) -> Box<ScheduledCheckedActionV1> {
    match schedule_checked_action(request, None).expect("the request schedules") {
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled,
        CheckedExecutionPlanV1::ProofOnly => panic!("this request must reserve capacity"),
    }
}

/// A fixture with `.gwz` present, so the merge-start purposes have their
/// declared prefix.
fn new_fixture(label: &str) -> Fixture {
    let fixture = Fixture::new(&format!("coord-{label}"));
    fixture.prepare_prefix(TargetVariantV1::Workspace, ".gwz");
    fixture
}

/// The preflight session: the plan a managed action derives against this target.
fn managed_plan(fixture: &Fixture, action: &CheckedManagedActionV1) -> ManagedParentPlanV1 {
    with_catalog(fixture, TargetVariantV1::Workspace, |catalog| {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        ManagedParentBootstrapOwnerV1::new(&provider).preflight_checked(action)
    })
    .expect("the managed prefix must be observable")
}

/// The admission session. The Phase-1 owner consumes the catalog, so this is a
/// lease session of its own — the production shape, not a test contrivance.
fn admit(fixture: &Fixture, scheduled: &ScheduledCheckedActionV1) -> AdmittedCheckedActionV1 {
    with_catalog(fixture, TargetVariantV1::Workspace, |catalog| {
        scheduled.admit(catalog)
    })
    .expect("the scheduled action must admit")
}

/// A coherent authority observation issued against `reservation`, whose streamed
/// provenance is the retained action directory of `streamed_under`.
///
/// The two arguments are the whole point. `synthetic_authority_observation` is
/// the test-only door onto R1's issuer, and `retained_parent_identity` is the
/// field production sets from the capability the payloads were actually streamed
/// through (`observe_streamed_payloads` mints it from the retained directory, and
/// `AuthorityTransactionV1` carries it forward untouched). Passing a *different*
/// admitted action's directory identity therefore reproduces exactly the
/// production shape "streamed under B, issued against A" — the Step-3.3 review's
/// [P1-1] — rather than the weaker "issued against a different reservation".
fn authority_observation_streamed_under(
    fixture: &Fixture,
    reservation: &ActionCapacityReservationV1,
    streamed_under: &AdmittedCheckedActionV1,
    operation: CheckedActionOperationV1,
) -> CheckedAuthorityObservationV1 {
    let provenance = streamed_under.admitted().directory_identity().clone();
    let (expected_sha256, goal_sha256) = leaf_digests(operation);
    with_catalog(fixture, TargetVariantV1::Workspace, |catalog| {
        let observed = catalog.observe_managed_prefix(&[component(b".gwz")])?;
        let facts = observed
            .at(1)
            .expect("the private root is a retained ancestor");
        synthetic_authority_observation(
            reservation,
            facts.path().clone(),
            provenance.clone(),
            DurableLeafFingerprintV1::new(provenance.clone(), 5, [3; 32]),
            expected_sha256,
            goal_sha256,
        )
        .map_err(|_| CheckedFsError::ambiguous("test authority observation", "not issuable"))
    })
    .expect("a coherent observation is issuable for this reservation")
}

/// The honest case: issued against this action's reservation *and* streamed under
/// this action's own retained directory.
fn authority_observation(
    fixture: &Fixture,
    admitted: &AdmittedCheckedActionV1,
    operation: CheckedActionOperationV1,
) -> CheckedAuthorityObservationV1 {
    authority_observation_streamed_under(
        fixture,
        admitted.admitted().reservation(),
        admitted,
        operation,
    )
}

#[test]
fn an_observation_schedules_proof_only_and_reserves_nothing() {
    let request = leaf_request(CheckedActionOperationV1::Observe);
    assert!(matches!(
        schedule_checked_action(&request, None).expect("an observation schedules"),
        CheckedExecutionPlanV1::ProofOnly
    ));
}

#[test]
fn a_scheduled_replacement_binds_the_action_its_own_reservation_admitted() {
    let fixture = new_fixture("admit");
    let request = leaf_request(CheckedActionOperationV1::Replace);
    let scheduled = schedule_of(&request);
    let admitted = admit(&fixture, &scheduled);

    assert_eq!(admitted.admitted().reservation(), scheduled.reservation());
    assert_eq!(
        admitted.action_digest(),
        scheduled.reservation().action_digest()
    );
}

#[test]
fn a_parent_only_action_bootstraps_its_managed_parents_through_the_provider() {
    let fixture = new_fixture("bootstrap");
    let action = managed_action();
    let plan = managed_plan(&fixture, &action);
    let scheduled = match schedule_checked_action(action.checked(), Some(&plan))
        .expect("the managed action schedules")
    {
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled,
        CheckedExecutionPlanV1::ProofOnly => panic!("a missing suffix must reserve capacity"),
    };
    let admitted = admit(&fixture, &scheduled);

    let facades = with_catalog(&fixture, TargetVariantV1::Workspace, |catalog| {
        admitted.bootstrap_managed_parents(&catalog, &action)
    })
    .expect("the managed parents must bootstrap through the coordinator");

    // [P2-1]: the coordinator's only egress is the facade set — no row, and
    // therefore no `path()`, reaches a caller of this surface.
    assert_eq!(facades.len(), 2);
    assert!(fixture.path().join(".gwz/merge").is_dir());
    assert!(fixture.path().join(".gwz/stash/bundles").is_dir());
}

#[test]
fn a_managed_request_for_another_checked_action_is_refused() {
    let fixture = new_fixture("foreign-managed");
    let action = managed_action();
    let plan = managed_plan(&fixture, &action);
    let scheduled = match schedule_checked_action(action.checked(), Some(&plan)).unwrap() {
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled,
        CheckedExecutionPlanV1::ProofOnly => panic!("a missing suffix must reserve capacity"),
    };
    let admitted = admit(&fixture, &scheduled);

    // A different workspace id is a different owner, so a different action.
    let other = CheckedManagedActionV1::for_merge_start("ws_r2d_step_33_other").unwrap();
    let refused = with_catalog(&fixture, TargetVariantV1::Workspace, |catalog| {
        admitted.bootstrap_managed_parents(&catalog, &other)
    })
    .is_err();

    assert!(
        refused,
        "a managed request from another checked action must be refused"
    );
    assert!(!fixture.path().join(".gwz/merge").exists());
}

#[test]
fn a_replacement_takes_a_write_authority_only_from_its_own_observation() {
    let fixture = new_fixture("authority");
    let request = leaf_request(CheckedActionOperationV1::Replace);
    let scheduled = schedule_of(&request);
    let admitted = admit(&fixture, &scheduled);

    let coherent = authority_observation(&fixture, &admitted, CheckedActionOperationV1::Replace);
    let authority = admitted
        .authorize_write(&coherent)
        .expect("an observation bound to this action must authorize the write");
    assert_eq!(authority.action(), admitted.action_digest());
    assert_ne!(authority.record_id(), [0; 32]);

    // The same observation shape *issued against* a different action's
    // reservation is coherent in itself and still refused: the pairing gate.
    let other = leaf_request(CheckedActionOperationV1::Remove);
    let other_admitted = admit(&fixture, &schedule_of(&other));
    let foreign =
        authority_observation(&fixture, &other_admitted, CheckedActionOperationV1::Remove);
    assert!(
        admitted.authorize_write(&foreign).is_err(),
        "an observation issued against another admitted action must not authorize"
    );
}

/// **Step-3.3 review [P3-4].** The pairing gate, one varied input at a time.
///
/// The observation is streamed under the target's *own* retained directory in
/// every row, so the provenance gate cannot fire and each refusal is
/// attributable to the pairing alone — asserted by its detail string, not just
/// by `is_err()`. A strictly one-*field* variation is not constructible:
/// `record_digest` is a SHA-256 over action digest + owner binding + schedule,
/// so moving any input moves it too. Varying one input at a time is the
/// available granularity, and the three inputs are exactly the three the digest
/// covers.
#[test]
fn the_pairing_gate_refuses_each_varied_reservation_input() {
    let fixture = new_fixture("pairing");
    let request = leaf_request(CheckedActionOperationV1::Replace);
    let scheduled = schedule_of(&request);
    let admitted = admit(&fixture, &scheduled);
    let base = scheduled.reservation();

    // A retry of the same action with a different barrier count: same action
    // digest, same owner, different schedule — the stale-schedule row.
    let retry_schedule =
        ActionScheduleV1::try_new(1, Vec::new(), CleanupAliasSetV1::all()).unwrap();
    assert_ne!(retry_schedule.digest(), base.schedule().digest());

    let varied = [
        (
            "another action digest",
            ActionCapacityReservationV1::new(
                ActionDigestV1::new([0xAB; 32]),
                base.request_owner_binding(),
                base.schedule().clone(),
            ),
        ),
        (
            "a stale schedule for the same action",
            ActionCapacityReservationV1::new(
                base.action_digest(),
                base.request_owner_binding(),
                retry_schedule,
            ),
        ),
        (
            "another owner binding",
            ActionCapacityReservationV1::new(
                base.action_digest(),
                RequestOwnerBindingV1::new([0xCD; 32]),
                base.schedule().clone(),
            ),
        ),
    ];

    for (label, reservation) in varied {
        let observation = authority_observation_streamed_under(
            &fixture,
            &reservation,
            &admitted,
            CheckedActionOperationV1::Replace,
        );
        match admitted.authorize_write(&observation) {
            Err(CheckedFsError::Ambiguous { detail, .. }) => assert_eq!(
                detail, "the authority observation was issued against another admitted action",
                "{label}: refused, but not by the pairing gate"
            ),
            Err(other) => panic!("{label}: unexpected refusal {other:?}"),
            Ok(_) => panic!("{label}: must not authorize"),
        }
    }
}

/// **Step-3.3 review [P1-1].** The cross-action shape the pairing gate alone does
/// not see: an observation *streamed under action B's retained action directory*
/// but *issued against action A's reservation*.
///
/// It passes `matches_reservation` by construction — R1's issuer copies all four
/// binding fields from the reservation argument — so only a check against the
/// observation's own **observed provenance** can refuse it. `request_owner_binding`
/// does not help: it is per merge owner, and both actions here share one owner,
/// which is what makes the case realizable rather than hypothetical.
#[test]
fn an_observation_streamed_under_another_action_must_not_authorize() {
    let fixture = new_fixture("cross-action");
    let target = admit(
        &fixture,
        &schedule_of(&leaf_request(CheckedActionOperationV1::Replace)),
    );
    let other = admit(
        &fixture,
        &schedule_of(&leaf_request(CheckedActionOperationV1::Remove)),
    );
    assert_ne!(
        target.admitted().directory_identity(),
        other.admitted().directory_identity(),
        "two admitted actions must have distinct retained directories"
    );

    // Issued against the target's reservation; streamed under the other action.
    let smuggled = authority_observation_streamed_under(
        &fixture,
        target.admitted().reservation(),
        &other,
        CheckedActionOperationV1::Replace,
    );

    assert!(
        target.authorize_write(&smuggled).is_err(),
        "an observation streamed under another action's retained directory must not \
         authorize a write on this one"
    );
}

#[test]
fn a_removal_takes_a_write_authority_but_a_parent_only_action_does_not() {
    let fixture = new_fixture("operations");
    let removal = leaf_request(CheckedActionOperationV1::Remove);
    let scheduled = schedule_of(&removal);
    let admitted = admit(&fixture, &scheduled);
    let coherent = authority_observation(&fixture, &admitted, CheckedActionOperationV1::Remove);
    assert!(admitted.authorize_write(&coherent).is_ok());

    let parent_only = new_fixture("parent-only");
    let action = managed_action();
    let plan = managed_plan(&parent_only, &action);
    let parent_scheduled = match schedule_checked_action(action.checked(), Some(&plan)).unwrap() {
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled,
        CheckedExecutionPlanV1::ProofOnly => panic!("a missing suffix must reserve capacity"),
    };
    let parent_admitted = admit(&parent_only, &parent_scheduled);
    let parent_observation = authority_observation(
        &parent_only,
        &parent_admitted,
        CheckedActionOperationV1::ParentOnly,
    );

    assert!(
        parent_admitted
            .authorize_write(&parent_observation)
            .is_err(),
        "a parent-only action writes no leaf and must take no leaf authority"
    );
}

#[test]
fn the_writer_facade_revalidates_its_retained_proof_and_refuses_drift() {
    let fixture = new_fixture("facade");
    let action = managed_action();
    let plan = managed_plan(&fixture, &action);
    let scheduled = match schedule_checked_action(action.checked(), Some(&plan)).unwrap() {
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled,
        CheckedExecutionPlanV1::ProofOnly => panic!("a missing suffix must reserve capacity"),
    };
    let admitted = admit(&fixture, &scheduled);
    let facades = with_catalog(&fixture, TargetVariantV1::Workspace, |catalog| {
        admitted.bootstrap_managed_parents(&catalog, &action)
    })
    .expect("the managed parents must bootstrap");
    assert_eq!(facades.len(), 2);
    let store = facades
        .iter()
        .find(|facade| facade.purpose() == ManagedParentPurpose::MergeStore)
        .expect("the merge store has a facade");

    let revalidated = with_catalog(&fixture, TargetVariantV1::Workspace, |catalog| {
        store.revalidate(&catalog)
    })
    .expect("an unchanged managed parent revalidates");
    assert_eq!(revalidated.purpose(), ManagedParentPurpose::MergeStore);

    // Substitute the managed parent for a different directory of the same name:
    // the proof's identity no longer matches, so no write may proceed through it.
    let merge = fixture.path().join(".gwz/merge");
    fs::remove_dir_all(&merge).expect("the managed parent is removable");
    fs::create_dir(&merge).expect("a substitute is creatable");

    let refused = with_catalog(&fixture, TargetVariantV1::Workspace, |catalog| {
        store.revalidate(&catalog)
    })
    .is_err();
    assert!(
        refused,
        "a substituted managed parent must not revalidate its retained proof"
    );
}
