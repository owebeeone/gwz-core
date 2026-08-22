//! R2-D Step 2.3 — the four managed backend operations, driven against real
//! durable state.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.3 ("the four
//! defaults become required and implemented — forward and restart observations
//! for component installation and marker retirement");
//! `GwzM5-8R2DInterfaceFreeze.md` §3.2 for the frozen signatures, §4.3 rows E15
//! and E16 for the two physical edges, and §4.4 Class 1 for the managed
//! source-interior arm E15 needs; `GwzM5-8R4bR2ConsumerCheckpoint.md` §8
//! (:228-231) for "both forward and restart observations" and §9 (:258-262) for
//! the per-component sequence this step owns two steps of.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! and out of the injection-site rescan
//! (`interface_tests/fault_expected_keys.rs`), exactly as
//! `namespace/tests_fault_matrix.rs` does.

use std::fs;
use std::path::{Path, PathBuf};

use super::host::ActionNamespaceHandleV1;
use super::tests_fault_matrix::{Fixture, TargetVariantV1, handoff, with_catalog};
use super::{
    ActionNamespace, BootstrapComponentSlots, HostActionNamespaceV1, retain_action_namespace,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    retain_managed_parent_at_for_test,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1,
    BootstrapOrdinalV1, CleanupAliasSetV1, ManagedBootstrapInputV1, ManagedParentBootstrapIntentV1,
    OwnershipMarkerV1, RequestOwnerBindingV1, managed_marker_name,
};

type ManagedSlots = BootstrapComponentSlots<
    ActionNamespaceHandleV1,
    DurableObjectIdentityV1,
    CanonicalPathIdentityV1,
>;

/// The managed parent every row installs into: one real directory outside the
/// catalog, retained through the provider owner exactly as a Phase-3 provider
/// will retain it.
const MANAGED_PARENT_LEAF: &str = "managed-parent";

/// The component this step installs. One component keeps the fixture's durable
/// state minimal while still crossing both edges.
const COMPONENT_LEAF: &[u8] = b"merge";

/// The managed spec this fixture's schedule reserves capacity for.
const MANAGED_SPEC: [u8; 32] = [3; 32];

