//! R2-D Step 1.3 — the executed `admission.*` interruption/restart/convergence
//! matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 1.3 (injection sites,
//! the matrix on both target variants after the
//! `catalog/bootstrap/tests.rs:326-383` pattern, and repeated same-boundary
//! crashes past nominal capacity with stable slots) and §4 Step 5.1 (:441) for
//! the per-key evidence form; `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 (:341,
//! :346); `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this
//! package flips; `GwzM5-8R4bP1P2-RemPlan-4.md` §4 R2 stop clause (:1089-1092)
//! for the stable-slot rule the repeated-crash case proves.
//!
//! Every row drives the frozen admission seam on a real target through the
//! sealed catalog owner, so an interruption is a real process stop across a
//! real durable edge rather than a reconstructed on-disk state — that
//! complement already exists at `driver/tests.rs:166`.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py:670-677`) and out of
//! the injection-site rescan
//! (`interface_tests/fault_expected_keys.rs:391`), and the aliased key import
//! mirrors `catalog/bootstrap/tests.rs:12-13`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_admission_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CleanupAliasSetV1, InfrastructureSlotV1, ManagedBootstrapInputV1,
    RequestOwnerBindingV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Every `admission.*` boundary, in the order one virgin drive crosses them.
/// The virgin sequence reaches all nineteen: steps 1-2 observe, step 3 writes
/// and installs `Preparing`, steps 4-6 stage, reserve and publish, and step 7
/// writes and installs `Idle` — the second install being the one that also
/// crosses the retire half, because it is the only one with a resident active
/// record to supersede (`admission/driver.rs:57`, `:139-141`).
const ADMISSION_MATRIX: [Fault; 19] = [
    Fault::AdmissionOccupancyObserve,
    Fault::AdmissionCapacityCheck,
    Fault::AdmissionPreparingScratchCreate,
    Fault::AdmissionPreparingScratchWrite,
    Fault::AdmissionPreparingScratchFlush,
    Fault::AdmissionPreparingPublish,
    Fault::AdmissionPreparingReobserve,
    Fault::AdmissionStagingCreate,
    Fault::AdmissionReservationCreate,
    Fault::AdmissionReservationWrite,
    Fault::AdmissionReservationFlush,
    Fault::AdmissionStagingFlush,
    Fault::AdmissionFinalPublish,
    Fault::AdmissionFinalReobserve,
    Fault::AdmissionIdleScratchCreate,
    Fault::AdmissionIdleScratchWrite,
    Fault::AdmissionIdleScratchFlush,
    Fault::AdmissionIdlePublish,
    Fault::AdmissionIdleReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin drive settles in eight durable edges (`admission/driver.rs:27-30`)
/// and leaves eight nominal catalog rows, so twelve crashes cross both readings
/// of "nominal capacity" (`RemPlan-4-ReviewFS-3.md:27`, "cross the nominal
/// capacity without cardinality growth or GC").
const REPEATED_CRASH_ROUNDS: usize = 12;

/// The two frozen target variants the matrix must execute on
/// (`GwzM5-8R2D-Plan.md` §4 Step 1.3; the
/// `catalog/bootstrap/tests.rs:328`/`:398` pair).
#[derive(Clone, Copy, Debug)]
enum TargetVariantV1 {
    Workspace,
    GitDirectory,
}

impl TargetVariantV1 {
    const fn label(self) -> &'static str {
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

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-fault-{label}-{}-{}",
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

    fn catalog_root(&self, variant: TargetVariantV1) -> PathBuf {
        let base = match variant {
            TargetVariantV1::Workspace => self.root.clone(),
            TargetVariantV1::GitDirectory => git2::Repository::open(&self.root)
                .unwrap()
                .commondir()
                .to_path_buf(),
        };
        base.join(CatalogPrivateNameV1::Final.relative_path(variant.private_root()))
    }

    fn children(&self, variant: TargetVariantV1) -> Vec<String> {
        sorted_children(&self.catalog_root(variant)).expect("the recovered catalog root must exist")
    }

    /// The interior of the one indexed staging action directory, or an empty
    /// list when no staging edge is in flight.
    fn staging_children(&self, variant: TargetVariantV1) -> Vec<String> {
        sorted_children(
            &self
                .catalog_root(variant)
                .join(InfrastructureSlotV1::ActionAdmissionStaging.name()),
        )
        .unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sorted_children(directory: &Path) -> Option<Vec<String>> {
    let mut names = fs::read_dir(directory)
        .ok()?
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    Some(names)
}

/// One fresh-process attempt on the requested target: acquire that target's
/// lease, recover through the sole sealed catalog owner, drive the frozen seam
/// once, release. The workspace arm mirrors `driver/tests.rs:85-95` and the
/// Git-directory arm mirrors `catalog/bootstrap/tests.rs:505-514`.
fn attempt(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) -> Result<AdmittedActionV1, CheckedFsError> {
    match variant {
        TargetVariantV1::Workspace => {
            let runtime = try_acquire_workspace_runtime(fixture.path())
                .unwrap()
                .expect("workspace runtime lease");
            let retained = recover_or_create(runtime.catalog_mutation_lease())
                .expect("the sealed catalog owner must recover a complete catalog");
            ActionAdmissionOwnerV1::from_retained_catalog(retained).resume_or_admit(expected)
        }
        TargetVariantV1::GitDirectory => {
            let request =
                CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
            let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
            let leases = CatalogLeaseSetV1::try_acquire(batch)
                .unwrap()
                .expect("Git catalog lease");
            let lease = leases.leases().next().expect("one Git catalog lease");
            let retained = recover_or_create(lease)
                .expect("the sealed catalog owner must recover a complete catalog");
            ActionAdmissionOwnerV1::from_retained_catalog(retained).resume_or_admit(expected)
        }
    }
}

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

/// Runs the seam to settlement and records what settled. The handoff carries
/// the published directory's durable identity, which is inode-scoped and so
/// comparable only within one fixture; the catalog row set is derived from the
/// frozen grammar and so is comparable across fixtures.
fn settle(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    context: &str,
) -> (AdmittedActionV1, Vec<String>) {
    let handoff = attempt(fixture, variant, expected)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    (handoff, fixture.children(variant))
}

/// The executed key list must reconcile against the vocabulary's own
/// `admission.*` inventory, so a key added to the family without a matrix row
/// fails here rather than silently escaping
/// (`interface_tests/fault_expected_keys.rs`; the
/// `catalog/bootstrap/tests.rs:358-372` reconciliation).
fn reconcile_executed_keys() {
    let mut actual = ADMISSION_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("admission.").then_some(value)
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

/// Interrupt at every `admission.*` boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    let expected = reservation(0xA1, 2);
    let settled_rows = {
        let fixture = Fixture::new(&format!("{}-settled", variant.label()));
        settle(&fixture, variant, &expected, "baseline").1
    };

    for key in ADMISSION_MATRIX {
        let stable = key.stable_key();
        let fixture = Fixture::new(&format!("{}-{}", variant.label(), suffix(&stable)));
        run_next_admission_fault(key, || panic!("simulated admission process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, variant, &expected);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let (resumed, resumed_rows) = settle(&fixture, variant, &expected, &stable);
        assert_eq!(
            resumed.reservation(),
            &expected,
            "{stable}: the restart admitted a different action"
        );
        assert_eq!(
            resumed_rows, settled_rows,
            "{stable}: the restart did not converge to the settled catalog"
        );
        assert!(
            !fixture
                .catalog_root(variant)
                .join(InfrastructureSlotV1::ActionAdmissionScratch.name())
                .exists(),
            "{stable}: the restart left the admission scratch resident"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // resumes the same published directory identity and rewrites nothing.
        let (again, again_rows) = settle(&fixture, variant, &expected, &stable);
        assert_eq!(
            again, resumed,
            "{stable}: the resume republished the action"
        );
        assert_eq!(
            again_rows, settled_rows,
            "{stable}: the resume mutated the settled catalog"
        );

        println!(
            "{stable} | {} | interrupted=yes | restart=settled | rows={} | resume=no-mutation",
            variant.label(),
            resumed_rows.len()
        );
    }
}

/// ConsumerCheckpoint §12 (:346) and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name.
///
/// A boundary is repeatable only when the durable state its crash leaves
/// resolves back to the same edge. The three *write* boundaries are not: the
/// harness stops the process after the syscall, so the record is complete and
/// the restart resolves past the edge. The rest are repeatable, and the three
/// crossed here are selected from them because each leaves a **resident row a
/// retry could be tempted to re-name**, which is the property
/// ConsumerCheckpoint §12 (:346) and the R2 stop clause (:1089-1092) are about:
///
/// * `admission.preparing_scratch_create` leaves the reusable global admission
///   scratch resident but undecodable, so every restart re-enters
///   `WriteAdmissionScratch` and must reopen the *same* slot name;
/// * `admission.reservation_create` leaves an empty resident reservation inside
///   the indexed staging directory, which `classify_expected_prefix` calls
///   `PartialExpectedPrefix`, so every restart routes back through
///   `WriteOrRewriteReservation` and must reuse the same derived slot name; and
/// * `admission.staging_flush` leaves the indexed staging action directory
///   resident and exact, so every restart re-enters `PublishStagingAction` and
///   must reuse the same staging name and the same derived reservation row.
///
/// The selection is **not** an exclusivity claim, and the boundaries it leaves
/// out are named so a future reader does not mistake the choice for the set.
/// `admission.idle_scratch_create` is equally repeatable — an empty scratch
/// decodes `Other`, so `(Preparing, Other)` resolves to
/// `ReplacePreparingWithIdle` and re-enters the same helper — but it re-crosses
/// the same slot name `preparing_scratch_create` already proves stable. The two
/// observation boundaries, `admission.occupancy_observe` and
/// `admission.capacity_check`, are trivially re-crossable and mutate nothing,
/// so repeating them would prove nothing about slot stability.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let expected = reservation(0xB2, 2);
    let reservation_row =
        ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(expected.action_digest());
    let settled_rows = {
        let fixture = Fixture::new(&format!("{}-repeat-settled", variant.label()));
        settle(&fixture, variant, &expected, "baseline").1
    };

    for key in [
        Fault::AdmissionPreparingScratchCreate,
        Fault::AdmissionReservationCreate,
        Fault::AdmissionStagingFlush,
    ] {
        let stable = key.stable_key();
        let fixture = Fixture::new(&format!("{}-r-{}", variant.label(), suffix(&stable)));
        let mut census: Option<(Vec<String>, Vec<String>)> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_admission_fault(key, || panic!("simulated admission process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = attempt(&fixture, variant, &expected);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );

            let observed = (fixture.children(variant), fixture.staging_children(variant));
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let (root_rows, staging_rows) = census.expect("the boundary is crashed at least once");
        assert!(
            root_rows.len() <= settled_rows.len() + 1,
            "{stable}: the interrupted catalog grew past one in-flight row: {root_rows:?}"
        );
        assert!(
            staging_rows.is_empty() || staging_rows == [reservation_row.clone()],
            "{stable}: staging holds a name the frozen grammar did not derive: {staging_rows:?}"
        );

        let (settled_handoff, converged_rows) = settle(&fixture, variant, &expected, &stable);
        assert_eq!(settled_handoff.reservation(), &expected);
        assert_eq!(
            converged_rows, settled_rows,
            "{stable}: the catalog did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | slots-stable=yes | root-rows={} | staging-rows={:?} | converged=yes",
            variant.label(),
            root_rows.len(),
            staging_rows
        );
    }
}

#[test]
fn admission_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn admission_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}
