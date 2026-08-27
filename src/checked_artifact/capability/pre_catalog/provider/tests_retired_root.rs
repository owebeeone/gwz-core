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

use cap_fs_ext::ambient_authority;

use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::catalog::{OpaqueRetainedCatalogV1, recover_or_create};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, CleanupAliasSetV1,
    InfrastructureSlotV1, MAX_RETIRED_ACTION_DIRS, ManagedBootstrapInputV1, RequestOwnerBindingV1,
    RootEntryNameV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The two frozen target variants. Added at the E3 remediation so the nested
/// retired-root chain's refusal is proved on both, per the F1 ruling.
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
    variant: TargetVariantV1,
}

impl Fixture {
    fn new(label: &str) -> Self {
        Self::on(TargetVariantV1::Workspace, label)
    }

    fn on(variant: TargetVariantV1, label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2e-retired-root-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        Self { root, variant }
    }

    fn catalog_root(&self) -> PathBuf {
        let base = match self.variant {
            TargetVariantV1::Workspace => self.root.clone(),
            TargetVariantV1::GitDirectory => git2::Repository::open(&self.root)
                .unwrap()
                .commondir()
                .to_path_buf(),
        };
        base.join(CatalogPrivateNameV1::Final.relative_path(self.variant.private_root()))
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

/// One fresh-process catalog session on the fixture's target: acquire that
/// target's lease, recover through the sole sealed catalog owner, run the body,
/// release. Both arms mirror `namespace/tests_fault_matrix.rs`.
fn with_catalog<T>(
    fixture: &Fixture,
    body: impl FnOnce(OpaqueRetainedCatalogV1<'_>) -> Result<T, CheckedFsError>,
) -> Result<T, CheckedFsError> {
    match fixture.variant {
        TargetVariantV1::Workspace => {
            let runtime = try_acquire_workspace_runtime(&fixture.root)
                .unwrap()
                .expect("workspace runtime lease");
            let catalog = recover_or_create(runtime.catalog_mutation_lease())?;
            body(catalog)
        }
        TargetVariantV1::GitDirectory => {
            let request =
                CatalogLeaseTargetRequestV1::repository_common_git_directory(&fixture.root);
            let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
            let leases = CatalogLeaseSetV1::try_acquire(batch)
                .unwrap()
                .expect("Git catalog lease");
            let lease = leases.leases().next().expect("one Git catalog lease");
            let catalog = recover_or_create(lease)?;
            body(catalog)
        }
    }
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

/// Deep enough that the first shape of this widening aborted the process here.
///
/// The E3 interior review's F1 probe measured the transition: a nested chain of
/// depth 200 returned a typed refusal, and depth 700, 2000 and 8000 all ended
/// `fatal runtime error: stack overflow, aborting … (signal: 6, SIGABRT)` —
/// a threshold that was a property of the host's thread stack, not of any check
/// in the code. 1024 is comfortably past it while staying cheap to plant and to
/// tear down.
const NESTED_CHAIN_DEPTH: usize = 1024;

/// Plants a chain of nested `retired-actions-v1` directories inside `start`.
///
/// One handle at a time, each level opened relative to the last, so the walk
/// needs neither a process-global `chdir` nor a path that would blow past
/// `PATH_MAX` at this depth.
fn plant_nested_retired_chain(start: &Path, depth: usize) {
    let name = InfrastructureSlotV1::RetiredActions.name();
    let mut directory = cap_std::fs::Dir::open_ambient_dir(start, ambient_authority())
        .expect("the retired root is openable");
    for _ in 0..depth {
        directory.create_dir(name).expect("the chain is writable");
        directory = directory
            .open_dir(name)
            .expect("the chain level is openable");
    }
}

/// Removes the chain, iteratively, before the fixture's `Drop` can reach it.
///
/// `std::fs::remove_dir_all` recurses one stack frame per level, so at this
/// depth the teardown would abort the process exactly as the defect under test
/// did — which would say nothing about the fix.
///
/// The walk is O(depth) with two handles and no descent: each turn lifts the
/// head's sub-chain up beside it, removes the now-empty head, and puts the
/// sub-chain back under the head's name, shortening the chain by one. A
/// bottom-up descent would instead be O(depth²) unless it held one descriptor
/// per level, which at these depths exhausts the descriptor table.
fn remove_nested_retired_chain(start: &Path) {
    const SCRATCH: &str = "gwz-chain-teardown";
    let name = InfrastructureSlotV1::RetiredActions.name();
    let Ok(root) = cap_std::fs::Dir::open_ambient_dir(start, ambient_authority()) else {
        return;
    };
    while let Ok(head) = root.open_dir(name) {
        if head.open_dir(name).is_err() {
            drop(head);
            let _ = root.remove_dir(name);
            return;
        }
        if head.rename(name, &root, SCRATCH).is_err() {
            return;
        }
        drop(head);
        if root.remove_dir(name).is_err() || root.rename(SCRATCH, &root, name).is_err() {
            return;
        }
    }
}

/// **The F1 cure, driven.** `observe_slot`'s first shape re-entered
/// `interior::observe` on a populated retired root; `exact_row` is
/// parent-independent, so a `retired-actions-v1` child of the retired root
/// classified as a perfectly good infrastructure row and the pair became
/// mutually recursive with no depth counter. Every budget in the loop was
/// allocated per level, so nothing caught it — the process aborted instead of
/// refusing, on the path of *every* catalog consumer, since `completed_record`
/// runs in every recovery and every publication acquisition window.
///
/// `read_retired_root` closes it structurally rather than by a counter: it is a
/// single directory read that classifies each child one level deep and calls
/// neither `observe` nor `observe_slot`, so there is no self-call to exceed. A
/// nested chain of any depth is now one read and a typed refusal — the chain's
/// first level classifies `Infrastructure`, which is not an `ActiveAction` row,
/// so `unaccepted_rows` is nonzero and the widened predicate refuses.
fn a_nested_retired_root_chain_is_a_typed_refusal(variant: TargetVariantV1) {
    let fixture = Fixture::on(variant, &format!("nested-{}", variant.label()));
    with_catalog(&fixture, |_| Ok(())).expect("the sealed catalog owner creates a catalog");
    plant_nested_retired_chain(&fixture.retired_root(), NESTED_CHAIN_DEPTH);

    let refusal = with_catalog(&fixture, |catalog| recovered(&catalog));
    remove_nested_retired_chain(&fixture.retired_root());
    assert!(
        refusal.is_err(),
        "a nested retired-root chain must be a typed refusal, never a process abort"
    );
    println!(
        "nested-retired-chain | {} | depth={NESTED_CHAIN_DEPTH} | refused=typed | aborted=no",
        variant.label()
    );
}

#[test]
fn a_nested_retired_root_chain_is_a_typed_refusal_on_a_workspace_target() {
    a_nested_retired_root_chain_is_a_typed_refusal(TargetVariantV1::Workspace);
}

#[test]
fn a_nested_retired_root_chain_is_a_typed_refusal_on_a_git_directory_target() {
    a_nested_retired_root_chain_is_a_typed_refusal(TargetVariantV1::GitDirectory);
}

/// The retired root's own entry bound is `MAX_RETIRED_ACTION_DIRS`, checked
/// explicitly inside the dedicated reader and **not** inherited from
/// `interior::observe`'s caps. Sixty-five well-formed action rows is one past
/// it, and is refused before any of them is classified into a fact.
#[test]
fn a_retired_root_past_the_frozen_retired_bound_refuses() {
    let fixture = Fixture::new("over-bound");
    with_catalog(&fixture, |_| Ok(())).expect("the sealed catalog owner creates a catalog");
    for index in 0..=u8::try_from(MAX_RETIRED_ACTION_DIRS).expect("the frozen bound is small") {
        let name = RootEntryNameV1::ActiveAction(ActionDigestV1::new([index; 32])).name();
        fs::create_dir(fixture.retired_root().join(name)).expect("the retired root is writable");
    }

    let refusal = with_catalog(&fixture, |catalog| recovered(&catalog));
    assert!(
        refusal.is_err(),
        "a retired root past MAX_RETIRED_ACTION_DIRS must refuse"
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
