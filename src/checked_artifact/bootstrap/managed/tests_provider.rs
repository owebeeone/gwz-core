//! R2-D Phase 3 Step 3.1 — the production managed-parent provider, driven
//! against real durable state.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 3.1 ("`observe_preflight`
//! / `revalidate_plan` / `execute_bound` on the retained provider; per missing
//! component: staged directory, ownership marker, installed observation, ... marker
//! retirement, final reproof; restart consumes the resident intent and scheduled
//! slots, never replans a partially completed live path");
//! `GwzM5-8R2DInterfaceFreeze.md` §3.4 for the frozen seam and §4.3 rows E15/E16
//! for the two physical edges the drive crosses;
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §9 (:249-266) for the per-component
//! sequence and the opaque retained-parent proof.
//!
//! Every row runs against a real target: a real lease, the sealed catalog owner,
//! the frozen admission seam, and a fresh provider per session — so a "restart"
//! here is a fresh session over the durable state the previous one left.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py`) and out of the
//! injection-site rescan (`interface_tests/fault_expected_keys.rs`), exactly as
//! `namespace/tests_managed.rs` does.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::provider::{RetainedManagedParentProviderV1, RetainedManagedParentsV1};
use super::{
    ManagedParentBootstrapOwnerV1, ManagedParentBootstrapRequest, ManagedParentPlanV1,
    ManagedParentPurpose,
};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{CheckedFsError, DurableObjectIdentityV1};
use crate::checked_artifact::catalog::{OpaqueRetainedCatalogV1, recover_or_create};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    CleanupAliasSetV1, RecordObservationV1, RequestOwnerBindingV1, RootEntryNameV1,
    admit_observed_action, managed_staging_name,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The action every row of this suite bootstraps under. One action per fixture
/// keeps the durable state minimal while still crossing both purposes.
const ACTION: ActionDigestV1 = ActionDigestV1::new([0xB3; 32]);
const OWNER: RequestOwnerBindingV1 = RequestOwnerBindingV1::new([0x71; 32]);

/// The two frozen target variants (`GwzM5-8R2D-Plan.md` §4 Step 1.3).
#[derive(Clone, Copy, Debug)]
enum TargetVariantV1 {
    Workspace,
    GitDirectory,
}

impl TargetVariantV1 {
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
            "gwz-r2d-managed-provider-{label}-{}-{}",
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

    fn action_directory(&self, variant: TargetVariantV1) -> PathBuf {
        let base = match variant {
            TargetVariantV1::Workspace => self.root.clone(),
            TargetVariantV1::GitDirectory => git2::Repository::open(&self.root)
                .unwrap()
                .commondir()
                .to_path_buf(),
        };
        base.join(CatalogPrivateNameV1::Final.relative_path(variant.private_root()))
            .join(RootEntryNameV1::ActiveAction(ACTION).name())
    }

