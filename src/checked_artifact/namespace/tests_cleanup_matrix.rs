//! R2-E Phase E1 Step E1.2 — the executed `cleanup.*`
//! interruption/restart/convergence matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2E-Plan.md` §3 Phase E1;
//! `GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §2 (the eleven key semantics,
//! DECISIONS C-1/C-2/C-3, the `AdmittedActionV1` duty and the §2.5 convergence
//! obligation) as amended by `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §4
//! (OPEN-C1 struck — no interleaving proof is owed), §6.3 (the amended duty
//! list) and §8's [P2-4] cure (the corrected admitted-action duty);
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation row this package
//! flips; `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 for the repeated-crash rule.
//! Calibrated on `admission/tests_fault_matrix.rs` and
//! `namespace/tests_fault_matrix.rs`, whose fixture this file reuses.
//!
//! **The `AdmittedActionV1` duty, and how it is discharged here** (amendment
//! §2.4 as corrected by addendum §8's [P2-4] cure). The **first** attempt of
//! every fixture drives a real admitted action: the reservation is derived by
//! the coordinator's own schedule facade from a real `CheckedActionRequestV1`
//! (`coordinator/schedule.rs`, `derive_new_reservation`) wherever the schedule
//! it derives is the one under test, and the handoff comes from
//! `ActionAdmissionOwnerV1::resume_or_admit` — the exact call
//! `ScheduledCheckedActionV1::admit` makes (`coordinator/execution.rs:137-152`).
//! `coordinator::execution` is a deliberately private module whose own header
//! reserves its widening for R2-E's *consumer* conversion, so this step names
//! the seam that module calls rather than widening it for a test; the substance
//! the cure asks for — a real admission, never a hand-built reservation — is
//! unchanged, and the coordinator's own derivation supplies the reservation too.
//!
//! **Every restart rebuilds the handoff through the test-only issuer**
//! (`protocol/admission/test_support.rs`, via
//! `tests_fault_matrix::handoff`), inheriting the documented deviation at
//! `namespace/tests_fault_matrix.rs:20-35`: once a namespace edge has published
//! its first row the action directory is no longer *exact*
//! (`protocol/admission/owner.rs:29-38`), which is precisely the state a second
//! admission must refuse, and resuming that handoff from durable state "is not
//! owned by any landed step … It is item 6 of the Phase 3 settle docket".
//! R2-E inherits that deviation and cites the docket item; it mints no new
//! owner. `retain_action_namespace` still fails closed if the reconstructed
//! identity is not the resident one.
//!
//! **The one-alias row.** The single-crossing half is driven on a schedule
//! reserving exactly one cleanup alias, because the three alias retirements
//! share one helper: on a three-alias row each of the retirement boundaries is
//! crossed once per alias and is therefore neither repeatable nor
//! single-crossing (amendment §2.5). The coordinator's schedule facade derives
//! only the masks `0b111` / `0b110` / `0b101` / `0` from an operation
//! (`coordinator/schedule.rs:39-49`), so a one-alias schedule is stated
//! directly for that row and admitted through the same real seam.
//!
//! **THE GIT-DIRECTORY ROUTE STATEMENT** (addendum §7.7's duty on E1.2 / E2.3 /
//! E3.2). This matrix's Git-directory arm takes **neither** offered route: not
//! route (a), the Step-2.3 `cfg(test)` door
//! `retain_managed_parent_at_for_test`
//! (`capability/pre_catalog/provider/managed_mutation.rs:383`), and not route
//! (b), a managed prefix under the target's own retained root. It needs no
//! managed parent at all. Every `cleanup.*` boundary is inside the one retained
//! action directory (DECISION C-2), which is reached by a single
//! identity-proved no-follow hop from the permit-retained completed catalog —
//! and for a Git-directory target that catalog is the one
//! `CatalogLeaseTargetRequestV1::repository_common_git_directory` leases, which
//! `tests_fault_matrix::with_catalog`'s Git arm already acquires. So the
//! settle's "a Git-directory catalog has no `.gwz` ancestor" concern
//! (freeze `:596-600`, `:672-680`) does not reach this family: **E1 leaves the
//! Step-2.3 door untouched and inherits it unchanged**, and the disposition of
//! that door, plus the Git-directory workspace-root binding, stay E4.2's.
//!
//! **`CATALOG_PUBLICATION_CALL_COUNTS` DOES NOT MOVE** (addendum §6.2(a)'s
//! stated-confirmation duty). Every cleanup retirement routes through
//! `RetainedActionNamespaceV1::execute_edge`, which already holds
//! `namespace_mutation.rs`'s single counted `publish_verified_no_replace` call
//! site, and the worklist publication routes through that same call site. E1
//! opens no sealed primitive of its own and adds no production caller file, so
//! the dict's per-file counts and its set-equality check are both unmoved.
//!
//! **Census.** 165 keys total, unchanged; no key minted, none retired;
//! `cleanup.*` moves 0/11 reserved to 11/11 executed in the commit that lands
//! this matrix, per RemPlan §10's duty.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py`) and out of the
//! injection-site rescan (`interface_tests/fault_expected_keys.rs`), exactly as
//! `tests_fault_matrix.rs` does.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;

