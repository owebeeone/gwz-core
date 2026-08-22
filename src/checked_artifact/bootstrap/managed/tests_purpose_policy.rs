//! R2-D Phase 3 Step 3.2 — the purpose policy matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 3.2, which defines
//! this matrix as "all four purposes
//! (`MergeStore`/`MergeArchive`/`PreservationBundles`/`RootPreservationMarkers`),
//! overlap rejection (a missing `.gwz/merge` is `MergeStore`; `MergeArchive`
//! legal only with an existing `.gwz/merge` — ConsumerCheckpoint §9 :253-256),
//! every component/generation/marker boundary, repeated-crash slot stability".
//! The last two clauses are the two fault matrices beside this file
//! (`tests_writer_matrix.rs`, `tests_intent_matrix.rs`); this file is the first
//! two.
//!
//! **Why this is not the synthetic-provider test that already exists.**
//! `interface_tests/managed_plan_binding.rs` proves the *owner* enforces the
//! policy against a provider that reports whatever retained count it is told to.
//! Here the counts come from the real provider's bounded walk over real durable
//! state, so what is under test is the composition: that the physical
//! observation of a workspace produces exactly the classification
//! ConsumerCheckpoint §9 requires, and that the resulting plan bootstraps.
//!
//! Every purpose is reached through a **production** request constructor —
//! `for_merge_start`, `try_for_durable_merge`, `for_archive` — never a synthetic
//! one, because which purposes may travel together is itself part of the policy.

use super::tests_provider::{
    Fixture, OWNER, TargetVariantV1, admit, execute, handoff, merge_start, plan_for, reservation,
};
use super::{
    ManagedParentBootstrapRequest, ManagedParentPlanV1, ManagedParentPurpose,
    ValidatedArchiveSourceV1, provider::RetainedManagedParentsV1,
};
use crate::checked_artifact::capability::CheckedFsError;

/// The archive prerequisite an `Archive` request must carry: an exact durable
/// merge-store record owned by this request's own owner binding. Only the owner
/// binding and non-emptiness are policy-bearing (`validate_authority`), so the
/// digest is a fixed nonzero constant.
fn archive_prerequisite() -> ValidatedArchiveSourceV1 {
    ValidatedArchiveSourceV1::from_exact_record_owner(OWNER, [0x5A; 32])
        .expect("a nonzero source record digest is a valid prerequisite")
}

/// One planned purpose set over a target whose declared prefixes exist.
struct PurposeRun {
    fixture: Fixture,
    variant: TargetVariantV1,
    plan: Result<ManagedParentPlanV1, CheckedFsError>,
}

impl PurposeRun {
    /// `prefixes` are the declared path components that must already exist for
    /// the purposes under test — `.gwz` for the merge family (which a
    /// Git-directory target does not have), `gwz.conf` for the root-marker
    /// family, and `.gwz/merge` when the case is an archive over an existing
    /// merge store.
    fn plan(
        variant: TargetVariantV1,
        label: &str,
        request: &ManagedParentBootstrapRequest,
        prefixes: &[&str],
    ) -> Self {
        let fixture = Fixture::new(&format!("purpose-{label}"));
        for prefix in prefixes {
            fixture.prepare_prefix(variant, prefix);
        }
        let plan = plan_for(&fixture, variant, request);
        Self {
            fixture,
            variant,
            plan,
        }
    }

    fn planned(&self) -> &ManagedParentPlanV1 {
        self.plan
            .as_ref()
            .expect("the declared purposes must plan against this target")
    }

    /// Admits the plan's own schedule and executes it, the production sequence.
    fn bootstrap(&self) -> RetainedManagedParentsV1 {
        let plan = self.planned();
        let expected = reservation(plan);
        let identity = admit(&self.fixture, self.variant, &expected);
        let admitted = handoff(&expected, &identity);
        execute(&self.fixture, self.variant, plan, &admitted)
            .expect("the declared purposes must bootstrap")
    }

