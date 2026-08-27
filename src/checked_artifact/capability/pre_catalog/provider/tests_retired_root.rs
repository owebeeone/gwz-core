//! R2-E E3.1 precondition — the T1 widening's own rows.
//!
//! Controlling text: `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2 (THE T1
//! CURE, authorized by both axes), whose §2.4 item 4 states this file's duty:
//! "a retired-root-populated catalog recovers, revalidates and publishes; a
//! retired root holding any non-`ActiveAction` child still refuses".
//!
//! **Census statement.** 165 total, unchanged; no key minted, none retired.
//! The widening mints no record, no slot, no name and no fault key — only a
//! provider-private observation fact, which is the shape freeze `:1443-1450`
//! sanctions for a Class 2 extension.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py`) and out of the
//! injection-site rescan (`interface_tests/fault_expected_keys.rs`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::try_acquire_workspace_runtime;
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::catalog::{OpaqueRetainedCatalogV1, recover_or_create};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, CleanupAliasSetV1,
    InfrastructureSlotV1, ManagedBootstrapInputV1, RequestOwnerBindingV1, RootEntryNameV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2e-retired-root-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        Self { root }
    }

    fn catalog_root(&self) -> PathBuf {
        self.root
            .join(CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::Workspace))
    }

    fn retired_root(&self) -> PathBuf {
        self.catalog_root()
            .join(InfrastructureSlotV1::RetiredActions.name())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One fresh-process catalog session: acquire the workspace lease, recover
/// through the sole sealed catalog owner, run the body, release.
fn with_catalog<T>(
    fixture: &Fixture,
    body: impl FnOnce(OpaqueRetainedCatalogV1<'_>) -> Result<T, CheckedFsError>,
) -> Result<T, CheckedFsError> {
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let catalog = recover_or_create(runtime.catalog_mutation_lease())?;
    body(catalog)
}

/// Reaching this at all is the proof: `recover_or_create` runs
/// `retain_completed_catalog`, which drives gate 1 (`completed_record`), gate 2
/// (`retain_directory`) and — through its closing `revalidate` — gate 3
/// (`require_named_directory_identity`). Before the widening each of the three
/// refused a retired-root-populated catalog on its own.
const fn recovered(_catalog: &OpaqueRetainedCatalogV1<'_>) -> Result<(), CheckedFsError> {
    Ok(())
}

fn reservation(action: u8) -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([action; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            2,
            vec![ManagedBootstrapInputV1::new([3; 32], 2).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

/// Creates the catalog, then plants `child` under its retired root by hand.
///
/// Planting rather than retiring is deliberate for the precondition step: the
/// terminal retirement edge itself is E3.1 proper, and this commit's claim is
/// only about the *reading* of a retired root that already holds a row.
fn planted(label: &str, child: &str) -> Fixture {
    let fixture = Fixture::new(label);
    with_catalog(&fixture, |_| Ok(())).expect("the sealed catalog owner creates a catalog");
    fs::create_dir(fixture.retired_root().join(child)).expect("the retired root is writable");
    fixture
}

fn retired_action_name() -> String {
    RootEntryNameV1::ActiveAction(ActionDigestV1::new([0xE3; 32])).name()
}

fn children(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .expect("directory is readable")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// The T1 defect, stated as its cure: before the widening a **single** retired
/// child made `completed_record` `None`, so `retain_completed_catalog` refused,
/// so recovery refused — the catalog was unobservable and therefore
/// unrecoverable at its own first terminal retirement.
#[test]
fn a_retired_root_holding_an_action_row_still_recovers_and_revalidates() {
    let fixture = planted("recovers", &retired_action_name());

    with_catalog(&fixture, |catalog| recovered(&catalog))
        .expect("a retired-root-populated catalog recovers and revalidates");

    // Settled, not merely reached: a second fresh process recovers the same
    // catalog and the retired root is untouched.
    with_catalog(&fixture, |catalog| recovered(&catalog))
        .expect("the second recovery of a retired-root-populated catalog settles");
    assert_eq!(
        children(&fixture.retired_root()),
        vec![retired_action_name()]
    );
}

/// The publication half of §2.4 item 4: an admission publishes into a catalog
/// whose retired root is populated. This exercises the `AdmissionCatalogInterior`
/// destination recheck, which re-proves `completed_record` **inside** the
/// acquisition window (`publication.rs`), so it is a distinct gate from the
/// recovery row above.
#[test]
fn an_admission_publishes_into_a_retired_root_populated_catalog() {
    let fixture = planted("publishes", &retired_action_name());
    let expected = reservation(0x7A);

    let admitted = with_catalog(&fixture, |catalog| {
        ActionAdmissionOwnerV1::from_retained_catalog(catalog).resume_or_admit(&expected)
    })
    .expect("the frozen admission seam admits into a retired-root-populated catalog");

    let published = fixture
        .catalog_root()
        .join(RootEntryNameV1::ActiveAction(expected.action_digest()).name());
    assert!(published.is_dir(), "the action row must be published");
    assert_eq!(
        admitted.reservation().action_digest(),
        expected.action_digest()
    );

    // And the catalog carrying both a retired row and a live action row still
    // recovers, which is the state every later terminal retirement starts from.
    with_catalog(&fixture, |catalog| recovered(&catalog))
        .expect("a catalog with both a retired row and an active row recovers");
}

/// The bounded reading accepts **only** `RootEntryNameV1::ActiveAction`
/// children (E0.2b §2.4 item 2). An infrastructure-slot name planted in the
/// retired root classifies into the observation's `rows` rather than being
/// refused by the classifier, so the widened predicate refuses on
/// `infrastructure_rows != 0` — which is why that emptiness is checked
/// explicitly instead of being assumed.
#[test]
fn a_retired_root_holding_an_infrastructure_slot_name_still_refuses() {
    let fixture = planted(
        "infrastructure",
        InfrastructureSlotV1::RetiredActions.name(),
    );

    let refusal = with_catalog(&fixture, |catalog| recovered(&catalog));
    assert!(
        refusal.is_err(),
        "a retired root holding an infrastructure-slot row must refuse"
    );
}

/// The classifier's own unowned-child refusal is unchanged by the widening: a
/// foreign child of the retired root is refused by `exact_row` before it can be
/// classified at all.
#[test]
fn a_retired_root_holding_a_foreign_child_still_refuses() {
    let fixture = planted("foreign", "not-an-action-row");

    let refusal = with_catalog(&fixture, |catalog| recovered(&catalog));
    assert!(
        refusal.is_err(),
        "a retired root holding a foreign child must refuse"
    );
}

/// A malformed-recognized action name — the right prefix, the wrong digest —
/// is refused too, so the widening admits the *grammar*, never the prefix.
#[test]
fn a_retired_root_holding_a_malformed_action_name_still_refuses() {
    let fixture = planted("malformed", "action-notahexdigest-v1");

    let refusal = with_catalog(&fixture, |catalog| recovered(&catalog));
    assert!(
        refusal.is_err(),
        "a retired root holding a malformed action name must refuse"
    );
}

/// A bootstrap staging interior keeps its refusal, preserved by name rather
/// than by fall-through (`interior::staging_plan`). A catalog being built must
/// not adopt a staging directory that already holds retired action rows.
#[test]
fn the_bootstrap_staging_plan_still_refuses_a_populated_retired_root() {
    let source = include_str!("interior.rs");
    assert!(
        source.contains("Some(RawCatalogInteriorFactV1::RetiredActionRoot { .. }) => {"),
        "interior::staging_plan must refuse a populated retired root by name, so a future \
         reshaping of the fact type cannot silently widen the bootstrap's adoption grammar"
    );
}