use super::tests_fault_matrix::{Fixture, TargetVariantV1, handoff, slot_leaf, with_catalog};
use super::{ActionNamespace, HostActionNamespaceV1, PublishRoleV1, retain_action_namespace};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::capability::{
    AsciiComponent, CheckedFsError, DurableObjectIdentityV1, PreCatalogRootKindV1,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::coordinator::{
    CheckedActionOperationV1, CheckedActionOwnerV1, CheckedLeafFactV1,
    CoordinatorScheduleDecisionV1, derive_new_reservation, synthetic_leaf_request,
};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_cleanup_fault, take_armed_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CleanupAliasSetV1, CleanupAliasV1, CleanupPhysicalFactV1,
    CleanupResolutionV1, CleanupRowV1, CleanupWorklistV1, ProtocolRecordKindV1,
    RequestOwnerBindingV1,
};

/// Every `cleanup.*` boundary, in the order one virgin drive crosses them.
///
/// The virgin sequence reaches all eleven in one pass over one admitted action:
/// each reserved alias's physical fact pair is observed first (`source_reobserve`,
/// `destination_reobserve`), because those facts are what the worklist record is
/// derived from *and* what resolves each row afterwards; the three scratch
/// boundaries then write the derived record into the shared
/// `BaseActionSlotV1::RecordScratch` row (DECISION C-3); the Step-2.2 backend
/// publishes it onto `BaseActionSlotV1::CleanupWorklist` and `worklist_publish` /
/// `worklist_reobserve` name that rename's post-edge state and its bounded proof
/// (DECISION C-1); each `Retire` row then crosses `alias_retire`,
/// `retired_alias_reobserve` and `row_complete`; and `completion_reobserve` is
/// the whole-worklist proof `terminal.cleanup_reobserve` will consume.
const CLEANUP_MATRIX: [Fault; 11] = [
    Fault::CleanupSourceReobserve,
    Fault::CleanupDestinationReobserve,
    Fault::CleanupWorklistScratchCreate,
    Fault::CleanupWorklistScratchWrite,
    Fault::CleanupWorklistScratchFlush,
    Fault::CleanupWorklistPublish,
    Fault::CleanupWorklistReobserve,
    Fault::CleanupAliasRetire,
    Fault::CleanupRetiredAliasReobserve,
    Fault::CleanupRowComplete,
    Fault::CleanupCompletionReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin cleanup drive settles in one publication and three retirements and
/// leaves five rows in the action directory, so twelve crashes cross the nominal
/// capacity several times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

/// **The inclusion criterion, stated once**, in the form
/// `bootstrap/managed/tests_writer_matrix.rs` states it: a boundary is
/// *single-crossing* when the durable state its crash leaves routes the next
/// drive **past** it; every other boundary is repeatable, because a restart
/// re-enters it on the same durable state.
///
/// * `source_reobserve` / `destination_reobserve` — read-only, crossed once per
///   reserved alias on every attempt including the settled resume.
/// * `worklist_scratch_create` / `_write` / `_flush` — crossed while the
///   worklist row is absent, and a crash at any of them leaves it absent, so
///   every restart re-enters the same deterministic scratch row rather than
///   allocating a retry name (the write-or-rewrite discipline of DECISION C-3).
/// * `worklist_publish` / `worklist_reobserve` — crossed on every attempt once
///   the row is resident, which a crash at either leaves it.
/// * `completion_reobserve` — the settling proof; a crash at it leaves every row
///   complete, so the next drive re-proves the same state.
const REPEATED_BOUNDARIES: [Fault; 8] = [
    Fault::CleanupSourceReobserve,
    Fault::CleanupDestinationReobserve,
    Fault::CleanupWorklistScratchCreate,
    Fault::CleanupWorklistScratchWrite,
    Fault::CleanupWorklistScratchFlush,
    Fault::CleanupWorklistPublish,
    Fault::CleanupWorklistReobserve,
    Fault::CleanupCompletionReobserve,
];

/// The complement of [`REPEATED_BOUNDARIES`], and the reason the probe below
/// runs on a **one-alias** row: a crash at any of the three leaves the alias
/// retired, so the row classifies `CleanupResolutionV1::Complete`
/// (`protocol/cleanup.rs:394-398`) and the next drive routes past all three.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 3] = [
    Fault::CleanupAliasRetire,
    Fault::CleanupRetiredAliasReobserve,
    Fault::CleanupRowComplete,
];

