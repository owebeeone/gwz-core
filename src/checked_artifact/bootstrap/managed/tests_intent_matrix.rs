//! R2-D Phase 3 Step 3.1b — the executed `managed_bootstrap.*` intent-lifecycle
//! subset's interruption/restart/convergence matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 3.1 ("durable
//! successor, prior-generation retirement ... restart consumes the resident
//! intent and scheduled slots"); `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the
//! activation map this package flips a second time and §4.3 row E17 for the
//! edges driven here; `GwzM5-8R4bR2ConsumerCheckpoint.md` §9 (:249-266) for the
//! per-component sequence and §12 for the repeated-crash rule;
//! `GwzM5-8R2DStep31-Review.md` [P1-1] for the window this step closes and
//! [P2-1] for the activation duty it discharges same-commit.
//!
//! **Which fifteen keys.** Step 2.3 executed the eight boundaries of edges
//! E15/E16 and Step 3.2 the five staged-component writer keys
//! (`tests_writer_matrix.rs`). These fifteen are the intent record's own durable
//! lifecycle: the initial generation, every successor, every prior-generation
//! retirement, and the final retirement that is a row's completion record. Only
//! `preflight` and `plan_complete` remain unactivated, and their disposition is
//! the Phase 3 settle's (Step-3.1b review [P3-1]).
//!
//! The row, the census and the two runners are the shared harness in
//! `tests_provider.rs`'s `matrix` module, because Step 3.2's writer matrix is
//! crossed by the same drive; this file declares only its key set, its
//! classification and its four tests.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as `namespace/tests_managed_matrix.rs`
//! does.

use super::tests_provider::TargetVariantV1;
use super::tests_provider::matrix::{
    RowShapeV1, assert_boundary_partition, reconcile_executed_keys, run_boundary_matrix,
    run_repeated_crashes, run_single_crossing_probe,
};
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1 as Fault;

/// The row shape: `.gwz/stash/bundles`, two missing components and five
/// generations — the smallest shape that reaches a *mid-retirement*
/// interruption, which is the window this step closes.
const SHAPE: RowShapeV1 = RowShapeV1::TwoComponent;

/// Every activated intent-lifecycle boundary, in the order one virgin drive
/// crosses them.
///
/// The initial generation is published before any component is touched; each
/// component's evidence then derives a successor, which is written to the one
/// scheduled scratch row, published onto its own active row, reobserved, and its
/// predecessor retired; the last generation's retirement is the row's completion
/// record.
const MANAGED_INTENT_MATRIX: [Fault; 15] = [
    Fault::ManagedBootstrapInitialIntentScratchCreate,
    Fault::ManagedBootstrapInitialIntentScratchWrite,
    Fault::ManagedBootstrapInitialIntentScratchFlush,
    Fault::ManagedBootstrapInitialIntentPublish,
    Fault::ManagedBootstrapInitialIntentReobserve,
    Fault::ManagedBootstrapSuccessorScratchCreate,
    Fault::ManagedBootstrapSuccessorScratchWrite,
    Fault::ManagedBootstrapSuccessorScratchFlush,
    Fault::ManagedBootstrapSuccessorScratchReobserve,
    Fault::ManagedBootstrapSuccessorPublish,
    Fault::ManagedBootstrapSuccessorReobserve,
    Fault::ManagedBootstrapPriorGenerationRetire,
    Fault::ManagedBootstrapPriorGenerationReobserve,
    Fault::ManagedBootstrapFinalIntentRetire,
    Fault::ManagedBootstrapFinalIntentRetiredReobserve,
];