    fn exists(&self, relative: &str) -> bool {
        let mut path = self.fixture.target_root(self.variant);
        for component in relative.split('/') {
            path = path.join(component);
        }
        path.is_dir()
    }
}

/// The merge-start pair, which is the only production route to `MergeStore`:
/// both parents are bootstrapped, and the plan records the declared purposes in
/// canonical order with their mask.
fn merge_start_bootstraps_its_pair(variant: TargetVariantV1) {
    let run = PurposeRun::plan(variant, "merge-start", &merge_start(), &[".gwz"]);
    let plan = run.planned();

    assert_eq!(
        plan.declared_purposes(),
        [
            ManagedParentPurpose::MergeStore,
            ManagedParentPurpose::PreservationBundles
        ]
    );
    // `MergeStore` is bit 1 and `PreservationBundles` bit 4 of the frozen mask.
    assert_eq!(plan.declared_purpose_mask(), 1 | 4);

    let retained = run.bootstrap();
    assert!(run.exists(".gwz/merge"));
    assert!(run.exists(".gwz/stash/bundles"));
    assert_eq!(
        retained
            .row(ManagedParentPurpose::MergeStore)
            .expect("the merge store is bootstrapped")
            .path()
            .components()
            .len(),
        2
    );
    assert_eq!(
        retained
            .row(ManagedParentPurpose::PreservationBundles)
            .expect("the bundles parent is bootstrapped")
            .path()
            .components()
            .len(),
        3
    );
}

/// `RootPreservationMarkers` is the one purpose outside the `.gwz` subtree; its
/// declared path is `gwz.conf/markers`, so the policy's minimum retained count of
/// one is satisfied by `gwz.conf` and nothing about `.gwz` matters.
fn root_preservation_markers_bootstraps_outside_the_private_root(variant: TargetVariantV1) {
    let request = ManagedParentBootstrapRequest::try_for_durable_merge(&[
        ManagedParentPurpose::RootPreservationMarkers,
    ])
    .expect("the durable-merge constructor admits the root-marker purpose");
    let run = PurposeRun::plan(variant, "root-markers", &request, &["gwz.conf"]);

    assert_eq!(run.planned().rows().len(), 1);
    assert_eq!(run.planned().rows()[0].missing_suffix().len(), 1);

    let retained = run.bootstrap();
    assert!(run.exists("gwz.conf/markers"));
    assert_eq!(
        retained
            .row(ManagedParentPurpose::RootPreservationMarkers)
            .expect("the root-marker parent is bootstrapped")
            .path()
            .components()
            .len(),
        2
    );
}

/// ConsumerCheckpoint §9 (:253-256), the negative half: `MergeArchive`
/// (`.gwz/merge/done`) is legal **only** over an existing merge store. With
/// `.gwz` present and `.gwz/merge` absent the provider observes a retained count
/// of one, below the purpose's minimum of two, and the owner refuses — so a
/// missing `.gwz/merge` can only ever be `MergeStore`'s to create.
fn merge_archive_is_refused_without_an_existing_merge_store(variant: TargetVariantV1) {
    let request = ManagedParentBootstrapRequest::for_archive(archive_prerequisite());
    let run = PurposeRun::plan(variant, "archive-refused", &request, &[".gwz"]);

    // Pin the *reason*, not just the failure: this test exists to discharge the
    // ownership-policy clause, and a bare `is_err()` would pass for a lease
    // failure or a bad prerequisite without anyone noticing (Step-3.2 review
    // [P3-3]). The refusal must be the owner's purpose-ownership one, raised
    // because the provider observed a retained count below this purpose's
    // minimum of two.
    match run.plan.as_ref() {
        Err(CheckedFsError::Ambiguous { fact, detail }) => {
            assert_eq!(*fact, "managed-parent plan");
            assert_eq!(
                detail,
                "provider retained prefix violates the purpose ownership policy"
            );
        }
        Err(other) => panic!("the archive refusal is not the ownership policy: {other:?}"),
        Ok(_) => panic!("an archive over a missing merge store must be refused"),
    }
    assert!(
        !run.exists(".gwz/merge"),
        "a refused archive plan must create nothing"
    );
}

