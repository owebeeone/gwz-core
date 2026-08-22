//! R2-D Step 2.1 — the executed `durable_leaf.*` interruption/restart/
//! convergence matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.1 (activate
//! `durable_leaf.*` with rows) and §4 Step 5.1 (:441) for the per-key evidence
//! form; `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this
//! package flips; `GwzM5-8R4bR2ConsumerCheckpoint.md` §8 (:232-237) for the
//! observation contract each row interrupts. The matrix, both target variants,
//! and the reconciliation follow `catalog/bootstrap/tests.rs:326-383` by way of
//! the Phase 1 package (`admission/tests_fault_matrix.rs`).
//!
//! Every row drives the frozen leaf seam against a real payload leaf resident
//! in a real admitted action directory, reached through the sealed catalog
//! owner and the Phase 1 admission owner on a real target — so an interruption
//! is a real process stop across a real durable edge.
//!
//! A leaf *observation* is a read: its two durable edges are the leaf flush and
//! the namespace barrier, and neither may move a name or a byte. So the
//! convergence property this matrix proves is stronger than "the restart
//! settles": the restart must re-derive the **identical** proof — same durable
//! identity, same length, same fingerprint — over an unchanged parent.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::leaf_observation::HostLeafObserverV1;
use super::platform::HostPlatform;
use super::tests_leaf_observation::{
    BarrierNamespaceV1, ExpectedPayloadV1, census, component, open_dir, retain, write_leaf,
};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1,
};
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_leaf_fault,
};
use crate::checked_artifact::leaf::{DurableLeafExpectation, DurableLeafProof, LeafObserver};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CleanupAliasSetV1, ManagedBootstrapInputV1, RequestOwnerBindingV1,
    RootEntryNameV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Which side of the two-sided proof a row interrupts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeafArmV1 {
    /// The exact payload leaf: proof, flush, barrier, exact reobservation.
    Exact,
    /// The durably absent leaf: absence, barrier, absence again.
    Absent,
}

impl LeafArmV1 {
    const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Absent => "absent",
        }
    }
}

/// Every `durable_leaf.*` boundary, on the arm that crosses it, in the order
/// one observation reaches them. The exact arm crosses ten of the eleven; the
/// absence arm crosses `missing_revalidate` and re-crosses the three boundaries
/// the two arms share, so both sides of the two-sided proof are interrupted at
/// every boundary they own.
const DURABLE_LEAF_MATRIX: [(Fault, LeafArmV1); 14] = [
    (Fault::DurableLeafFirstOpen, LeafArmV1::Exact),
    (Fault::DurableLeafFirstIdentity, LeafArmV1::Exact),
    (Fault::DurableLeafFirstContent, LeafArmV1::Exact),
    (Fault::DurableLeafFileFlush, LeafArmV1::Exact),
    (Fault::DurableLeafNamespaceBarrier, LeafArmV1::Exact),
    (Fault::DurableLeafParentRevalidate, LeafArmV1::Exact),
    (Fault::DurableLeafNameRevalidate, LeafArmV1::Exact),
    (Fault::DurableLeafHandleRevalidate, LeafArmV1::Exact),
    (Fault::DurableLeafLengthRevalidate, LeafArmV1::Exact),
    (Fault::DurableLeafContentRevalidate, LeafArmV1::Exact),
    (Fault::DurableLeafFirstOpen, LeafArmV1::Absent),
    (Fault::DurableLeafNamespaceBarrier, LeafArmV1::Absent),
    (Fault::DurableLeafParentRevalidate, LeafArmV1::Absent),
    (Fault::DurableLeafMissingRevalidate, LeafArmV1::Absent),
];

/// Repeated crashes at one boundary, past the nominal capacity of the
/// observation: the exact arm crosses ten boundaries, so twelve rounds cross
/// it comfortably (the Phase 1 reading of "nominal capacity",
/// `admission/tests_fault_matrix.rs:75-80`).
const REPEATED_CRASH_ROUNDS: usize = 12;

/// The two frozen target variants the matrix must execute on
/// (`GwzM5-8R2D-Plan.md` §4 Step 1.3; the `catalog/bootstrap/tests.rs:328`/
/// `:398` pair).
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

struct MatrixFixture {
    root: PathBuf,
}

