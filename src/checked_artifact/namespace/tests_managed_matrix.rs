//! R2-D Step 2.3 — the executed `managed_bootstrap.*` subset's
//! interruption/restart/convergence matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.3;
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this package
//! partially flips and §4.3 rows E15/E16 for the two edges driven here;
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §8 (:228-231) for the forward *and*
//! restart observations both edges must carry, and §12 for the repeated-crash
//! rule.
//!
//! **Why only eight of the family's thirty keys.** The frozen map assigns
//! `managed_bootstrap.*` to Phase 3, but §4.3 assigns rows E15 and E16 to Step
//! 2.3, and RemPlan §10's duty binds per *edge*: the package that converts an
//! edge lands its sites and its rows. These eight are exactly the boundaries the
//! four managed backend operations cross. The other twenty-two are the managed
//! parent *provider*'s — preflight, the intent record lifecycle, staged-directory
//! and marker construction, the durable successor and prior-generation edges,
//! final intent retirement and plan completion — none of which this step writes,
//! and `interface_tests/fault_expected_keys.rs` still proves each of them
//! siteless, key by key.
//!
//! Every row drives the real backend against a real target: a real lease, the
//! sealed catalog owner, the frozen admission seam, a real managed parent beside
//! the catalog, and a *fresh* retained capability pair per attempt — so an
//! interruption is a real process stop across a real durable managed edge.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan, exactly as `tests_fault_matrix.rs` does.

use super::tests_fault_matrix::TargetVariantV1;
use super::tests_managed::{ManagedFixture, drive_managed_sequence};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_managed_fault,
};

/// Every activated `managed_bootstrap.*` boundary, in the order one virgin drive
/// crosses them.
///
/// `parent_revalidate` fires on the first managed operation of the install;
/// `staging_directory_publish` is edge E15's rename; `final_directory_reopen`
/// and `final_directory_reobserve` are its post-edge proof; `component_reobserve`
/// is the restart entry a second process takes into that same proof;
/// `marker_retire` is edge E16's rename; and `marker_retired_reobserve` with
/// `final_identity_reobserve` are its post-edge proof.
const MANAGED_MATRIX: [Fault; 8] = [
    Fault::ManagedBootstrapParentRevalidate,
    Fault::ManagedBootstrapStagingDirectoryPublish,
    Fault::ManagedBootstrapFinalDirectoryReopen,
    Fault::ManagedBootstrapFinalDirectoryReobserve,
    Fault::ManagedBootstrapComponentReobserve,
    Fault::ManagedBootstrapMarkerRetire,
    Fault::ManagedBootstrapMarkerRetiredReobserve,
    Fault::ManagedBootstrapFinalIdentityReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin managed drive settles in two durable edges and leaves one row in
/// the managed parent and one in the action directory, so twelve crashes cross
/// the nominal capacity several times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

/// **The inclusion criterion, stated once.** A boundary is *single-crossing* when
/// the durable state its crash leaves routes the next drive **past** it. Every
/// other activated boundary is repeatable and belongs in
/// [`REPEATED_BOUNDARIES`], because a restart re-enters it on the same durable
/// state — which is what ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause
/// (:1089-1092) ask to be proved: no fresh retry name, no slot growth, past
/// nominal capacity.
///
/// Note the criterion is about *routing*, not about whether the boundary is a
/// rename. An earlier revision of this file said the excluded set was "the five
/// boundaries at or after edge E16's rename"; that was false twice over —
/// `staging_directory_publish` is E15's rename and `final_directory_reobserve`
/// is E15's post-edge proof, and neither produces a retirement row — and it
/// wrongly excluded `final_directory_reobserve`, which meets the criterion for
/// inclusion. Both are corrected here (Step-2.3 review [P2-2]).
const REPEATED_BOUNDARIES: [Fault; 4] = [
    // Crash leaves the staged directory resident and the component uninstalled,
    // so every restart re-enters edge E15 and must reopen the *same* staging
    // name rather than allocate a retry name.
    Fault::ManagedBootstrapParentRevalidate,
    // Crash leaves the component installed with the marker still inside, and
    // `recover_installed_bootstrap_component` runs on every drive, so every
    // restart re-opens the same final name.
    Fault::ManagedBootstrapFinalDirectoryReopen,
    // Same durable state, one boundary later: the restart observation is the
    // path a resumed drive always takes, so it re-enters itself.
    Fault::ManagedBootstrapComponentReobserve,
    // Same durable state again, one boundary later still. Read-only, and the
    // settled short-circuit cannot fire (no retirement row yet), so the restart
    // reaches it every time — the boundary [P2-2] found missing.
    Fault::ManagedBootstrapFinalDirectoryReobserve,
];

/// The single-crossing complement, each with the routing that skips it.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 4] = [
    // E15's rename. Its completion consumes the staging name, so the next drive
    // computes `staged == false` and skips edge E15 entirely.
    Fault::ManagedBootstrapStagingDirectoryPublish,
    // E16's rename. Its completion publishes the retirement row, and the next
    // drive short-circuits on `marker_retired_row_exists()`.
    Fault::ManagedBootstrapMarkerRetire,
    // After E16's rename: the retirement row is already resident, same
    // short-circuit.
    Fault::ManagedBootstrapMarkerRetiredReobserve,
    // After E16's rename: same short-circuit.
    Fault::ManagedBootstrapFinalIdentityReobserve,
];

