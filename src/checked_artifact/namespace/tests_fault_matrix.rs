//! R2-D Step 2.2 — the executed `namespace.*` interruption/restart/convergence
//! matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.2 (the retained
//! handle backend for `publish_exact`/`retire_exact`/`barrier` over scheduled
//! namespace roles, "Activates `namespace.*` with rows") and §4 Step 1.3 for the
//! matrix form on both target variants after the
//! `catalog/bootstrap/tests.rs:328`/`:398` pattern;
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this package
//! flips and §4.3 rows E12/E13/E14 for the three edges driven here;
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 for the repeated-crash rule;
//! `GwzM5-8R4bP1P2-RemPlan-4.md` §4 R2 stop clause (:1089-1092) for the stable
//! deterministic slots the repeated-crash case proves.
//!
//! Every row drives the real backend against a real target: a real lease, the
//! sealed catalog owner, the frozen admission seam, and then a *fresh* retained
//! action-namespace capability per attempt — so an interruption is a real
//! process stop across a real durable namespace edge.
//!
//! **What the restart reconstructs, and why.** Each restart re-acquires the
//! lease, re-recovers the catalog, and re-retains the action directory through
//! the permit, which re-proves the directory's identity against the handoff. The
//! `AdmittedActionV1` handoff token itself is rebuilt through the test-only
//! admission issuer rather than by re-running `resume_or_admit`: once a
//! namespace edge has published its first row the action directory is no longer
//! *exact* (`protocol/admission/owner.rs:29-38`), which is precisely the state a
//! second admission must refuse. Resuming that handoff from durable state is
//! **not owned by any landed step**: Step 3.3 considered it and declined — the
//! plan's 3.3 sentence does not name it, and the only route to reconstruct an
//! `AdmittedActionV1` in that state is `protocol/admission/test_support.rs`,
//! so closing it means widening the frozen admission classifier
//! (`GwzM5-8R2DInterfaceFreeze.md` §3.1). It is item 6 of the Phase 3 settle
//! docket. `retain_action_namespace` still fails closed if the reconstructed
//! identity is not the resident one.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py:670-677`) and out of the
//! injection-site rescan (`interface_tests/fault_expected_keys.rs:369-401`); the
//! aliased key import mirrors `admission/tests_fault_matrix.rs:36-38`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{ActionNamespace, HostActionNamespaceV1, retain_action_namespace};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CheckedFsError, DurableObjectIdentityV1,
};
use crate::checked_artifact::catalog::{OpaqueRetainedCatalogV1, recover_or_create};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_namespace_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CleanupAliasSetV1, ManagedBootstrapInputV1, ProtocolRecordKindV1,
    RecordObservationV1, RequestOwnerBindingV1, RootEntryNameV1, admit_observed_action,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Every `namespace.*` boundary, in the order one virgin drive crosses them.
///
/// The virgin sequence reaches all eleven with the three Step 2.2 edges over one
/// scheduled barrier role: `source_retain` retains the scratch row;
/// `parent_revalidate` fires on the first `validate_operation`
/// (`namespace/mod.rs:277-289`) of the publish; the four publish boundaries are
/// edge E12; `parent_barrier` is edge E14; and the four retirement boundaries
/// are edge E13.
///
/// No key is reserved for Step 2.3. The four managed operations are physically
/// the same publish and retire edges plus a managed *observation*, so when 2.3
/// makes them real they reuse these eleven boundaries through the same shared
/// edge helper; the observation boundaries they add belong to the
/// `managed_bootstrap.*` family (`fault_v1.rs:148-177`, Phase 3), not to this
/// one.
const NAMESPACE_MATRIX: [Fault; 11] = [
    Fault::NamespaceSourceRetain,
    Fault::NamespaceParentRevalidate,
    Fault::NamespaceDestinationReserve,
    Fault::NamespacePrePublishReobserve,
    Fault::NamespacePublishNoReplace,
    Fault::NamespacePublishedReobserve,
    Fault::NamespaceParentBarrier,
    Fault::NamespaceRetirementReserve,
    Fault::NamespacePreRetireReobserve,
    Fault::NamespaceRetireExact,
    Fault::NamespaceRetiredReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin namespace drive settles in two durable edges and leaves two rows
/// in the action directory, so twelve crashes cross the nominal capacity
/// several times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

/// The two frozen target variants the matrix must execute on
/// (`GwzM5-8R2D-Plan.md` §4 Step 1.3; the
/// `catalog/bootstrap/tests.rs:328`/`:398` pair).
#[derive(Clone, Copy, Debug)]
pub(super) enum TargetVariantV1 {
    Workspace,
    GitDirectory,
}

impl TargetVariantV1 {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::GitDirectory => "git-directory",
        }
    }

    const fn private_root(self) -> CatalogPrivateRootV1 {
        match self {
            Self::Workspace => CatalogPrivateRootV1::Workspace,
            Self::GitDirectory => CatalogPrivateRootV1::GitDirectory,
        }
    }
}

