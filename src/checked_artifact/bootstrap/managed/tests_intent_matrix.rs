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
//! **Which fifteen keys, and why not the other seven.** Step 2.3 executed the
//! eight boundaries of edges E15/E16. These fifteen are the intent record's own
//! durable lifecycle: the initial generation, every successor, every
//! prior-generation retirement, and the final retirement that is a row's
//! completion record. The remaining seven are the five staged-component *writer*
//! keys — whose edges Step 3.1 converted and whose activation the plan assigns to
//! Step 3.2 — plus `preflight` and `plan_complete`, which name plan-level states
//! rather than durable edges and which no step has yet given a boundary.
//!
//! The row driven is `PreservationBundles` (`.gwz/stash/bundles`) alone: two
//! missing components, five scheduled generations, and therefore the only shape
//! that reaches a *mid-retirement* interruption — the window Step 3.1 could not
//! converge from.
//!
//! Every row drives the real provider against a real target: a real lease, the
//! sealed catalog owner, the frozen admission seam, and a fresh provider and
//! namespace per attempt — so an interruption is a real process stop across a
//! real durable edge.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as `namespace/tests_managed_matrix.rs`
//! does.

use std::path::PathBuf;

use super::tests_provider::{
    Fixture, TargetVariantV1, admit, children, execute, handoff, plan_for, reservation,
};
use super::{
    ManagedParentBootstrapRequest, ManagedParentPlanV1, ManagedParentPurpose,
    provider::RetainedManagedParentsV1,
};
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_intent_fault,
};
use crate::checked_artifact::protocol::AdmittedActionV1;

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

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin drive settles in five generations and leaves two managed rows and
/// two retirement rows, so twelve crashes cross the nominal capacity several
/// times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

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

/// The activated subset, as stable keys, so a key added to the fixture's
/// activated list without a matrix row fails here rather than silently escaping.
fn reconcile_executed_keys() {
    let mut actual = MANAGED_INTENT_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = [
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
    ]
    .map(str::to_owned)
    .to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

/// The two classes must partition the activated matrix exactly: no boundary in
/// both, none in neither.
fn assert_boundary_partition() {
    let mut union = REPEATED_BOUNDARIES
        .iter()
        .chain(SINGLE_CROSSING_BOUNDARIES.iter())
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut matrix = MANAGED_INTENT_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    union.sort_unstable();
    matrix.sort_unstable();
    let mut deduped = union.clone();
    deduped.dedup();
    assert_eq!(
        union, deduped,
        "a boundary is declared both repeatable and single-crossing"
    );
    assert_eq!(
        union, matrix,
        "the repeatable / single-crossing classes do not partition the activated matrix"
    );
}

fn suffix(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .1
}

/// One planned, admitted row, ready to be driven repeatedly by fresh sessions.
struct IntentFixture {
    fixture: Fixture,
    variant: TargetVariantV1,
    plan: ManagedParentPlanV1,
    admitted: AdmittedActionV1,
}

impl IntentFixture {
    fn new(variant: TargetVariantV1, label: &str) -> Self {
        let fixture = Fixture::new(&format!("intent-{label}"));
        // The catalog must exist before the managed root is placed for the
        // Git-directory arm; `plan_for` recovers it.
        fixture.prepare_managed_root(variant);
        let request = ManagedParentBootstrapRequest::try_for_durable_merge(&[
            ManagedParentPurpose::PreservationBundles,
        ])
        .expect("the durable-merge constructor admits the bundles purpose");
        let plan =
            plan_for(&fixture, variant, &request).expect("the managed prefix must be observable");
        let expected = reservation(&plan);
        let identity = admit(&fixture, variant, &expected);
        let admitted = handoff(&expected, &identity);
        Self {
            fixture,
            variant,
            plan,
            admitted,
        }
    }

    fn drive(&self) -> Result<RetainedManagedParentsV1, CheckedFsError> {
        execute(&self.fixture, self.variant, &self.plan, &self.admitted)
    }

    fn managed_root(&self) -> PathBuf {
        self.fixture.managed_root(self.variant)
    }

    /// The census of every directory this row writes into: the managed root
    /// (which holds the first component and the first staging row), the first
    /// component (which holds the second), and the action directory (which holds
    /// every intent generation and both marker-retirement rows).
    fn census(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let root = self.managed_root();
        (
            children(&root),
            children(&root.join("stash")),
            children(&self.fixture.action_directory(self.variant)),
        )
    }
}

type Census = (Vec<String>, Vec<String>, Vec<String>);

fn settle(fixture: &IntentFixture, context: &str) -> Census {
    fixture
        .drive()
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    fixture.census()
}

fn variant_label(variant: TargetVariantV1) -> &'static str {
    match variant {
        TargetVariantV1::Workspace => "workspace",
        TargetVariantV1::GitDirectory => "git-directory",
    }
}

/// Interrupt at every activated boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_intent_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    assert_boundary_partition();
    let settled = {
        let fixture = IntentFixture::new(variant, "matrix-settled");
        settle(&fixture, "baseline")
    };

    for key in MANAGED_INTENT_MATRIX {
        let stable = key.stable_key();
        let fixture = IntentFixture::new(variant, &format!("m-{}", suffix(&stable)));

        run_next_intent_fault(key, || panic!("simulated managed intent process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fixture.drive();
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let resumed = settle(&fixture, &stable);
        assert_eq!(
            resumed, settled,
            "{stable}: the restart did not converge to the settled managed state"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-reads the same resident chain and mutates nothing.
        let again = settle(&fixture, &stable);
        assert_eq!(
            again, settled,
            "{stable}: the resume mutated the settled managed state"
        );

        println!(
            "{stable} | {} | interrupted=yes | restart=settled | managed={} component={} action={} | resume=no-mutation",
            variant_label(variant),
            resumed.0.len(),
            resumed.1.len(),
            resumed.2.len()
        );
    }
}

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
fn run_repeated_intent_crashes(variant: TargetVariantV1) {
    assert_boundary_partition();
    let settled = {
        let fixture = IntentFixture::new(variant, "repeat-settled");
        settle(&fixture, "baseline")
    };

    for key in REPEATED_BOUNDARIES {
        let stable = key.stable_key();
        let fixture = IntentFixture::new(variant, &format!("r-{}", suffix(&stable)));
        let mut census: Option<Census> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_intent_fault(key, || panic!("simulated managed intent process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = fixture.drive();
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );

            let observed = fixture.census();
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let converged = settle(&fixture, &stable);
        assert_eq!(
            converged, settled,
            "{stable}: the managed state did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | slots-stable=yes | converged=yes",
            variant_label(variant)
        );
    }
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