/// The activated subset, as stable keys, so a key added to
/// `MANAGED_BOOTSTRAP_STEP_2_3_KEYS` without a matrix row fails here rather than
/// silently escaping (the `tests_fault_matrix.rs:344-359` reconciliation, narrowed
/// to a partially-executed family).
fn reconcile_executed_keys() {
    let mut actual = MANAGED_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = [
        "managed_bootstrap.parent_revalidate",
        "managed_bootstrap.staging_directory_publish",
        "managed_bootstrap.final_directory_reopen",
        "managed_bootstrap.final_directory_reobserve",
        "managed_bootstrap.component_reobserve",
        "managed_bootstrap.marker_retire",
        "managed_bootstrap.marker_retired_reobserve",
        "managed_bootstrap.final_identity_reobserve",
    ]
    .map(str::to_owned)
    .to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

/// The two classes must partition the activated matrix exactly: no boundary in
/// both, none in neither. Without this, a key added to `MANAGED_MATRIX` could sit
/// in no class at all and the stop-clause proof would quietly lose a boundary —
/// which is the drift [P2-2] caught by reading, and which is now machine-checked.
fn assert_boundary_partition() {
    let mut union = REPEATED_BOUNDARIES
        .iter()
        .chain(SINGLE_CROSSING_BOUNDARIES.iter())
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut matrix = MANAGED_MATRIX
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

/// The settled census of both directories the managed sequence writes into.
fn settle(managed: &ManagedFixture, context: &str) -> (Vec<String>, Vec<String>) {
    drive_managed_sequence(managed)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    managed.census()
}

/// Interrupt at every activated boundary, restart, and converge — with the
/// per-key evidence line the L1-16/L2-14 form expects printed for the run tail.
fn run_managed_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    assert_boundary_partition();
    let settled = {
        let managed = ManagedFixture::new(variant, "matrix-settled");
        settle(&managed, "baseline")
    };

    for key in MANAGED_MATRIX {
        let stable = key.stable_key();
        let managed = ManagedFixture::new(variant, &format!("m-{}", suffix(&stable)));

        run_next_managed_fault(key, || panic!("simulated managed process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = drive_managed_sequence(&managed);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let resumed = settle(&managed, &stable);
        assert_eq!(
            resumed, settled,
            "{stable}: the restart did not converge to the settled managed state"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-retains the same capabilities and mutates nothing.
        let again = settle(&managed, &stable);
        assert_eq!(
            again, settled,
            "{stable}: the resume mutated the settled managed state"
        );

        println!(
            "{stable} | {} | interrupted=yes | restart=settled | managed={} action={} | resume=no-mutation",
            variant.label(),
            resumed.0.len(),
            resumed.1.len()
        );
    }
}

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
///
/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause (:1089-1092):
/// crashing the same boundary far past nominal capacity must never allocate a
/// fresh retry name and must never grow the durable slot set.
///
/// The set driven here is [`REPEATED_BOUNDARIES`], whose criterion and
/// per-boundary reasons are stated at its definition; the complement is
/// [`SINGLE_CROSSING_BOUNDARIES`], and `assert_boundary_partition` proves the two
/// cover the activated matrix exactly once each.
fn run_repeated_managed_crashes(variant: TargetVariantV1) {
    assert_boundary_partition();
    let settled = {
        let managed = ManagedFixture::new(variant, "repeat-settled");
        settle(&managed, "baseline")
    };

    for key in REPEATED_BOUNDARIES {
        let stable = key.stable_key();
        let managed = ManagedFixture::new(variant, &format!("r-{}", suffix(&stable)));
        let mut census: Option<(Vec<String>, Vec<String>)> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_managed_fault(key, || panic!("simulated managed process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = drive_managed_sequence(&managed);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );

            let observed = managed.census();
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let converged = settle(&managed, &stable);
        assert_eq!(
            converged, settled,
            "{stable}: the managed state did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | slots-stable=yes | converged=yes",
            variant.label()
        );
    }
}

#[test]
fn managed_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_managed_matrix(TargetVariantV1::Workspace);
}

#[test]
fn managed_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_managed_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_managed_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_managed_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_managed_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_managed_crashes(TargetVariantV1::GitDirectory);
}
