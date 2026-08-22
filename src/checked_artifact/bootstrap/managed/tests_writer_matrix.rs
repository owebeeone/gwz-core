//! R2-D Phase 3 Step 3.2 — the executed `managed_bootstrap.*` staged-component
//! writer subset's interruption/restart/convergence matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 3.2
//! (`managed_bootstrap.*` activation + purpose policy matrix, "every
//! component/generation/marker boundary, repeated-crash slot stability");
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5's deferral record and its Step-3.1b
//! supersession clause, which name exactly these five keys and the 23 → 28 list
//! edit this step owes; `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 for the
//! repeated-crash rule.
//!
//! **Which five keys, and whose edges they are.** Step 3.1's `stage_component`
//! and `write_or_rewrite_marker`
//! (`capability/pre_catalog/provider/managed_mutation.rs`) converted the edges of
//! `staging_directory_create`, `ownership_marker_create`, `ownership_marker_write`,
//! `ownership_marker_flush` and `staging_directory_flush` *without* injection
//! sites, deliberately and on the record. This step lands the sites and these
//! rows, closing that split. It flips no other key: with these five the family
//! stands at 28 executed and 2 reserved, and `preflight` / `plan_complete` are
//! left exactly as they were, their disposition being the Phase 3 settle's
//! (Step-3.1b review [P3-1]).
//!
//! The row, the census and the two runners are the shared harness in
//! `tests_provider.rs`'s `matrix` module — one drive of one `PreservationBundles`
//! row crosses these five and Step 3.1b's fifteen alike, so the fixture is
//! written once.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as
//! `namespace/tests_managed_matrix.rs` does.

use super::tests_provider::TargetVariantV1;
use super::tests_provider::matrix::{
    RowShapeV1, assert_boundary_partition, reconcile_executed_keys, run_boundary_matrix,
    run_repeated_crashes, run_single_crossing_probe,
};
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1 as Fault;

/// The row shape: `gwz.conf/markers`, one missing component.
///
/// Every boundary in this matrix sits inside `stage_component`, which runs
/// **once per component**. On a two-component row a crash at the first
/// component's crossing leaves the second component's crossing ahead, so the
/// boundary is re-entered once more and then never again — neither repeatable
/// nor single-crossing under the stated criterion. A one-component row crosses
/// each boundary exactly once per drive, which is the shape the criterion is
/// written for. (The probe below is what caught this: the classification these
/// constants carried on a two-component row was wrong, and it failed loudly
/// rather than passing quietly.)
const SHAPE: RowShapeV1 = RowShapeV1::OneComponent;

/// Every activated writer boundary, in the order one virgin drive crosses them.
///
/// The staging directory is created inside the managed parent, its ownership
/// marker is opened, written and flushed, and the two directory flushes that
/// make the row durable follow — the staged interior's first, then the managed
/// parent's.
const MANAGED_WRITER_MATRIX: [Fault; 5] = [
    Fault::ManagedBootstrapStagingDirectoryCreate,
    Fault::ManagedBootstrapOwnershipMarkerCreate,
    Fault::ManagedBootstrapOwnershipMarkerWrite,
    Fault::ManagedBootstrapOwnershipMarkerFlush,
    Fault::ManagedBootstrapStagingDirectoryFlush,
];

/// **The inclusion criterion, unchanged from the two matrices before this one.**
/// A boundary is *single-crossing* when the durable state its crash leaves routes
/// the next drive **past** it; every other boundary is repeatable, because a
/// restart re-enters it on the same durable state.
///
/// This writer inverts the usual proportion — one repeatable, four
/// single-crossing — and that is exactly what its write-or-rewrite shape
/// predicts. Each step of the sequence *completes* a piece of the staged row, and
/// `stage_component` re-enters only the pieces that are still missing, so all but
/// one crash lands on a state the next drive is routed past. The one exception is
/// the boundary that leaves the marker present but empty.
const REPEATED_BOUNDARIES: [Fault; 1] = [
    // Crash immediately after the marker file is opened: the row exists, the
    // marker exists and is empty, so the interior is *not* exact and the next
    // drive re-enters `write_or_rewrite_marker` — opening the same frozen marker
    // name again, now as a rewrite. No retry name is allocated, and the staged
    // row's cardinality cannot grow.
    Fault::ManagedBootstrapOwnershipMarkerCreate,
];

/// The single-crossing complement, each with the routing that skips it.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 4] = [
    // The create is reached only when the staging row is absent. Its completion
    // makes the row resident, so the next drive takes the `Ok(_) => false` arm
    // and never calls `create_dir` again.
    Fault::ManagedBootstrapStagingDirectoryCreate,
    // After `write_all` the marker's bytes are the expected ones, so the next
    // drive's interior observation is exact and skips the whole rewrite. (The
    // durability of those bytes is what the flush below adds; a simulated process
    // stop is not a power loss, and the freeze's own E9/E16 records are where the
    // power-loss reasoning lives.)
    Fault::ManagedBootstrapOwnershipMarkerWrite,
    // Same state, one boundary later: exact interior, rewrite skipped.
    Fault::ManagedBootstrapOwnershipMarkerFlush,
    // Both instances of this key sit at or after the point where the interior is
    // already exact — the staged interior's flush ends the rewrite, and the
    // managed parent's flush runs only on a creating drive. Either way the next
    // drive finds the row staged and enters neither.
    Fault::ManagedBootstrapStagingDirectoryFlush,
];

/// The activated subset, as stable keys, checked against the matrix.
const EXPECTED_KEYS: [&str; 5] = [
    "managed_bootstrap.staging_directory_create",
    "managed_bootstrap.ownership_marker_create",
    "managed_bootstrap.ownership_marker_write",
    "managed_bootstrap.ownership_marker_flush",
    "managed_bootstrap.staging_directory_flush",
];

fn run_writer_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys(&MANAGED_WRITER_MATRIX, &EXPECTED_KEYS);
    assert_boundary_partition(
        &MANAGED_WRITER_MATRIX,
        &REPEATED_BOUNDARIES,
        &SINGLE_CROSSING_BOUNDARIES,
    );
    run_boundary_matrix(variant, SHAPE, &MANAGED_WRITER_MATRIX);
    run_single_crossing_probe(variant, SHAPE, &SINGLE_CROSSING_BOUNDARIES);
}

fn run_repeated_writer_crashes(variant: TargetVariantV1) {
    assert_boundary_partition(
        &MANAGED_WRITER_MATRIX,
        &REPEATED_BOUNDARIES,
        &SINGLE_CROSSING_BOUNDARIES,
    );
    run_repeated_crashes(variant, SHAPE, &REPEATED_BOUNDARIES);
}

#[test]
fn managed_writer_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_writer_matrix(TargetVariantV1::Workspace);
}

#[test]
fn managed_writer_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_writer_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_writer_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_writer_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_writer_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_writer_crashes(TargetVariantV1::GitDirectory);
}