const WORKSPACE: &str = "ws_r2e_e1_cleanup";

/// The alias-to-scheduled-row pairing this matrix seeds against, mirrored from
/// the production derivation in `namespace/mod.rs`'s `cleanup_retirement`. The
/// drive itself uses the production names, so a divergence fails here rather
/// than passing silently.
const fn alias_source_slot(alias: CleanupAliasV1) -> BaseActionSlotV1 {
    match alias {
        CleanupAliasV1::Source => BaseActionSlotV1::SourcePayload,
        CleanupAliasV1::Goal => BaseActionSlotV1::GoalPayload,
        CleanupAliasV1::Authority => BaseActionSlotV1::Authority,
    }
}

const fn alias_retired_slot(alias: CleanupAliasV1) -> BaseActionSlotV1 {
    match alias {
        CleanupAliasV1::Source => BaseActionSlotV1::RetiredSourceAlias,
        CleanupAliasV1::Goal => BaseActionSlotV1::RetiredGoalAlias,
        CleanupAliasV1::Authority => BaseActionSlotV1::RetiredAuthorityAlias,
    }
}

const fn alias_bit(alias: CleanupAliasV1) -> u8 {
    match alias {
        CleanupAliasV1::Source => 1,
        CleanupAliasV1::Goal => 2,
        CleanupAliasV1::Authority => 4,
    }
}

fn reserves(reservation: &ActionCapacityReservationV1, alias: CleanupAliasV1) -> bool {
    reservation.schedule().cleanup_aliases().mask() & alias_bit(alias) != 0
}

fn name_of(leaf: &AsciiComponent) -> String {
    std::str::from_utf8(leaf.as_bytes())
        .expect("a scheduled slot name is ASCII")
        .to_owned()
}

fn slot_name(slot: BaseActionSlotV1, action: ActionDigestV1) -> String {
    name_of(&slot_leaf(ActionSlotV1::Base(slot), action))
}

fn cleanup_drive_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("cleanup drive", detail)
}