pub(super) struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-namespace-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        Self { root }
    }

    /// The fixture's own root. Widened from private for Step 2.3's managed
    /// matrix, which places its managed parent beside the catalog rather than
    /// inside it — where a real managed parent lives.
    pub(super) fn path(&self) -> &Path {
        &self.root
    }

    pub(super) fn catalog_root(&self, variant: TargetVariantV1) -> PathBuf {
        let base = match variant {
            TargetVariantV1::Workspace => self.root.clone(),
            TargetVariantV1::GitDirectory => git2::Repository::open(&self.root)
                .unwrap()
                .commondir()
                .to_path_buf(),
        };
        base.join(CatalogPrivateNameV1::Final.relative_path(variant.private_root()))
    }

    pub(super) fn action_directory(
        &self,
        variant: TargetVariantV1,
        action: ActionDigestV1,
    ) -> PathBuf {
        self.catalog_root(variant)
            .join(RootEntryNameV1::ActiveAction(action).name())
    }

    /// The action directory's sorted child names. Widened from private for R2-E
    /// Step E1.2's cleanup matrix, which censuses the same directory.
    pub(super) fn action_children(
        &self,
        variant: TargetVariantV1,
        action: ActionDigestV1,
    ) -> Vec<String> {
        let mut names = fs::read_dir(self.action_directory(variant, action))
            .expect("the admitted action directory must exist")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    /// The one real admission per fixture. An action is admitted once; the
    /// namespace edges that follow are what this matrix interrupts.
    pub(super) fn admit(
        &self,
        variant: TargetVariantV1,
        expected: &ActionCapacityReservationV1,
    ) -> DurableObjectIdentityV1 {
        with_catalog(self, variant, |catalog| {
            Ok(ActionAdmissionOwnerV1::from_retained_catalog(catalog)
                .resume_or_admit(expected)?
                .directory_identity()
                .clone())
        })
        .expect("the frozen admission seam must admit the action")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One fresh-process catalog session on the requested target: acquire that
/// target's lease, recover through the sole sealed catalog owner, run the body,
/// release. The workspace arm mirrors `admission/tests_fault_matrix.rs:179-186`
/// and the Git-directory arm mirrors `:187-198`.
pub(super) fn with_catalog<T>(
    fixture: &Fixture,
    variant: TargetVariantV1,
    body: impl FnOnce(OpaqueRetainedCatalogV1<'_>) -> Result<T, CheckedFsError>,
) -> Result<T, CheckedFsError> {
    match variant {
        TargetVariantV1::Workspace => {
            let runtime = try_acquire_workspace_runtime(fixture.path())
                .unwrap()
                .expect("workspace runtime lease");
            let catalog = recover_or_create(runtime.catalog_mutation_lease())
                .expect("the sealed catalog owner must recover a complete catalog");
            body(catalog)
        }
        TargetVariantV1::GitDirectory => {
            let request =
                CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
            let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
            let leases = CatalogLeaseSetV1::try_acquire(batch)
                .unwrap()
                .expect("Git catalog lease");
            let lease = leases.leases().next().expect("one Git catalog lease");
            let catalog = recover_or_create(lease)
                .expect("the sealed catalog owner must recover a complete catalog");
            body(catalog)
        }
    }
}

pub(super) fn reservation(action: u8, barriers: usize) -> ActionCapacityReservationV1 {
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

pub(super) fn slot_leaf(slot: ActionSlotV1, action: ActionDigestV1) -> AsciiComponent {
    AsciiComponent::parse(slot.name(action).as_bytes()).expect("a scheduled slot name is ASCII")
}

/// Rebuilds the handoff for a restart. See the module header: the resume of a
/// durable handoff is Step 3.3's coordinator, and `retain_action_namespace`
/// re-proves this identity against the resident action directory anyway.
pub(super) fn handoff(
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
) -> AdmittedActionV1 {
    admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        expected,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity.clone(),
            RecordObservationV1::Exact(expected.clone()),
        ),
    )
    .expect("a resident exact action directory admits its own reservation")
}

/// One fresh-process namespace attempt: retain the action namespace through the
/// permit and drive the scheduled barrier role forward by whichever of the three
/// Step 2.2 edges the resident durable state has not yet crossed.
///
/// Every name is derived from the schedule; a resumed attempt reuses the same
/// deterministic slot names and never allocates a retry name.
fn attempt(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    identity: &DurableObjectIdentityV1,
) -> Result<(), CheckedFsError> {
    let action = expected.action_digest();
    let action_directory = fixture.action_directory(variant, action);
    with_catalog(fixture, variant, |catalog| {
        drive(
            &catalog,
            handoff(expected, identity),
            expected,
            &action_directory,
        )
    })
}

fn drive(
    catalog: &OpaqueRetainedCatalogV1<'_>,
    admitted: AdmittedActionV1,
    expected: &ActionCapacityReservationV1,
    action_directory: &Path,
) -> Result<(), CheckedFsError> {
    let action = expected.action_digest();
    let scratch = slot_leaf(
        ActionSlotV1::Base(BaseActionSlotV1::BarrierIntentScratch),
        action,
    );
    let mut namespace: ActionNamespace<HostActionNamespaceV1> =
        retain_action_namespace(catalog, admitted)?;
    let slots = namespace.scheduled_barrier_slots(0, scratch.clone())?;
    let active = slots.active_leaf().clone();
    let retired = slots.retired_leaf().clone();

    if namespace.scheduled_row_is_resident(&retired) {
        return Ok(());
    }
    if !namespace.scheduled_row_is_resident(&active) {
        if !namespace.scheduled_row_is_resident(&scratch) {
            write_scratch(action_directory, &scratch, expected);
        }
        let source =
            namespace.retain_scheduled_source(scratch, ProtocolRecordKindV1::BarrierIntent)?;
        namespace.publish_barrier_intent(&source, &slots)?;
    }
    let parent = namespace.retained_parent();
    namespace.barrier_namespace(&parent, &slots)?;
    let source = namespace.retain_scheduled_source(active, ProtocolRecordKindV1::BarrierIntent)?;
    namespace.retire_barrier_intent(&source, &slots)?;
    Ok(())
}

/// The scratch write is not a `namespace.*` edge: the durable record write of a
/// scheduled role is edge family P2 and belongs to plan §4 Step 2.4
/// (`record.*`), so the fixture places the row and the matrix interrupts only
/// the three namespace edges that move it.
pub(super) fn write_scratch(
    action_directory: &Path,
    scratch: &AsciiComponent,
    expected: &ActionCapacityReservationV1,
) {
    let name = std::str::from_utf8(scratch.as_bytes()).expect("a scheduled slot name is ASCII");
    let bytes = expected
        .encode_canonical()
        .expect("the reservation is canonically encodable");
    fs::write(action_directory.join(name), bytes).expect("the action directory is writable");
}

/// The executed key list must reconcile against the vocabulary's own
/// `namespace.*` inventory, so a key added to the family without a matrix row
/// fails here rather than silently escaping
/// (`interface_tests/fault_expected_keys.rs`; the
/// `catalog/bootstrap/tests.rs:358-372` reconciliation).
fn reconcile_executed_keys() {
    let mut actual = NAMESPACE_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("namespace.").then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
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

/// Interrupt at every `namespace.*` boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    let expected = reservation(0xC3, 2);
    let settled_rows = {
        let fixture = Fixture::new(&format!("{}-settled", variant.label()));
        let identity = fixture.admit(variant, &expected);
        settle(&fixture, variant, &expected, &identity, "baseline")
    };

    for key in NAMESPACE_MATRIX {
        let stable = key.stable_key();
        let fixture = Fixture::new(&format!("{}-{}", variant.label(), suffix(&stable)));
        let identity = fixture.admit(variant, &expected);

        run_next_namespace_fault(key, || panic!("simulated namespace process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, variant, &expected, &identity);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let resumed_rows = settle(&fixture, variant, &expected, &identity, &stable);
        assert_eq!(
            resumed_rows, settled_rows,
            "{stable}: the restart did not converge to the settled action directory"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-retains the same action namespace and mutates nothing.
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

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
///
/// A boundary is repeatable only when the durable state its crash leaves
/// resolves back to the same edge. The four boundaries *after* a rename —
/// `publish_no_replace`, `published_reobserve`, `retire_exact` and
/// `retired_reobserve` — are crossed at most once per action by construction,
/// because the row they moved is exactly the state that advances the sequence;
/// that they converge on restart is what the matrix above proves. The four
/// read-only pre-rename boundaries — `destination_reserve`,
/// `retirement_reserve`, `pre_publish_reobserve` and `parent_revalidate` —
/// are equally repeatable (their crashes leave no durable delta at all);
/// the three crossed here are chosen because each is additionally a row a
/// retry could be tempted to re-name, one per Step 2.2 edge:
///
/// * `namespace.source_retain` leaves the scratch row resident, so every restart
///   re-enters edge E12 and must reopen the *same* scratch name;
/// * `namespace.parent_barrier` leaves the active row resident with the barrier
///   unwitnessed, so every restart re-crosses edge E14 over the same retained
///   action directory; and
/// * `namespace.pre_retire_reobserve` leaves the active row resident with the
///   retirement destination still free, so every restart re-enters edge E13 and
///   must re-derive the same retirement row rather than a fresh one.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let expected = reservation(0xD4, 2);
    let settled_rows = {
        let fixture = Fixture::new(&format!("{}-repeat-settled", variant.label()));
        let identity = fixture.admit(variant, &expected);
        settle(&fixture, variant, &expected, &identity, "baseline")
    };

    for key in [
        Fault::NamespaceSourceRetain,
        Fault::NamespaceParentBarrier,
        Fault::NamespacePreRetireReobserve,
    ] {
        let stable = key.stable_key();
        let fixture = Fixture::new(&format!("{}-r-{}", variant.label(), suffix(&stable)));
        let identity = fixture.admit(variant, &expected);
        let mut census: Option<Vec<String>> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_namespace_fault(key, || panic!("simulated namespace process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = attempt(&fixture, variant, &expected, &identity);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
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
        assert_eq!(
            rows.len(),
            settled_rows.len(),
            "{stable}: the interrupted action directory changed cardinality: {rows:?}"
        );

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

#[test]
fn namespace_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn namespace_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_namespace_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_namespace_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}
