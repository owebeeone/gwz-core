//! Failing production-shaped tests for the physical admission driver.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Phase 1 Step 1.1;
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §7 (:203-224) nine-step durable sequence
//! and §6 (:199-201) row classification; `GwzM5-8R2DInterfaceFreeze.md` §3.1
//! (the frozen driver seam); `GwzM5-8R2CCatalogBootstrapAmendment.md` §7 owner
//! boundaries.
//!
//! These are red tests, not freeze pins. Every case drives the frozen seam on a
//! real workspace through the sealed catalog owner, so a failure naming the
//! Step 1.2 refusal is the expected result today, and a failure naming anything
//! else is a fixture defect. Step 1.2 makes them pass without editing them.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::checked_artifact::bootstrap::{WorkspaceRuntimeLease, try_acquire_workspace_runtime};
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{
    ActionDigestV1, ActionDirectoryAdmissionV1, ActionScheduleV1, ActionSlotV1, BaseActionSlotV1,
    CatalogNameClassificationV1, CleanupAliasSetV1, InfrastructureSlotV1, MAX_ACTIVE_ACTION_DIRS,
    ManagedBootstrapInputV1, RequestOwnerBindingV1, RootEntryNameV1, read_bounded_record,
};

/// The typed refusal the frozen seam answers until Step 1.2 lands
/// (`admission/mod.rs`, interface freeze §3.1 "Body today").
const ADMISSION_FACT: &str = "action admission";
const DRIVER_UNAVAILABLE: &str = "physical admission driver is implemented in R2-D phase 1";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// One real workspace, mirroring the R2-C2 interruption/restart fixture at
/// `catalog/bootstrap/tests.rs:19-44`.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2c3-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn catalog_root(&self) -> PathBuf {
        self.root
            .join(CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::Workspace))
    }

    fn slot(&self, slot: InfrastructureSlotV1) -> PathBuf {
        self.catalog_root().join(slot.name())
    }

    fn action_directory(&self, reservation: &ActionCapacityReservationV1) -> PathBuf {
        self.catalog_root()
            .join(RootEntryNameV1::ActiveAction(reservation.action_digest()).name())
    }

    fn catalog_children(&self) -> Vec<String> {
        let mut names = fs::read_dir(self.catalog_root())
            .expect("the recovered catalog root must exist")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One complete production-shaped attempt: acquire the workspace runtime lease,
/// recover the catalog through its sole sealed owner, and drive the frozen
/// admission seam once. The lease is released on return, so successive calls
/// are fresh-process restarts.
fn attempt(
    fixture: &Fixture,
    expected: &ActionCapacityReservationV1,
) -> Result<AdmittedActionV1, CheckedFsError> {
    let runtime = runtime_lease(fixture);
    let retained = recover_or_create(runtime.catalog_mutation_lease())
        .expect("the sealed catalog owner must recover a complete catalog");
    ActionAdmissionOwnerV1::from_retained_catalog(retained).resume_or_admit(expected)
}

/// Recovers the catalog without admitting, so a test can pin the state that
/// precedes the first admission mutation.
fn recover_catalog(fixture: &Fixture) {
    recover_or_create(runtime_lease(fixture).catalog_mutation_lease())
        .expect("the sealed catalog owner must recover a complete catalog");
}

fn runtime_lease(fixture: &Fixture) -> WorkspaceRuntimeLease {
    try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease")
}

fn driver_is_unimplemented(error: &CheckedFsError) -> bool {
    matches!(
        error,
        CheckedFsError::Ambiguous { fact, detail }
            if *fact == ADMISSION_FACT && detail.as_str() == DRIVER_UNAVAILABLE
    )
}

/// Unwraps a handoff, separating "Step 1.2 is not written yet" from a fixture
/// defect so a red run proves it failed for the right reason.
#[track_caller]
fn admitted(result: Result<AdmittedActionV1, CheckedFsError>, property: &str) -> AdmittedActionV1 {
    match result {
        Ok(admitted) => admitted,
        Err(error) if driver_is_unimplemented(&error) => panic!(
            "R2-D Step 1.2 must implement the admission driver before this holds: {property}; \
             the frozen seam still answers {ADMISSION_FACT}: {DRIVER_UNAVAILABLE}"
        ),
        Err(error) => {
            panic!("admission refused for an unexpected reason, not {property}: {error:?}")
        }
    }
}

/// Asserts a stop. The unimplemented refusal is rejected explicitly, so a
/// negative case cannot pass today on the strength of the Step 1.2 sentinel.
#[track_caller]
fn stopped(result: Result<AdmittedActionV1, CheckedFsError>, property: &str) {
    match result {
        Ok(_) => panic!("admission must stop rather than hand back an action: {property}"),
        Err(error) if driver_is_unimplemented(&error) => panic!(
            "R2-D Step 1.2 must implement the admission driver before this holds: {property}; \
             the frozen seam still answers {ADMISSION_FACT}: {DRIVER_UNAVAILABLE}"
        ),
        Err(_) => {}
    }
}

/// Asserts a stop carrying exactly the typed refusal named, so a capacity case
/// cannot pass on the strength of some unrelated ambiguity.
#[track_caller]
fn stopped_with(
    result: Result<AdmittedActionV1, CheckedFsError>,
    expected_fact: &str,
    expected_detail: &str,
) {
    match result {
        Ok(_) => panic!("admission must stop rather than hand back an action: {expected_detail}"),
        Err(CheckedFsError::Ambiguous { fact, detail })
            if fact == expected_fact && detail.as_str() == expected_detail => {}
        Err(error) => panic!("admission stopped for the wrong reason: {error:?}"),
    }
}

/// A coordinator-derived reservation whose schedule is non-trivial in every
/// capacity family ConsumerCheckpoint §7 (:222-224) names.
fn reservation(action: u8, barriers: usize) -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([action; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            barriers,
            vec![ManagedBootstrapInputV1::new([3; 32], 2).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

/// The one resident reservation the sequence leaves inside the deterministic
/// final action directory, read through the frozen bounded record reader. Also
/// pins "no extra children" and the frozen action-slot name grammar.
fn resident_reservation(
    fixture: &Fixture,
    expected: &ActionCapacityReservationV1,
) -> ActionCapacityReservationV1 {
    let directory = fixture.action_directory(expected);
    let mut children = fs::read_dir(&directory)
        .expect("the deterministic final action directory must exist")
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        children.len(),
        1,
        "the final action directory must carry the resident reservation and no extra children"
    );
    let name = children.pop().expect("one child");
    let text = name.to_string_lossy();
    assert!(
        matches!(
            ActionSlotV1::parse(expected.action_digest(), text.as_bytes()),
            CatalogNameClassificationV1::Valid(_)
        ),
        "the resident reservation name must be in the frozen action slot grammar: {text}"
    );
    read_bounded_record(fs::File::open(directory.join(&name)).unwrap())
        .expect("the resident reservation must be a canonical bounded capacity record")
}

/// The durable admission record, whose `Idle`/`Preparing` states are the
/// transition kernel of ConsumerCheckpoint §7 steps 3 and 7.
fn admission_record(fixture: &Fixture) -> ActionDirectoryAdmissionV1 {
    let path = fixture.slot(InfrastructureSlotV1::ActionAdmissionActive);
    read_bounded_record(
        fs::File::open(&path).expect("the durable admission record must survive the sequence"),
    )
    .expect("the admission record must be a canonical bounded record")
}

/// The five capacity families §7 (:223-224) requires before the first action
/// mutation, derived from the persisted schedule through the frozen slot
/// grammar alone.
fn reserved_capacity_families(
    schedule: &ActionScheduleV1,
) -> Vec<(&'static str, Vec<ActionSlotV1>)> {
    use ActionSlotV1 as Slot;
    let barriers = 0..schedule.barrier_count() as u8;
    vec![
        (
            "barrier",
            barriers
                .clone()
                .flat_map(|o| [Slot::BarrierIntentActive(o), Slot::BarrierIntentRetired(o)])
                .collect(),
        ),
        (
            "managed generation",
            (0..schedule.generation_count() as u8)
                .flat_map(|g| {
                    [
                        Slot::BootstrapIntentActive(g),
                        Slot::BootstrapIntentRetired(g),
                    ]
                })
                .collect(),
        ),
        (
            "marker",
            (0..schedule.component_count() as u8)
                .map(Slot::RetiredBootstrapMarker)
                .collect(),
        ),
        (
            "cleanup",
            vec![Slot::Base(BaseActionSlotV1::CleanupWorklist)],
        ),
        (
            "terminal retirement",
            [
                BaseActionSlotV1::RetiredSourceAlias,
                BaseActionSlotV1::RetiredGoalAlias,
                BaseActionSlotV1::RetiredAuthorityAlias,
            ]
            .into_iter()
            .map(Slot::Base)
            .chain(barriers.map(Slot::RetiredRoamingAnchorAlias))
            .collect(),
        ),
    ]
}

/// Step 1.1 (a), resume half. The bounded global catalog lookup finds the
/// derived action/owner binding already published and resumes that exact
/// action instead of admitting a second one (ConsumerCheckpoint §7 :205-207).
#[test]
fn a_global_catalog_lookup_resumes_the_exact_existing_action() {
    let fixture = Fixture::new("resume-exact");
    let expected = reservation(0x11, 2);

    let first = admitted(
        attempt(&fixture, &expected),
        "a first admission publishes the action",
    );
    let settled = fixture.catalog_children();

    let resumed = admitted(
        attempt(&fixture, &expected),
        "a restart resumes the exact existing action",
    );

    assert_eq!(
        resumed, first,
        "resume must hand back the same admitted action, not a second admission"
    );
    assert_eq!(resumed.reservation(), &expected);
    assert_eq!(
        fixture.catalog_children(),
        settled,
        "resuming an exact existing action must not mutate the catalog"
    );
}

/// Step 1.1 (a), stop half. A second reservation that derives the same final
/// action name but different capacity is a conflict, and any conflicting or
/// ambiguous action stops (ConsumerCheckpoint §7 :206-207).
#[test]
fn a_conflicting_action_at_the_same_final_name_stops_admission() {
    let fixture = Fixture::new("conflict-stops");
    let expected = reservation(0x22, 2);
    let conflicting = reservation(0x22, 3);
    assert_eq!(conflicting.action_digest(), expected.action_digest());
    assert_ne!(conflicting.record_digest(), expected.record_digest());

    admitted(
        attempt(&fixture, &expected),
        "a first admission publishes the action",
    );
    let settled = fixture.catalog_children();

    stopped(
        attempt(&fixture, &conflicting),
        "a conflicting reservation at the same final action name stops",
    );

    assert_eq!(
        fixture.catalog_children(),
        settled,
        "a stopped admission must leave the catalog untouched"
    );
    assert_eq!(
        resident_reservation(&fixture, &expected),
        expected,
        "a stopped admission must not rewrite the resident reservation of the exact action"
    );
}

/// Step 1.1 (b). The exact durable sequence
/// `Idle -> Preparing -> staging dir -> resident reservation -> no-replace
/// publish -> Preparing -> Idle -> reobserve`, pinned by the durable evidence
/// each step must leave behind (ConsumerCheckpoint §7 :209-221).
#[test]
fn the_durable_admission_sequence_settles_idle_with_the_published_action() {
    let fixture = Fixture::new("durable-sequence");
    let expected = reservation(0x33, 2);

    // Steps 1-2 derive the plan and schedule from read-only observations, so a
    // recovered catalog carries no admission record, scratch, or staging yet.
    recover_catalog(&fixture);
    for slot in [
        InfrastructureSlotV1::ActionAdmissionActive,
        InfrastructureSlotV1::ActionAdmissionScratch,
        InfrastructureSlotV1::ActionAdmissionStaging,
    ] {
        assert!(
            !fixture.slot(slot).exists(),
            "deriving the plan must not mutate the catalog: {}",
            slot.name()
        );
    }
    assert!(!fixture.action_directory(&expected).exists());

    let handoff = admitted(
        attempt(&fixture, &expected),
        "the nine-step durable sequence completes",
    );

    // Steps 3 and 7: the transition kernel persisted `Preparing` and returned
    // the record to `Idle` before any handoff was issued.
    assert_eq!(
        admission_record(&fixture),
        ActionDirectoryAdmissionV1::idle(),
        "the durable admission record must settle back to idle"
    );
    // Steps 4 and 6: the one indexed staging directory was consumed by the
    // no-replace publish into the deterministic final name.
    assert!(
        !fixture
            .slot(InfrastructureSlotV1::ActionAdmissionStaging)
            .exists(),
        "the staging action directory must be consumed by the publish"
    );
    assert!(
        fixture.action_directory(&expected).is_dir(),
        "the action must be published under the deterministic final name"
    );
    // Steps 5, 8 and 9: the flushed resident reservation survives the
    // reobservation and is exactly what the handoff carries.
    assert_eq!(resident_reservation(&fixture, &expected), expected);
    assert_eq!(handoff.reservation(), &expected);
}

/// Step 1.1 (c). `AdmittedActionV1` is obtainable only from idle + missing
/// staging + exact final reservation with no extra children
/// (ConsumerCheckpoint §7 :220-221; amendment §7 owner boundaries).
#[test]
fn an_admitted_action_requires_idle_missing_staging_and_an_exact_final_reservation() {
    let fixture = Fixture::new("exact-handoff");
    let expected = reservation(0x44, 2);

    let handoff = admitted(
        attempt(&fixture, &expected),
        "the exact preconditions issue the handoff",
    );
    assert_eq!(handoff.reservation(), &expected);
    assert_eq!(
        admission_record(&fixture),
        ActionDirectoryAdmissionV1::idle()
    );
    assert!(
        !fixture
            .slot(InfrastructureSlotV1::ActionAdmissionStaging)
            .exists()
    );
    assert_eq!(resident_reservation(&fixture, &expected), expected);

    // One extra child in the final action directory withdraws the handoff.
    fs::write(
        fixture.action_directory(&expected).join("unexpected-child"),
        b"",
    )
    .unwrap();

    stopped(
        attempt(&fixture, &expected),
        "an extra child in the final action directory withdraws the handoff",
    );
}

/// Step 1.1 (d). Retry reuses the same names and capacity and never chooses a
/// nonce: every catalog child stays inside the frozen deterministic grammar
/// and the one action row is the derived final name (§7 :222-223).
#[test]
fn a_retried_admission_reuses_the_deterministic_names_and_never_chooses_a_nonce() {
    let fixture = Fixture::new("no-nonce");
    let expected = reservation(0x55, 2);
    let final_name = RootEntryNameV1::ActiveAction(expected.action_digest()).name();

    let mut runs = Vec::new();
    for _ in 0..3 {
        let handoff = admitted(
            attempt(&fixture, &expected),
            "every retry admits the same action",
        );
        runs.push((handoff, fixture.catalog_children()));
    }
    let (first, first_children) = &runs[0];
    for (handoff, children) in &runs[1..] {
        assert_eq!(handoff, first, "a retry must not admit a different action");
        assert_eq!(
            children, first_children,
            "a retry must reuse the same names rather than choose a fresh nonce"
        );
    }

    let mut actions = Vec::new();
    for name in first_children {
        match RootEntryNameV1::parse(name.as_bytes()) {
            CatalogNameClassificationV1::Valid(RootEntryNameV1::ActiveAction(_)) => {
                actions.push(name.clone());
            }
            CatalogNameClassificationV1::Valid(RootEntryNameV1::Infrastructure(_)) => {}
            _ => panic!("admission left a name outside the frozen catalog grammar: {name}"),
        }
    }
    assert_eq!(
        actions,
        vec![final_name],
        "retry must reuse the one deterministic final action name"
    );
    assert_eq!(
        resident_reservation(&fixture, &expected),
        expected,
        "retry must reuse the same capacity"
    );
}

/// Round-2 remediation of the settled State-axis [P1-1]. ConsumerCheckpoint §7's
/// capacity rule and the C-3 widening's zero-headroom premise ("capacity refusal
/// fires first") require that a full catalog root refuse a **new** admission
/// rather than publish a 65th active-action row.
///
/// The row budget is zero-headroom by construction — `MAX_ROOT_ENTRIES` is
/// exactly `MAX_INFRASTRUCTURE_ENTRIES + MAX_ACTIVE_ACTION_DIRS` — so an
/// over-published row is not a recoverable overflow: `interior::observe` refuses
/// the root from then on, no admission edge can remove an action row, and every
/// sealed path including `recover_or_create` runs through that observation. The
/// catalog would be durably unobservable.
///
/// The distinction this case pins is new-versus-resume. Refusal applies only to
/// admitting a new action; an action already published stays resumable at full
/// capacity, the catalog stays recoverable, and once a row leaves the root the
/// previously refused admission succeeds.
#[test]
fn a_full_catalog_root_refuses_a_new_admission_and_still_resumes_and_recovers() {
    let fixture = Fixture::new("capacity-full");
    let resident = reservation(0x77, 2);
    let overflowing = reservation(0x78, 2);
    assert_ne!(resident.action_digest(), overflowing.action_digest());

    // One genuinely admitted action, then the root filled to the frozen budget
    // with grammar-legal action rows. `exact_row` classifies a root child by
    // name alone, so a bare directory in the frozen `action-<hex>-v1` grammar is
    // exactly what the census counts.
    let resident_handoff = admitted(
        attempt(&fixture, &resident),
        "a first admission publishes the action",
    );
    let filler = (0..u8::try_from(MAX_ACTIVE_ACTION_DIRS - 1).unwrap())
        .map(|index| RootEntryNameV1::ActiveAction(ActionDigestV1::new([index; 32])).name())
        .collect::<Vec<_>>();
    for name in &filler {
        fs::create_dir(fixture.catalog_root().join(name)).unwrap();
    }
    let full = fixture.catalog_children();
    assert_eq!(
        full.iter().filter(|name| is_action_row(name)).count(),
        MAX_ACTIVE_ACTION_DIRS,
        "the fixture must fill the root to exactly the frozen active-action budget: {full:?}"
    );

    // The 65th admission refuses cleanly, before any durable write.
    stopped_with(
        attempt(&fixture, &overflowing),
        ADMISSION_FACT,
        "the catalog root already holds the frozen active-action budget",
    );
    assert_eq!(
        fixture.catalog_children(),
        full,
        "a refused admission must leave the catalog root untouched"
    );
    assert!(
        !fixture.action_directory(&overflowing).exists(),
        "a refused admission must not publish its action row"
    );

    // The catalog stays observable through the sealed recovery path, and the
    // already-published action stays resumable at full capacity: the refusal
    // gates new admissions only, and `admit` runs before it.
    recover_catalog(&fixture);
    assert_eq!(
        admitted(
            attempt(&fixture, &resident),
            "an existing action still resumes at full capacity",
        ),
        resident_handoff,
    );
    assert_eq!(
        fixture.catalog_children(),
        full,
        "resuming at full capacity must not mutate the catalog"
    );

    // Retiring one row (Phase 4's edge, performed here as the out-of-band state
    // it produces) restores headroom, and the previously refused admission
    // completes.
    fs::remove_dir(fixture.catalog_root().join(&filler[0])).unwrap();
    let overflow_handoff = admitted(
        attempt(&fixture, &overflowing),
        "the refused admission succeeds once a row is retired",
    );
    assert_eq!(overflow_handoff.reservation(), &overflowing);
    assert!(fixture.action_directory(&overflowing).is_dir());
    assert_eq!(
        admission_record(&fixture),
        ActionDirectoryAdmissionV1::idle(),
        "the retried admission must settle the record back to idle"
    );
}

/// The commit-point half of the same remediation. The driver's gate gets the
/// *new*-admission case, but an admission already `Preparing` when other
/// admissions filled the root never re-enters that arm — it drives `Preparing`
/// straight to `PublishStagingAction`. The `AdmissionCatalogInterior`
/// destination recheck is what closes that path, re-proving the frozen
/// active-action bound inside the acquisition window, where it is race-free.
///
/// The in-flight state is reconstructed on disk rather than fault-injected:
/// publish an action, move its published directory back onto the staging name
/// (an exact staging interior carrying the derived reservation), restore the
/// `Preparing` record, and only then fill the root. The sequence resumes from
/// step 6 with capacity already gone.
#[test]
fn a_resumed_in_flight_admission_refuses_at_the_publish_when_the_root_filled() {
    let fixture = Fixture::new("capacity-in-flight");
    let resident = reservation(0x79, 2);
    let mid_flight = reservation(0x7A, 2);

    admitted(attempt(&fixture, &resident), "the first action publishes");
    admitted(
        attempt(&fixture, &mid_flight),
        "the second action publishes before it is rewound",
    );

    // Rewind the second action to its pre-publish state: its published
    // directory becomes the staging directory again, and the durable record
    // goes back to `Preparing`.
    fs::rename(
        fixture.action_directory(&mid_flight),
        fixture.slot(InfrastructureSlotV1::ActionAdmissionStaging),
    )
    .unwrap();
    fs::write(
        fixture.slot(InfrastructureSlotV1::ActionAdmissionActive),
        ActionDirectoryAdmissionV1::preparing(&mid_flight)
            .encode_canonical()
            .expect("the preparing record encodes canonically"),
    )
    .unwrap();
    assert!(
        !fixture
            .slot(InfrastructureSlotV1::ActionAdmissionScratch)
            .exists()
    );

    // Fill the root to the frozen budget while that admission is in flight.
    for index in 0..u8::try_from(MAX_ACTIVE_ACTION_DIRS - 1).unwrap() {
        let name = RootEntryNameV1::ActiveAction(ActionDigestV1::new([index; 32])).name();
        fs::create_dir(fixture.catalog_root().join(name)).unwrap();
    }
    let full = fixture.catalog_children();
    assert_eq!(
        full.iter().filter(|name| is_action_row(name)).count(),
        MAX_ACTIVE_ACTION_DIRS,
        "the fixture must fill the root to exactly the frozen budget: {full:?}"
    );

    // The driver's new-admission gate is bypassed by construction here, so the
    // refusal has to come from the destination recheck at the rename.
    stopped_with(
        attempt(&fixture, &mid_flight),
        "publish admission action directory",
        "publication would exceed the frozen active-action bound",
    );
    assert_eq!(
        fixture.catalog_children(),
        full,
        "a refusal inside the acquisition window must publish nothing"
    );
    assert!(
        !fixture.action_directory(&mid_flight).exists(),
        "the over-filling action row must not reach the catalog root"
    );
    assert!(
        fixture
            .slot(InfrastructureSlotV1::ActionAdmissionStaging)
            .is_dir(),
        "the refusal must leave the staging directory intact for a later retry"
    );
}

/// Whether a catalog-root child classifies as an active-action row under the
/// frozen grammar — the same predicate the interior census charges.
fn is_action_row(name: &str) -> bool {
    matches!(
        RootEntryNameV1::parse(name.as_bytes()),
        CatalogNameClassificationV1::Valid(RootEntryNameV1::ActiveAction(_))
    )
}

/// Step 1.1 (e). Capacity includes all barrier, managed-generation, marker,
/// cleanup, and terminal-retirement slots before the first action mutation
/// (ConsumerCheckpoint §7 :223-224): the reservation resident at admission
/// already covers every family, and none of those slots exists yet.
#[test]
fn capacity_covers_every_slot_family_before_the_first_action_mutation() {
    let fixture = Fixture::new("complete-capacity");
    let expected = reservation(0x66, 2);
    let schedule = expected.schedule();
    assert!(schedule.barrier_count() > 0);
    assert!(schedule.generation_count() > 0);
    assert!(schedule.component_count() > 0);
    assert_eq!(
        schedule.cleanup_aliases().mask(),
        CleanupAliasSetV1::all().mask()
    );

    let handoff = admitted(
        attempt(&fixture, &expected),
        "admission reserves the complete capacity",
    );

    let resident = resident_reservation(&fixture, &expected);
    assert_eq!(
        resident, expected,
        "the reservation persisted before the publish must be the complete derived capacity"
    );
    assert_eq!(handoff.reservation(), &expected);

    let directory = fixture.action_directory(&expected);
    for (family, slots) in reserved_capacity_families(resident.schedule()) {
        assert!(
            !slots.is_empty(),
            "the resident capacity reserves no {family} slot"
        );
        for name in slots
            .into_iter()
            .map(|slot| slot.name(expected.action_digest()))
        {
            assert!(
                !directory.join(&name).exists(),
                "capacity must be reserved before the first action mutation: {name}"
            );
        }
    }
}