    fn retirement_row(&self, variant: TargetVariantV1, ordinal: u8) -> PathBuf {
        self.action_directory(variant)
            .join(ActionSlotV1::RetiredBootstrapMarker(ordinal).name(ACTION))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One fresh-process catalog session on the requested target, mirroring
/// `namespace/tests_fault_matrix.rs`'s own two arms.
fn with_catalog<T>(
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

/// The merge-start request: `MergeStore` (`.gwz/merge`, one missing component)
/// and `PreservationBundles` (`.gwz/stash/bundles`, two). One row of each depth
/// is what makes the prefix-composed path profile load-bearing — a
/// one-component profile would refuse the second component of the second row.
fn merge_start() -> ManagedParentBootstrapRequest {
    ManagedParentBootstrapRequest::for_merge_start()
}

fn preflight(
    fixture: &Fixture,
    variant: TargetVariantV1,
) -> Result<ManagedParentPlanV1, CheckedFsError> {
    with_catalog(fixture, variant, |catalog| {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        ManagedParentBootstrapOwnerV1::new(&provider).preflight(&merge_start(), ACTION, OWNER)
    })
}

fn reservation(plan: &ManagedParentPlanV1) -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ACTION,
        OWNER,
        ActionScheduleV1::try_from_managed_plan(
            0,
            plan.schedule_inputs(),
            CleanupAliasSetV1::all(),
        )
        .expect("the plan's own schedule inputs are schedulable"),
    )
}

/// The one real admission per fixture, followed by the test-only handoff a fresh
/// session rebuilds — the `namespace/tests_fault_matrix.rs` pattern, and for the
/// same reason: resuming a durable handoff is plan §4 Step 3.3's coordinator.
fn admit(
    fixture: &Fixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) -> DurableObjectIdentityV1 {
    with_catalog(fixture, variant, |catalog| {
        Ok(ActionAdmissionOwnerV1::from_retained_catalog(catalog)
            .resume_or_admit(expected)?
            .directory_identity()
            .clone())
    })
    .expect("the frozen admission seam must admit the action")
}

fn handoff(
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

/// One fresh-process bind-and-execute over the durable state a previous session
/// left. Every session re-binds the plan, which re-runs `revalidate_plan`.
fn execute(
    fixture: &Fixture,
    variant: TargetVariantV1,
    plan: &ManagedParentPlanV1,
    admitted: &AdmittedActionV1,
) -> Result<RetainedManagedParentsV1, CheckedFsError> {
    with_catalog(fixture, variant, |catalog| {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        let owner = ManagedParentBootstrapOwnerV1::new(&provider);
        let bound = owner.bind(admitted, plan)?;
        owner.execute(&bound)
    })
}

/// A bootstrapped fixture: planned, admitted, executed once.
struct Bootstrapped {
    fixture: Fixture,
    plan: ManagedParentPlanV1,
    admitted: AdmittedActionV1,
    retained: RetainedManagedParentsV1,
}

fn bootstrap(label: &str) -> Bootstrapped {
    let fixture = Fixture::new(label);
    let variant = TargetVariantV1::Workspace;
    let plan = preflight(&fixture, variant).expect("the managed prefix must be observable");
    let expected = reservation(&plan);
    let identity = admit(&fixture, variant, &expected);
    let admitted = handoff(&expected, &identity);
    let retained = execute(&fixture, variant, &plan, &admitted)
        .expect("the managed-parent bootstrap must execute");
    Bootstrapped {
        fixture,
        plan,
        admitted,
        retained,
    }
}

fn children(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .expect("the directory must exist")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn preflight_plans_only_the_missing_suffix_of_each_declared_managed_path() {
    let fixture = Fixture::new("preflight");
    let plan = preflight(&fixture, TargetVariantV1::Workspace)
        .expect("the managed prefix must be observable");

    assert_eq!(plan.rows().len(), 2);
    let store = &plan.rows()[0];
    let bundles = &plan.rows()[1];
    assert_eq!(store.purpose(), ManagedParentPurpose::MergeStore);
    assert_eq!(bundles.purpose(), ManagedParentPurpose::PreservationBundles);
    // `.gwz` is the only durably present ancestor of either path after catalog
    // recovery, so both rows retain exactly one and plan the rest.
    assert_eq!(store.retained_existing_parent_count(), 1);
    assert_eq!(bundles.retained_existing_parent_count(), 1);
    assert_eq!(store.missing_suffix().len(), 1);
    assert_eq!(bundles.missing_suffix().len(), 2);
    // The retained path profile is as deep as the retained count, which is what
    // the owner's `retained_path_matches` requires of a provider.
    assert_eq!(store.retained_parent_path().components().len(), 1);
    assert_eq!(bundles.retained_parent_path().components().len(), 1);
}

#[test]
fn execute_bound_bootstraps_every_declared_managed_parent() {
    let bootstrapped = bootstrap("execute");
    let root = bootstrapped.fixture.path();

    assert!(root.join(".gwz/merge").is_dir());
    assert!(root.join(".gwz/stash").is_dir());
    assert!(root.join(".gwz/stash/bundles").is_dir());
    // Every staged directory was consumed by its edge, and every ownership
    // marker left its component through edge E16.
    assert_eq!(children(&root.join(".gwz/merge")), Vec::<String>::new());
    assert_eq!(
        children(&root.join(".gwz/stash")),
        vec!["bundles".to_owned()]
    );
    assert_eq!(
        children(&root.join(".gwz/stash/bundles")),
        Vec::<String>::new()
    );
    // One scheduled retirement row per component, in global ordinal order.
    for ordinal in 0..3 {
        assert!(
            bootstrapped
                .fixture
                .retirement_row(TargetVariantV1::Workspace, ordinal)
                .is_file(),
            "the marker of component {ordinal} must retire into its scheduled row"
        );
    }
}

/// The two durable windows the staged-component writer itself can leave — the
/// staging row created before its marker, and a marker whose bytes never
/// finished — must converge on the next drive rather than wedge on a
/// deterministic name that can never be re-derived differently.
fn a_staging_window_converges(label: &str, seed: Option<&[u8]>) {
    let fixture = Fixture::new(label);
    let variant = TargetVariantV1::Workspace;
    let plan = preflight(&fixture, variant).expect("the managed prefix must be observable");
    let expected = reservation(&plan);
    let identity = admit(&fixture, variant, &expected);
    let admitted = handoff(&expected, &identity);

    // Component 0 is the merge store's only component, so its staging row sits
    // in `.gwz` under this action's own deterministic staging name.
    let staging = managed_staging_name(ACTION, 0).expect("component 0 is scheduled");
    let row = fixture.path().join(".gwz").join(
        String::from_utf8(staging.as_bytes().to_vec()).expect("a frozen managed name is ASCII"),
    );
    fs::create_dir(&row).expect("the staging row is creatable");
    if let Some(bytes) = seed {
        fs::write(row.join("gwz-bootstrap-owner-v1"), bytes)
            .expect("the partial marker is writable");
    }

    execute(&fixture, variant, &plan, &admitted)
        .expect("an owned staging window must converge, not wedge");

    assert!(!row.exists(), "the staged row must be consumed by its edge");
    assert!(fixture.path().join(".gwz/merge").is_dir());
}

#[test]
fn a_staging_row_left_without_its_marker_converges() {
    a_staging_window_converges("staging-window-empty", None);
}

#[test]
fn a_staging_row_left_with_a_partial_marker_converges() {
    a_staging_window_converges("staging-window-partial", Some(b"gwz-boot"));
}

/// The one thing the rewrite must *not* adopt: a staging row carrying content
/// this owner did not write. The extra child survives the marker rewrite, the
/// interior re-proof fails, and the sequence stops with nothing published.
#[test]
fn a_staging_row_carrying_a_foreign_child_is_refused() {
    let fixture = Fixture::new("staging-foreign");
    let variant = TargetVariantV1::Workspace;
    let plan = preflight(&fixture, variant).expect("the managed prefix must be observable");
    let expected = reservation(&plan);
    let identity = admit(&fixture, variant, &expected);
    let admitted = handoff(&expected, &identity);

    let staging = managed_staging_name(ACTION, 0).expect("component 0 is scheduled");
    let row = fixture.path().join(".gwz").join(
        String::from_utf8(staging.as_bytes().to_vec()).expect("a frozen managed name is ASCII"),
    );
    fs::create_dir(&row).expect("the staging row is creatable");
    fs::write(row.join("intruder"), b"x").expect("the staging row is writable");

    let refused = execute(&fixture, variant, &plan, &admitted).is_err();

    assert!(refused, "a foreign staging interior must be refused");
    assert!(
        !fixture.path().join(".gwz/merge").exists(),
        "a refused staging must publish nothing"
    );
}

#[test]
fn the_retained_parents_proof_reports_each_purpose_at_its_full_declared_depth() {
    let bootstrapped = bootstrap("proof");
    let rows = bootstrapped.retained.rows();

    assert_eq!(rows.len(), 2);
    let store = bootstrapped
        .retained
        .row(ManagedParentPurpose::MergeStore)
        .expect("the merge store is bootstrapped");
    let bundles = bootstrapped
        .retained
        .row(ManagedParentPurpose::PreservationBundles)
        .expect("the preservation bundles parent is bootstrapped");
    // The proof is the *final reproof*: a fresh bounded observation of the whole
    // declared path, so its profile is as deep as the path itself.
    assert_eq!(store.path().components().len(), 2);
    assert_eq!(bundles.path().components().len(), 3);
    assert_ne!(store.identity(), bundles.identity());
}

#[test]
fn a_settled_row_re_executes_to_the_identical_proof_without_touching_the_namespace() {
    let bootstrapped = bootstrap("settled-restart");
    let variant = TargetVariantV1::Workspace;
    let before = children(&bootstrapped.fixture.action_directory(variant));

    let again = execute(
        &bootstrapped.fixture,
        variant,
        &bootstrapped.plan,
        &bootstrapped.admitted,
    )
    .expect("a settled managed bootstrap must re-execute");

    assert_eq!(
        children(&bootstrapped.fixture.action_directory(variant)),
        before,
        "a settled re-execution must add no durable row"
    );
    for purpose in [
        ManagedParentPurpose::MergeStore,
        ManagedParentPurpose::PreservationBundles,
    ] {
        let first = bootstrapped.retained.row(purpose).expect("purpose planned");
        let second = again.row(purpose).expect("purpose replanned");
        assert_eq!(first.identity(), second.identity());
        assert_eq!(first.path(), second.path());
        assert_eq!(first.mode(), second.mode());
    }
}

/// The one window the evidence-replay drive cannot cross, pinned so that closing
/// it is a deliberate edit: a row whose retirement rows are only partly resident
/// is refused, typed, with nothing mutated.
#[test]
fn a_partially_retired_row_is_refused_rather_than_replanned() {
    let bootstrapped = bootstrap("partial-retire");
    let variant = TargetVariantV1::Workspace;
    // Component 2 is the deepest component of the two-component row.
    fs::remove_file(bootstrapped.fixture.retirement_row(variant, 2))
        .expect("the scheduled retirement row is removable");
    let before = children(&bootstrapped.fixture.path().join(".gwz/stash/bundles"));

    let refused = execute(
        &bootstrapped.fixture,
        variant,
        &bootstrapped.plan,
        &bootstrapped.admitted,
    )
    .is_err();

    assert!(refused, "a partially retired row must be refused");
    assert_eq!(
        children(&bootstrapped.fixture.path().join(".gwz/stash/bundles")),
        before,
        "a refused resume must mutate nothing"
    );
}

#[test]
fn revalidate_plan_refuses_a_substituted_retained_parent() {
    let fixture = Fixture::new("substituted");
    let variant = TargetVariantV1::Workspace;
    let plan = preflight(&fixture, variant).expect("the managed prefix must be observable");

    let current = with_catalog(&fixture, variant, |catalog| {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        use super::owner::ManagedParentBootstrap;
        provider.revalidate_plan(&plan)
    })
    .expect("revalidation must reach the durable prefix");
    assert!(current, "an untouched prefix must revalidate");

    // Substitute `.gwz` for a different directory of the same name: the recorded
    // retained parent's *identity* is what the plan bound, not its name.
    let private = fixture.path().join(".gwz");
    let moved = fixture.path().join(".gwz-moved");
    fs::rename(&private, &moved).expect("the private parent is renameable");
    fs::create_dir(&private).expect("a substitute is creatable");

    let stale = with_catalog(&fixture, variant, |catalog| {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        use super::owner::ManagedParentBootstrap;
        provider.revalidate_plan(&plan)
    });

    // The catalog itself lives under `.gwz`, so the substitution is refused
    // either by the permit's own revalidation or by the plan comparison; both are
    // the same verdict — this plan may not execute.
    assert!(
        stale.map(|current| !current).unwrap_or(true),
        "a substituted retained parent must not revalidate"
    );
}

/// The managed paths are workspace-relative by construction
/// (`ManagedParentPurpose::path`), so a Git-directory *catalog target* — whose
/// retained root is the actual Git directory — has no `.gwz` ancestor to retain
/// and refuses, typed, before any plan exists. Binding a Git-directory catalog to
/// the workspace root it belongs to is a named follow-up, not a silent fallback.
#[test]
fn a_git_directory_target_refuses_the_workspace_rooted_managed_paths() {
    let fixture = Fixture::new("git-directory");
    let refused = preflight(&fixture, TargetVariantV1::GitDirectory).is_err();
    assert!(
        refused,
        "a Git-directory target must refuse the workspace-rooted managed paths"
    );
}

#[test]
fn the_provider_instance_binds_one_target_and_separates_two() {
    let first = Fixture::new("instance-first");
    let second = Fixture::new("instance-second");
    let variant = TargetVariantV1::Workspace;

    let instance = |fixture: &Fixture| {
        with_catalog(fixture, variant, |catalog| {
            use super::owner::ManagedParentBootstrap;
            Ok(
                RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?
                    .provider_instance_id(),
            )
        })
        .expect("the provider instance is derivable")
    };

    assert_eq!(instance(&first), instance(&first));
    assert_ne!(instance(&first), instance(&second));
    assert_ne!(instance(&first), [0; 32]);
}