/// The positive half of the same clause: with the merge store resident the
/// archive plans exactly its own `done` component and nothing above it.
fn merge_archive_plans_only_its_own_component_over_a_resident_store(variant: TargetVariantV1) {
    let request = ManagedParentBootstrapRequest::for_archive(archive_prerequisite());
    let run = PurposeRun::plan(variant, "archive", &request, &[".gwz/merge"]);
    let plan = run.planned();

    assert_eq!(plan.rows().len(), 1);
    assert_eq!(plan.rows()[0].purpose(), ManagedParentPurpose::MergeArchive);
    assert_eq!(plan.rows()[0].retained_existing_parent_count(), 2);
    assert_eq!(plan.rows()[0].missing_suffix().len(), 1);

    let retained = run.bootstrap();
    assert!(run.exists(".gwz/merge/done"));
    assert_eq!(
        retained
            .row(ManagedParentPurpose::MergeArchive)
            .expect("the archive parent is bootstrapped")
            .path()
            .components()
            .len(),
        3
    );
}

/// A purpose whose declared path is already fully present plans **no row**: the
/// plan is proof-only, which is what stops a second bootstrap of the same
/// purposes from reserving capacity it would never use.
fn a_fully_present_purpose_set_plans_no_row(variant: TargetVariantV1) {
    let run = PurposeRun::plan(variant, "proof-only", &merge_start(), &[".gwz"]);
    run.bootstrap();

    let again = plan_for(&run.fixture, variant, &merge_start())
        .expect("a settled workspace still preflights");
    assert!(
        again.is_proof_only(),
        "a fully present purpose set must plan no row"
    );
    assert!(again.rows().is_empty());
}

#[test]
fn merge_start_bootstraps_its_pair_on_a_workspace_target() {
    merge_start_bootstraps_its_pair(TargetVariantV1::Workspace);
}

#[test]
fn merge_start_bootstraps_its_pair_on_a_git_directory_target() {
    merge_start_bootstraps_its_pair(TargetVariantV1::GitDirectory);
}

#[test]
fn root_preservation_markers_bootstraps_on_a_workspace_target() {
    root_preservation_markers_bootstraps_outside_the_private_root(TargetVariantV1::Workspace);
}

#[test]
fn root_preservation_markers_bootstraps_on_a_git_directory_target() {
    root_preservation_markers_bootstraps_outside_the_private_root(TargetVariantV1::GitDirectory);
}

#[test]
fn merge_archive_is_refused_without_a_merge_store_on_a_workspace_target() {
    merge_archive_is_refused_without_an_existing_merge_store(TargetVariantV1::Workspace);
}

#[test]
fn merge_archive_is_refused_without_a_merge_store_on_a_git_directory_target() {
    merge_archive_is_refused_without_an_existing_merge_store(TargetVariantV1::GitDirectory);
}

#[test]
fn merge_archive_bootstraps_over_a_resident_store_on_a_workspace_target() {
    merge_archive_plans_only_its_own_component_over_a_resident_store(TargetVariantV1::Workspace);
}

#[test]
fn merge_archive_bootstraps_over_a_resident_store_on_a_git_directory_target() {
    merge_archive_plans_only_its_own_component_over_a_resident_store(TargetVariantV1::GitDirectory);
}

#[test]
fn a_fully_present_purpose_set_plans_no_row_on_a_workspace_target() {
    a_fully_present_purpose_set_plans_no_row(TargetVariantV1::Workspace);
}

#[test]
fn a_fully_present_purpose_set_plans_no_row_on_a_git_directory_target() {
    a_fully_present_purpose_set_plans_no_row(TargetVariantV1::GitDirectory);
}