impl MatrixFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-leaf-matrix-{label}-{}-{}",
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

    /// The published action directory the payload leaves live in.
    fn action_root(
        &self,
        variant: TargetVariantV1,
        expected: &ActionCapacityReservationV1,
    ) -> PathBuf {
        self.catalog_root(variant)
            .join(RootEntryNameV1::ActiveAction(expected.action_digest()).name())
    }
}

impl Drop for MatrixFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One fresh-process admission on the requested target: acquire that target's
/// lease, recover through the sole sealed catalog owner, admit, release.
/// Mirrors `admission/tests_fault_matrix.rs:173-200`.
fn admit(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) -> AdmittedActionV1 {
    let handoff = match variant {
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
    };
    handoff.expect("the admission owner must admit the action")
}

/// A fresh-process recovery on the requested target, with no admission edge:
/// the lease is re-acquired, the sealed catalog owner recovers the complete
/// catalog, and both are released again before the observation runs.
///
/// Admission itself is executed once, by `prepare`. Its §7 (:220-221)
/// exactness predicate requires the published action directory to hold the
/// resident reservation and *no other child*
/// (`provider/interior.rs:497-500`), which a resident payload slot is; teaching
/// `resume_or_admit` to tolerate a mid-life action is the action-lifecycle
/// question R2-E owns, not Step 2.1's.
fn recover(fixture: &MatrixFixture, variant: TargetVariantV1) {
    match variant {
        TargetVariantV1::Workspace => {
            let runtime = try_acquire_workspace_runtime(fixture.path())
                .unwrap()
                .expect("workspace runtime lease");
            recover_or_create(runtime.catalog_mutation_lease())
                .expect("the sealed catalog owner must recover a complete catalog");
        }
        TargetVariantV1::GitDirectory => {
            let request =
                CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
            let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
            let leases = CatalogLeaseSetV1::try_acquire(batch)
                .unwrap()
                .expect("Git catalog lease");
            let lease = leases.leases().next().expect("one Git catalog lease");
            recover_or_create(lease)
                .expect("the sealed catalog owner must recover a complete catalog");
        }
    }
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

fn slot_name(expected: &ActionCapacityReservationV1, slot: BaseActionSlotV1) -> String {
    ActionSlotV1::Base(slot).name(expected.action_digest())
}

/// The payload the exact arm proves. Larger than one streaming chunk, so every
/// restart re-streams a multi-chunk payload rather than a single buffer.
fn payload_bytes() -> Vec<u8> {
    (0..9_000_u32).map(|index| (index % 241) as u8).collect()
}

/// One fresh-process attempt: recover the catalog through the sealed owner on
/// the real target, reopen the published action directory, mint a retained
/// capability from that live handle, and observe the leaf the arm names.
fn attempt(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    arm: LeafArmV1,
) -> Result<DurableLeafProof<DurableObjectIdentityV1>, CheckedFsError> {
    recover(fixture, variant);
    let parent = retain(open_dir(&fixture.action_root(variant, expected)));
    let mut namespace = BarrierNamespaceV1::default();
    match arm {
        LeafArmV1::Exact => {
            let content = ExpectedPayloadV1::new(payload_bytes());
            HostLeafObserverV1.observe_durable(
                &parent,
                &component(&slot_name(expected, BaseActionSlotV1::SourcePayload)),
                DurableLeafExpectation::Exact(&content),
                &mut namespace,
                0,
            )
        }
        LeafArmV1::Absent => HostLeafObserverV1.observe_durable::<ExpectedPayloadV1, _>(
            &parent,
            &component(&slot_name(expected, BaseActionSlotV1::GoalPayload)),
            DurableLeafExpectation::Missing,
            &mut namespace,
            0,
        ),
    }
}

/// Admits the action once and installs the exact source payload, so every
/// later attempt observes a leaf that is already durable.
///
/// The admission handoff's directory identity is checked against the live
/// action directory here, so the retained parent every attempt mints from that
/// same directory is the one the sealed owners published — the retained-parent
/// binding is proven, not assumed.
fn prepare(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) {
    let admitted = admit(fixture, variant, expected);
    let action = open_dir(&fixture.action_root(variant, expected));
    assert_eq!(
        HostPlatform.dir_identity(&action).unwrap().durable(),
        admitted.directory_identity(),
        "the observed action directory is the one the admission handoff proved"
    );
    write_leaf(
        &action,
        &slot_name(expected, BaseActionSlotV1::SourcePayload),
        &payload_bytes(),
    );
}

fn settle(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    arm: LeafArmV1,
    context: &str,
) -> DurableLeafProof<DurableObjectIdentityV1> {
    attempt(fixture, variant, expected, arm)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"))
}

/// The executed key list must reconcile against the vocabulary's own
/// `durable_leaf.*` inventory, so a key added to the family without a matrix
/// row fails here rather than silently escaping
/// (`interface_tests/fault_expected_keys.rs`; the
/// `catalog/bootstrap/tests.rs:358-372` reconciliation).
fn reconcile_executed_keys() {
    let mut actual = DURABLE_LEAF_MATRIX
        .iter()
        .map(|(key, _)| key.stable_key())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    actual.dedup();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("durable_leaf.").then_some(value)
        })
        .collect::<Vec<_>>();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