/// **The inclusion criterion, unchanged from `namespace/tests_managed_matrix.rs`.**
/// A boundary is *single-crossing* when the durable state its crash leaves routes
/// the next drive **past** it; every other boundary is repeatable, because a
/// restart re-enters it on the same durable state.
///
/// Thirteen of the fifteen are repeatable, and that is a property of the design
/// rather than an accident: every generation step is *guarded by residency and
/// then observed unconditionally*, so a restart past the guarded edge still
/// re-crosses both of that edge's observation boundaries. That is what makes the
/// lifecycle idempotent.
const REPEATED_BOUNDARIES: [Fault; 13] = [
    // Crash leaves a scratch row and no active generation, so the next drive
    // rewrites the *same* scratch slot rather than allocating a retry name.
    Fault::ManagedBootstrapInitialIntentScratchCreate,
    Fault::ManagedBootstrapInitialIntentScratchWrite,
    Fault::ManagedBootstrapInitialIntentScratchFlush,
    // Crash leaves the initial generation active and unobserved; the resume finds
    // it, skips the publish, and re-enters the observation.
    Fault::ManagedBootstrapInitialIntentPublish,
    Fault::ManagedBootstrapInitialIntentReobserve,
    // Same shape one generation on: the component's own work is already durable,
    // so the resume replays its restart observation and re-derives the same
    // successor into the same scratch slot.
    Fault::ManagedBootstrapSuccessorScratchCreate,
    Fault::ManagedBootstrapSuccessorScratchWrite,
    Fault::ManagedBootstrapSuccessorScratchFlush,
    Fault::ManagedBootstrapSuccessorScratchReobserve,
    // Crash leaves two active generations — the successor published, its
    // predecessor not yet retired. The resume takes the newer and re-enters both
    // observations, which is exactly how the pending retirement gets done.
    Fault::ManagedBootstrapSuccessorPublish,
    Fault::ManagedBootstrapSuccessorReobserve,
    // Crash leaves the predecessor retired and unobserved; the retirement is
    // skipped on the next drive and the observation re-entered.
    Fault::ManagedBootstrapPriorGenerationRetire,
    Fault::ManagedBootstrapPriorGenerationReobserve,
];

/// The single-crossing complement, each with the routing that skips it.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 2] = [
    // The final retirement is the row's completion record. Once it is durable no
    // active generation remains, so the resume classifies the row settled and the
    // whole drive short-circuits to the final reproof.
    Fault::ManagedBootstrapFinalIntentRetire,
    Fault::ManagedBootstrapFinalIntentRetiredReobserve,
];

/// The activated subset, as stable keys, checked against the matrix.
const EXPECTED_KEYS: [&str; 15] = [
    "managed_bootstrap.initial_intent_scratch_create",
    "managed_bootstrap.initial_intent_scratch_write",
    "managed_bootstrap.initial_intent_scratch_flush",
    "managed_bootstrap.initial_intent_publish",
    "managed_bootstrap.initial_intent_reobserve",
    "managed_bootstrap.successor_scratch_create",
    "managed_bootstrap.successor_scratch_write",
    "managed_bootstrap.successor_scratch_flush",
    "managed_bootstrap.successor_scratch_reobserve",
    "managed_bootstrap.successor_publish",
    "managed_bootstrap.successor_reobserve",
    "managed_bootstrap.prior_generation_retire",
    "managed_bootstrap.prior_generation_reobserve",
    "managed_bootstrap.final_intent_retire",
    "managed_bootstrap.final_intent_retired_reobserve",
];

fn run_intent_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys(&MANAGED_INTENT_MATRIX, &EXPECTED_KEYS);
    assert_boundary_partition(
        &MANAGED_INTENT_MATRIX,
        &REPEATED_BOUNDARIES,
        &SINGLE_CROSSING_BOUNDARIES,
    );
    run_boundary_matrix(variant, SHAPE, &MANAGED_INTENT_MATRIX);
    run_single_crossing_probe(variant, SHAPE, &SINGLE_CROSSING_BOUNDARIES);
}

fn run_repeated_intent_crashes(variant: TargetVariantV1) {
    assert_boundary_partition(
        &MANAGED_INTENT_MATRIX,
        &REPEATED_BOUNDARIES,
        &SINGLE_CROSSING_BOUNDARIES,
    );
    run_repeated_crashes(variant, SHAPE, &REPEATED_BOUNDARIES);
}

#[test]
fn managed_intent_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_intent_matrix(TargetVariantV1::Workspace);
}

#[test]
fn managed_intent_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_intent_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_intent_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_intent_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_intent_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_intent_crashes(TargetVariantV1::GitDirectory);
}