/// The three-alias schedule, derived by the coordinator itself: a `Replace` over
/// an exact expected leaf reserves `0b111` (`coordinator/schedule.rs:39-49`), so
/// nothing about this reservation is hand-built.
fn coordinator_reservation(variant: TargetVariantV1) -> ActionCapacityReservationV1 {
    let owner = CheckedActionOwnerV1::for_merge_start(WORKSPACE).expect("the owner is valid");
    let root_kind = match variant {
        TargetVariantV1::Workspace => PreCatalogRootKindV1::Workspace,
        TargetVariantV1::GitDirectory => PreCatalogRootKindV1::GitDirectory,
    };
    let request = synthetic_leaf_request(
        &owner,
        CheckedActionOperationV1::Replace,
        root_kind,
        vec![AsciiComponent::parse(b"gwz.conf").expect("a fixed test component is ASCII")],
        CheckedLeafFactV1::Exact {
            length: 7,
            sha256: [5; 32],
        },
        CheckedLeafFactV1::Exact {
            length: 11,
            sha256: [9; 32],
        },
        0,
    )
    .expect("the synthetic leaf request is valid");
    match derive_new_reservation(&request, None).expect("the request schedules") {
        CoordinatorScheduleDecisionV1::Reserve(reservation) => *reservation,
        CoordinatorScheduleDecisionV1::ProofOnly => {
            panic!("a replacement over an exact expected leaf must reserve capacity")
        }
    }
}

/// The one-alias schedule the single-crossing half needs. Stated directly,
/// because no operation makes the coordinator derive a one-alias mask; the
/// admission it is driven through is the same real seam.
fn one_alias_reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([0xE1; 32]),
        RequestOwnerBindingV1::new([0xE2; 32]),
        ActionScheduleV1::try_new(
            0,
            Vec::new(),
            CleanupAliasSetV1::from_mask(alias_bit(CleanupAliasV1::Source))
                .expect("a single-alias cleanup set is valid"),
        )
        .expect("the one-alias schedule is valid"),
    )
}

/// One admitted action with its reserved alias rows resident, and the one real
/// `AdmittedActionV1` its first attempt consumes.
struct CleanupFixture {
    inner: Fixture,
    variant: TargetVariantV1,
    reservation: ActionCapacityReservationV1,
    identity: DurableObjectIdentityV1,
    admitted: RefCell<Option<AdmittedActionV1>>,
}

impl CleanupFixture {
    fn new(
        variant: TargetVariantV1,
        label: &str,
        reservation: ActionCapacityReservationV1,
    ) -> Self {
        let inner = Fixture::new(&format!("cleanup-{}-{label}", variant.label()));
        let admitted = with_catalog(&inner, variant, |catalog| {
            ActionAdmissionOwnerV1::from_retained_catalog(catalog).resume_or_admit(&reservation)
        })
        .expect("the frozen admission seam must admit the action");
        let identity = admitted.directory_identity().clone();
        seed_alias_rows(&inner, variant, &reservation);
        Self {
            inner,
            variant,
            reservation,
            identity,
            admitted: RefCell::new(Some(admitted)),
        }
    }

    /// One fresh-process cleanup attempt. The first one carries the real
    /// admitted action; every restart rebuilds the handoff through the test-only
    /// issuer, per the module header.
    fn attempt(&self) -> Result<(), CheckedFsError> {
        let admitted = self
            .admitted
            .borrow_mut()
            .take()
            .unwrap_or_else(|| handoff(&self.reservation, &self.identity));
        with_catalog(&self.inner, self.variant, |catalog| {
            drive(&catalog, admitted, &self.reservation)
        })
    }

    fn children(&self) -> Vec<String> {
        self.inner
            .action_children(self.variant, self.reservation.action_digest())
    }

    /// Every name this action's schedule can legitimately produce in its own
    /// directory. A repeated crash that allocated a retry name would leave a
    /// child outside this set, which is the property ConsumerCheckpoint §12 asks
    /// to be proved.
    fn scheduled_names(&self) -> BTreeSet<String> {
        let action = self.reservation.action_digest();
        let mut names = BTreeSet::from([
            slot_name(BaseActionSlotV1::Reservation, action),
            slot_name(BaseActionSlotV1::RecordScratch, action),
            slot_name(BaseActionSlotV1::CleanupWorklist, action),
        ]);
        for alias in CleanupAliasV1::ALL {
            if reserves(&self.reservation, alias) {
                names.insert(slot_name(alias_source_slot(alias), action));
                names.insert(slot_name(alias_retired_slot(alias), action));
            }
        }
        names
    }
}