fn suffix(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .1
}

/// Interrupt at every `durable_leaf.*` boundary, restart, and converge — with
/// the per-key evidence line the L1-16/L2-14 form expects printed for the run
/// tail.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    let expected = reservation(0xC1);

    for (key, arm) in DURABLE_LEAF_MATRIX {
        let stable = key.stable_key();
        let fixture = MatrixFixture::new(&format!(
            "{}-{}-{}",
            variant.label(),
            arm.label(),
            suffix(&stable)
        ));
        prepare(&fixture, variant, &expected);
        let action_root = fixture.action_root(variant, &expected);
        let settled_proof = settle(&fixture, variant, &expected, arm, "baseline");
        let settled_rows = census(&action_root);
        let settled_catalog = census(&fixture.catalog_root(variant));
        match arm {
            LeafArmV1::Exact => assert!(settled_proof.is_exact_durable()),
            LeafArmV1::Absent => {
                assert_eq!(settled_proof, DurableLeafProof::MissingDurable);
            }
        }

        run_next_leaf_fault(key, || panic!("simulated leaf observation process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, variant, &expected, arm);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable} on the {} arm",
            arm.label()
        );

        let resumed = settle(&fixture, variant, &expected, arm, &stable);
        assert_eq!(
            resumed, settled_proof,
            "{stable}: the restart did not re-derive the identical proof"
        );
        assert_eq!(
            census(&action_root),
            settled_rows,
            "{stable}: the interrupted observation mutated the action directory"
        );
        assert_eq!(
            census(&fixture.catalog_root(variant)),
            settled_catalog,
            "{stable}: the interrupted observation mutated the catalog root"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // proves the same fact again and still writes nothing.
        let again = settle(&fixture, variant, &expected, arm, &stable);
        assert_eq!(again, resumed, "{stable}: the resume changed the proof");
        assert_eq!(census(&action_root), settled_rows);

        println!(
            "{stable} | {} | arm={} | interrupted=yes | restart=identical-proof | rows={} | resume=no-mutation",
            variant.label(),
            arm.label(),
            settled_rows.len()
        );
    }
}

/// ConsumerCheckpoint §12 (:346): crashing the same boundary far past nominal
/// capacity must leave the durable tree exactly as it found it. The two
/// boundaries crossed here are the observation's only durable edges — the leaf
/// flush and the namespace barrier — and both are repeatable, because the state
/// a crash at either leaves is the same state the next attempt re-enters.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let expected = reservation(0xD2);

    for key in [
        Fault::DurableLeafFileFlush,
        Fault::DurableLeafNamespaceBarrier,
    ] {
        let stable = key.stable_key();
        let fixture = MatrixFixture::new(&format!("{}-r-{}", variant.label(), suffix(&stable)));
        prepare(&fixture, variant, &expected);
        let action_root = fixture.action_root(variant, &expected);
        let settled_proof = settle(&fixture, variant, &expected, LeafArmV1::Exact, "baseline");
        let settled_rows = census(&action_root);

        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_leaf_fault(key, || panic!("simulated leaf observation process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = attempt(&fixture, variant, &expected, LeafArmV1::Exact);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );
            assert_eq!(
                census(&action_root),
                settled_rows,
                "{stable}: round {round} changed the durable row set"
            );
        }

        let converged = settle(&fixture, variant, &expected, LeafArmV1::Exact, &stable);
        assert_eq!(
            converged, settled_proof,
            "{stable}: the proof did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | rows-stable=yes | rows={} | converged=identical-proof",
            variant.label(),
            settled_rows.len()
        );
    }
}

#[test]
fn durable_leaf_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn durable_leaf_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_boundary_crashes_mutate_nothing_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_boundary_crashes_mutate_nothing_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}