/// A reservation whose schedule reserves exactly one managed component, so the
/// intent this fixture builds matches it. `tests_fault_matrix::reservation`
/// reserves two, which is right for that matrix and wrong for this one: an
/// intent must cover its schedule's whole component count before it can reach
/// `RetireMarkers`, and this step's second edge only exists in that phase.
fn managed_reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([0xE5; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            0,
            vec![ManagedBootstrapInputV1::new(MANAGED_SPEC, 1).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

pub(super) struct ManagedFixture {
    fixture: Fixture,
    variant: TargetVariantV1,
    expected: ActionCapacityReservationV1,
    identity: DurableObjectIdentityV1,
    parent_path: PathBuf,
}

impl ManagedFixture {
    pub(super) fn new(variant: TargetVariantV1, label: &str) -> Self {
        let fixture = Fixture::new(&format!("{}-{label}", variant.label()));
        let expected = managed_reservation();
        let identity = fixture.admit(variant, &expected);
        let parent_path = fixture.path().join(MANAGED_PARENT_LEAF);
        fs::create_dir(&parent_path).expect("the managed parent is creatable");
        Self {
            fixture,
            variant,
            expected,
            identity,
            parent_path,
        }
    }

    /// The intent the backend binds against, built from the *real* observed
    /// identity and path profile of the managed parent on disk, so
    /// `matches_component_parent` compares durable facts rather than fixtures.
    fn intent(&self) -> ManagedParentBootstrapIntentV1 {
        let retained = retain_managed_parent_at_for_test(
            self.fixture.path(),
            MANAGED_PARENT_LEAF,
            self.expected.record_digest(),
        )
        .expect("the managed parent is retainable");
        ManagedParentBootstrapIntentV1::try_initial_for_test(
            &self.expected,
            MANAGED_SPEC,
            BootstrapOrdinalV1::new(0).unwrap(),
            retained.identity().clone(),
            retained.parent_mode(),
            retained.path_profile().clone(),
            vec![AsciiComponent::parse(COMPONENT_LEAF).unwrap()],
            [4; 32],
        )
        .expect("the managed intent binds the reservation")
    }

    /// Writes the staged component directory the install edge consumes: one
    /// directory holding exactly the ownership marker record. This is Phase
    /// 3.1's writer half (`managed_bootstrap.staging_directory_create` and the
    /// three `ownership_marker_*` boundaries), placed by the fixture so this
    /// step interrupts only the two edges it owns.
    fn stage(&self, intent: &ManagedParentBootstrapIntentV1) -> OwnershipMarkerV1 {
        let marker = OwnershipMarkerV1::for_current_component(intent)
            .expect("the install intent issues its marker");
        let staging = self
            .parent_path
            .join(name(intent.components()[0].staging_name()));
        if !staging.exists() {
            fs::create_dir(&staging).expect("the staging directory is creatable");
            fs::write(
                staging.join(name(&managed_marker_name())),
                marker.encode_canonical(),
            )
            .expect("the ownership marker is writable");
        }
        marker
    }

    fn installed_path(&self) -> PathBuf {
        self.parent_path
            .join(String::from_utf8(COMPONENT_LEAF.to_vec()).unwrap())
    }

    fn staging_path(&self, intent: &ManagedParentBootstrapIntentV1) -> PathBuf {
        self.parent_path
            .join(name(intent.components()[0].staging_name()))
    }
}

/// The census of both directories the managed sequence writes into: the managed
/// parent (staged / installed component) and the action directory (the scheduled
/// `RetiredBootstrapMarker` row). Convergence is asserted over both, because the
/// two edges of this step land in different directories.
impl ManagedFixture {
    pub(super) fn census(&self) -> (Vec<String>, Vec<String>) {
        (
            children(&self.parent_path),
            children(
                &self
                    .fixture
                    .action_directory(self.variant, self.expected.action_digest()),
            ),
        )
    }

    /// Whether the scheduled retirement row is resident.
    ///
    /// The row is named by the *whole* slot grammar — action prefix, digest,
    /// grammar prefix, ordinal, version suffix — so it is derived here rather
    /// than probed by substring, which would silently never match.
    fn marker_retired_row_exists(&self) -> bool {
        let action = self.expected.action_digest();
        self.fixture
            .action_directory(self.variant, action)
            .join(ActionSlotV1::RetiredBootstrapMarker(0).name(action))
            .exists()
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

/// One fresh-process managed drive of the whole Step-2.3 sequence, resumed from
/// whatever durable state the previous attempt left.
///
/// The shape mirrors `tests_fault_matrix.rs:288-321`: the settled state short
/// circuits first, then each half of the sequence is entered only if its durable
/// row is not already there. Every name is intent- and schedule-derived, so a
/// resumed attempt reuses the same deterministic names and never allocates a
/// retry name.
///
/// The install's restart observation is crossed on *every* drive, not only on a
/// resumed one: ConsumerCheckpoint §9 (:258-262) puts an "installed observation"
/// between the install and the marker retirement, and running it unconditionally
/// is what makes that boundary a real one rather than a path only a crash
/// reaches.
pub(super) fn drive_managed_sequence(managed: &ManagedFixture) -> Result<(), CheckedFsError> {
    let intent = managed.intent();
    if !managed.installed_path().exists() && !managed.marker_retired_row_exists() {
        managed.stage(&intent);
    }
    let staged = managed.staging_path(&intent).exists();
    with_managed(managed, |namespace, slots| {
        if managed.marker_retired_row_exists() {
            return Ok(());
        }
        if staged {
            let source = namespace.retain_managed_staging_source(&intent, slots)?;
            namespace.install_bootstrap_component(&source, &intent, slots)?;
        }
        let installed = namespace.recover_installed_bootstrap_component(&intent, slots)?;
        let retirement = intent
            .successor_after_component(&installed)
            .map_err(|_| binding_failure("managed install evidence does not close the intent"))?;
        let source = namespace.retain_managed_marker_source(slots)?;
        namespace.retire_bootstrap_marker(&source, &retirement, slots)?;
        Ok(())
    })
}

fn binding_failure(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed matrix", detail)
}

fn name(component: &AsciiComponent) -> String {
    String::from_utf8(component.as_bytes().to_vec()).expect("a frozen managed name is ASCII")
}

/// One fresh-process managed session: retain the action namespace through the
/// permit, retain the managed parent through the provider owner, and hand both
/// to `body`.
fn with_managed<T>(
    managed: &ManagedFixture,
    body: impl FnOnce(
        &mut ActionNamespace<HostActionNamespaceV1>,
        &ManagedSlots,
    ) -> Result<T, CheckedFsError>,
) -> Result<T, CheckedFsError> {
    with_catalog(&managed.fixture, managed.variant, |catalog| {
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(&catalog, handoff(&managed.expected, &managed.identity))?;
        let retained = retain_managed_parent_at_for_test(
            managed.fixture.path(),
            MANAGED_PARENT_LEAF,
            managed.expected.record_digest(),
        )?;
        let slots = namespace.retain_managed_component_slots(
            retained,
            0,
            0,
            AsciiComponent::parse(COMPONENT_LEAF).unwrap(),
        )?;
        body(&mut namespace, &slots)
    })
}

/// Edge E15, end to end: the staged directory becomes the final one, the marker
/// travels inside it, and the observation the backend returns is the durable
/// truth rather than a seeded fact.
fn install_publishes_the_staged_component(variant: TargetVariantV1) {
    let managed = ManagedFixture::new(variant, "install");
    let intent = managed.intent();
    managed.stage(&intent);

    let evidence = with_managed(&managed, |namespace, slots| {
        let source = namespace.retain_managed_staging_source(&intent, slots)?;
        namespace.install_bootstrap_component(&source, &intent, slots)
    })
    .expect("the managed install must publish the staged component");

    assert!(
        !managed.staging_path(&intent).exists(),
        "the staged directory must be consumed by the edge"
    );
    assert!(
        managed
            .installed_path()
            .join(name(&managed_marker_name()))
            .is_file(),
        "the ownership marker must travel inside the installed component"
    );
    assert_eq!(
        evidence.final_leaf(),
        &AsciiComponent::parse(COMPONENT_LEAF).unwrap()
    );
    // The mode is the managed parent's own observed one — case-folding on a
    // case-insensitive volume — not a constant, so this asserts agreement with
    // the durable observation rather than a platform assumption.
    let observed_mode = retain_managed_parent_at_for_test(
        managed.fixture.path(),
        MANAGED_PARENT_LEAF,
        managed.expected.record_digest(),
    )
    .expect("the managed parent is retainable")
    .parent_mode();
    assert_eq!(evidence.installed_mode(), observed_mode);
}

/// The restart half of the same edge (ConsumerCheckpoint §8 :228-231): a fresh
/// process that finds the component already installed must reproduce the same
/// evidence without touching the namespace.
fn restart_reobserves_the_installed_component(variant: TargetVariantV1) {
    let managed = ManagedFixture::new(variant, "install-restart");
    let intent = managed.intent();
    managed.stage(&intent);

    let forward = with_managed(&managed, |namespace, slots| {
        let source = namespace.retain_managed_staging_source(&intent, slots)?;
        namespace.install_bootstrap_component(&source, &intent, slots)
    })
    .expect("the managed install must publish the staged component");

    let restart = with_managed(&managed, |namespace, slots| {
        namespace.recover_installed_bootstrap_component(&intent, slots)
    })
    .expect("a restart must reobserve the installed component");

    assert_eq!(restart.installed_identity(), forward.installed_identity());
    assert_eq!(
        restart.marker_object_identity(),
        forward.marker_object_identity()
    );
    assert_eq!(restart.installed_path(), forward.installed_path());
}

/// Edge E16, end to end: the marker retires *out of* the installed component
/// and into the action directory's scheduled retirement slot. The marker is a
/// regular file, which is why §4.3's E16 annotation ("needs a destination arm
/// only if the marker retires as a directory") resolves to no arm.
fn marker_retirement_moves_the_marker_into_the_action_slot(variant: TargetVariantV1) {
    let managed = ManagedFixture::new(variant, "retire");
    let intent = managed.intent();
    managed.stage(&intent);

    let installed = with_managed(&managed, |namespace, slots| {
        let source = namespace.retain_managed_staging_source(&intent, slots)?;
        namespace.install_bootstrap_component(&source, &intent, slots)
    })
    .expect("the managed install must publish the staged component");

    let retirement_intent = intent
        .successor_after_component(&installed)
        .expect("the installed evidence closes the install phase");

    let evidence = with_managed(&managed, |namespace, slots| {
        let source = namespace.retain_managed_marker_source(slots)?;
        namespace.retire_bootstrap_marker(&source, &retirement_intent, slots)
    })
    .expect("the managed marker must retire into the action slot");

    assert!(
        !managed
            .installed_path()
            .join(name(&managed_marker_name()))
            .exists(),
        "the marker must leave the installed component"
    );
    let retired = managed
        .fixture
        .action_directory(variant, managed.expected.action_digest())
        .join(name(evidence.marker_retirement_leaf()));
    assert!(
        retired.is_file(),
        "the marker must land in the scheduled retirement slot"
    );
}

/// A staged directory whose interior drifted before the edge is refused rather
/// than published: the §4.4 Class 1 managed source-interior arm, exercised.
fn interior_drift_is_refused_before_the_edge(variant: TargetVariantV1) {
    let managed = ManagedFixture::new(variant, "drift");
    let intent = managed.intent();
    managed.stage(&intent);
    fs::write(managed.staging_path(&intent).join("intruder"), b"x")
        .expect("the staged directory is writable");

    // `expect_err` would need `Debug` on the sealed evidence type; the sealed
    // types deliberately do not carry it, so the refusal is matched instead.
    let refused = with_managed(&managed, |namespace, slots| {
        let source = namespace.retain_managed_staging_source(&intent, slots)?;
        namespace.install_bootstrap_component(&source, &intent, slots)
    })
    .is_err();

    assert!(refused, "a drifted staged interior must be refused");
    assert!(
        !managed.installed_path().exists(),
        "a refused install must publish nothing"
    );
}

#[test]
fn managed_install_publishes_the_staged_component_on_a_workspace_target() {
    install_publishes_the_staged_component(TargetVariantV1::Workspace);
}

#[test]
fn managed_install_publishes_the_staged_component_on_a_git_directory_target() {
    install_publishes_the_staged_component(TargetVariantV1::GitDirectory);
}

#[test]
fn managed_install_restart_reobserves_on_a_workspace_target() {
    restart_reobserves_the_installed_component(TargetVariantV1::Workspace);
}

#[test]
fn managed_install_restart_reobserves_on_a_git_directory_target() {
    restart_reobserves_the_installed_component(TargetVariantV1::GitDirectory);
}

#[test]
fn managed_marker_retires_into_the_action_slot_on_a_workspace_target() {
    marker_retirement_moves_the_marker_into_the_action_slot(TargetVariantV1::Workspace);
}

#[test]
fn managed_marker_retires_into_the_action_slot_on_a_git_directory_target() {
    marker_retirement_moves_the_marker_into_the_action_slot(TargetVariantV1::GitDirectory);
}

#[test]
fn managed_staged_interior_drift_is_refused_on_a_workspace_target() {
    interior_drift_is_refused_before_the_edge(TargetVariantV1::Workspace);
}

#[test]
fn managed_staged_interior_drift_is_refused_on_a_git_directory_target() {
    interior_drift_is_refused_before_the_edge(TargetVariantV1::GitDirectory);
}