/// Places the reserved alias rows the worklist will fingerprint.
///
/// The rows are not a `cleanup.*` edge: the durable write of a scheduled
/// payload or authority row belongs to the `durable_leaf.*` and `record.*`
/// families that Steps 2.1 and 2.4 already executed, so the fixture places them
/// and this matrix interrupts only the eleven cleanup boundaries that resolve
/// them — the same division `tests_fault_matrix.rs`'s `write_scratch` makes.
fn seed_alias_rows(
    fixture: &Fixture,
    variant: TargetVariantV1,
    reservation: &ActionCapacityReservationV1,
) {
    let directory = fixture.action_directory(variant, reservation.action_digest());
    for alias in CleanupAliasV1::ALL {
        if !reserves(reservation, alias) {
            continue;
        }
        let name = slot_name(alias_source_slot(alias), reservation.action_digest());
        let bytes = vec![alias_bit(alias); 32 + usize::from(alias_bit(alias))];
        fs::write(directory.join(name), bytes).expect("the action directory is writable");
    }
}

/// One fresh-process cleanup drive: observe every reserved alias's durable fact
/// pair, publish the worklist record derived from them if this action has not
/// published one, then resolve each row from durable truth and prove the whole
/// worklist complete.
///
/// Every name is derived from the schedule; a resumed attempt reuses the same
/// deterministic slot names and never allocates a retry name.
fn drive(
    catalog: &OpaqueRetainedCatalogV1<'_>,
    admitted: AdmittedActionV1,
    expected: &ActionCapacityReservationV1,
) -> Result<(), CheckedFsError> {
    let mut namespace: ActionNamespace<HostActionNamespaceV1> =
        retain_action_namespace(catalog, admitted)?;

    // 1 — every reserved alias's `(source, destination)` physical fact pair.
    let mut planned = Vec::new();
    for alias in CleanupAliasV1::ALL {
        let Ok(retirement) = namespace.cleanup_retirement(alias) else {
            continue;
        };
        let (source, destination) = namespace.observe_cleanup_row_facts(&retirement)?;
        planned.push((alias, retirement, source, destination));
    }

    // 2 — the worklist record, derived from those same facts, if this action has
    // not published one yet. A leftover scratch is this drive's own residue and
    // is rewritten rather than wedged against (DECISION C-3).
    let worklist_leaf = namespace
        .publish_destination(PublishRoleV1::CleanupWorklist)
        .leaf()
        .clone();
    if !namespace.scheduled_row_is_resident(&worklist_leaf) {
        let mut rows = Vec::new();
        for (alias, _, source, _) in &planned {
            let CleanupPhysicalFactV1::Exact(fingerprint) = source else {
                return Err(cleanup_drive_error(
                    "a reserved cleanup alias row is not exactly resident",
                ));
            };
            rows.push(CleanupRowV1::new(*alias, fingerprint.clone()));
        }
        let worklist = CleanupWorklistV1::try_new(expected, rows).map_err(|_| {
            cleanup_drive_error("the observed cleanup rows do not form a reserved worklist")
        })?;
        let bytes = worklist.encode_canonical().map_err(|_| {
            cleanup_drive_error("the cleanup worklist is not canonically encodable")
        })?;
        namespace.write_cleanup_worklist_scratch(&bytes)?;
        let scratch_leaf = namespace
            .publish_destination(PublishRoleV1::RecordScratch)
            .leaf()
            .clone();
        let source = namespace
            .retain_scheduled_source(scratch_leaf, ProtocolRecordKindV1::CleanupWorklist)?;
        let destination = namespace.publish_destination(PublishRoleV1::CleanupWorklist);
        namespace.publish_no_replace(&source, &destination)?;
    }

    // 3 — the published row, bound to the resident reservation.
    let bound = namespace.observe_cleanup_worklist_row()?;

    // 4 — each row, resolved by `classify` from the durable facts of step 1.
    for index in 0..bound.len() {
        let row = bound
            .row(index)
            .ok_or_else(|| cleanup_drive_error("a bound worklist row is out of range"))?;
        let Some((_, retirement, source, destination)) =
            planned.iter().find(|(alias, ..)| *alias == row.alias())
        else {
            return Err(cleanup_drive_error(
                "the worklist names an alias the schedule did not reserve",
            ));
        };
        match bound.classify(index, source, destination) {
            Some(CleanupResolutionV1::Complete) => {}
            Some(CleanupResolutionV1::Retire) => {
                let leaf = retirement.source_leaf().clone();
                let source_bound = retirement.source_bound();
                let retained = namespace.retain_scheduled_source(leaf, source_bound)?;
                namespace.retire_exact(&retained, retirement)?;
                namespace.observe_cleanup_retirement(retirement, row.expected())?;
            }
            _ => {
                return Err(cleanup_drive_error("a cleanup row is physically ambiguous"));
            }
        }
    }

    // 5 — the whole-worklist proof `terminal.cleanup_reobserve` consumes.
    namespace.observe_cleanup_completion()
}

/// The executed key list must reconcile against the vocabulary's own `cleanup.*`
/// inventory, so a key added to the family without a matrix row fails here
/// rather than silently escaping (`interface_tests/fault_expected_keys.rs`; the
/// `namespace/tests_fault_matrix.rs:352-367` reconciliation this mirrors).
fn reconcile_executed_keys() {
    let mut actual = CLEANUP_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("cleanup.").then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);

    // The two classification classes partition the matrix exactly, so no key can
    // be quietly dropped from both.
    let mut classified = REPEATED_BOUNDARIES
        .iter()
        .chain(SINGLE_CROSSING_BOUNDARIES.iter())
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    classified.sort_unstable();
    assert_eq!(classified, expected);
}

fn suffix(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .1
}

fn settle(fixture: &CleanupFixture, context: &str) -> Vec<String> {
    fixture
        .attempt()
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    fixture.children()
}

/// Interrupt at every `cleanup.*` boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    let settled_rows = {
        let fixture = CleanupFixture::new(variant, "settled", coordinator_reservation(variant));
        settle(&fixture, "baseline")
    };

    for key in CLEANUP_MATRIX {
        let stable = key.stable_key();
        let fixture =
            CleanupFixture::new(variant, suffix(&stable), coordinator_reservation(variant));

        run_next_cleanup_fault(key, || panic!("simulated cleanup process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fixture.attempt();
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let resumed_rows = settle(&fixture, &stable);
        assert_eq!(
            resumed_rows, settled_rows,
            "{stable}: the restart did not converge to the settled action directory"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-reads the same resident worklist and mutates nothing.
        let again_rows = settle(&fixture, &stable);
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

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
///
/// The namespace matrix additionally asserts that the interrupted directory has
/// the *same cardinality* as the settled one; that is a property of its own
/// two-edge sequence and does not transfer here, because three of these
/// boundaries are crossed before the worklist row exists at all. The property
/// asserted instead is the one the rule actually names, and it is strictly
/// stated: the durable child set is identical across all twelve rounds, it never
/// exceeds the settled cardinality, and every name in it is one this action's
/// schedule derives — so a retry name would fail here rather than pass.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let settled_rows = {
        let fixture =
            CleanupFixture::new(variant, "repeat-settled", coordinator_reservation(variant));
        settle(&fixture, "baseline")
    };

    for key in REPEATED_BOUNDARIES {
        let stable = key.stable_key();
        let fixture = CleanupFixture::new(
            variant,
            &format!("r-{}", suffix(&stable)),
            coordinator_reservation(variant),
        );
        let scheduled = fixture.scheduled_names();
        let mut census: Option<Vec<String>> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_cleanup_fault(key, || panic!("simulated cleanup process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = fixture.attempt();
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );

            let observed = fixture.children();
            assert!(
                observed.iter().all(|name| scheduled.contains(name)),
                "{stable}: round {round} allocated a name the schedule does not derive: {observed:?}"
            );
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let rows = census.expect("the boundary is crashed at least once");
        assert!(
            rows.len() <= settled_rows.len(),
            "{stable}: the interrupted action directory grew past the settled one: {rows:?}"
        );

        let converged_rows = settle(&fixture, &stable);
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

/// R2-E Step E1.2 — the single-crossing claim, machine-checked, in the shape
/// `run_single_crossing_probe` (`bootstrap/managed/tests_provider.rs:564`)
/// states it: crash the boundary once, re-arm it, and require the *next* drive
/// to settle **without** firing it. That helper is
/// `pub(in crate::checked_artifact::bootstrap::managed)` and is bound to that
/// module's own `RowFixture`/`RowShapeV1`, so its shape is reproduced here over
/// this family's fixture rather than called; the property, the re-arm and the
/// `take_armed_fault` check are identical.
///
/// **On a one-alias row**, per amendment §2.5: with three aliases each of these
/// boundaries is crossed once per alias, so the probe would fire on the second
/// alias and prove nothing about the first.
fn run_single_crossing_probe(variant: TargetVariantV1) {
    for key in SINGLE_CROSSING_BOUNDARIES {
        let stable = key.stable_key();
        let fixture = CleanupFixture::new(
            variant,
            &format!("s-{}", suffix(&stable)),
            one_alias_reservation(),
        );

        run_next_cleanup_fault(key, || panic!("simulated cleanup process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fixture.attempt();
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        run_next_cleanup_fault(key, || {
            panic!("a single-crossing boundary was re-crossed by the drive after its crash")
        });
        fixture
            .attempt()
            .unwrap_or_else(|error| panic!("{stable}: the restart must settle: {error:?}"));
        assert!(
            take_armed_fault(),
            "{stable}: the probe's arm vanished without firing"
        );

        println!(
            "{stable} | {} | single-crossing=yes | restart=settled | re-crossed=no",
            variant.label()
        );
    }
}

/// The one-alias row settles and converges too, so the row the single-crossing
/// probe runs on is not proved only by the absence of a re-crossing.
fn run_one_alias_convergence(variant: TargetVariantV1) {
    let settled_rows = {
        let fixture = CleanupFixture::new(variant, "one-settled", one_alias_reservation());
        settle(&fixture, "baseline")
    };

    for key in CLEANUP_MATRIX {
        let stable = key.stable_key();
        let fixture = CleanupFixture::new(
            variant,
            &format!("one-{}", suffix(&stable)),
            one_alias_reservation(),
        );

        run_next_cleanup_fault(key, || panic!("simulated cleanup process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fixture.attempt();
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached on the one-alias row: {stable}"
        );

        let resumed_rows = settle(&fixture, &stable);
        assert_eq!(
            resumed_rows, settled_rows,
            "{stable}: the one-alias restart did not converge"
        );
        println!(
            "{stable} | {} | one-alias | interrupted=yes | restart=settled | rows={}",
            variant.label(),
            resumed_rows.len()
        );
    }
}

#[test]
fn cleanup_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn cleanup_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_cleanup_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_cleanup_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}

#[test]
fn single_crossing_cleanup_boundaries_are_not_recrossed_on_a_workspace_target() {
    run_single_crossing_probe(TargetVariantV1::Workspace);
}

#[test]
fn single_crossing_cleanup_boundaries_are_not_recrossed_on_a_git_directory_target() {
    run_single_crossing_probe(TargetVariantV1::GitDirectory);
}

#[test]
fn one_alias_cleanup_rows_converge_on_a_workspace_target() {
    run_one_alias_convergence(TargetVariantV1::Workspace);
}

#[test]
fn one_alias_cleanup_rows_converge_on_a_git_directory_target() {
    run_one_alias_convergence(TargetVariantV1::GitDirectory);
}
