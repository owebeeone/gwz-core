//! R2-E Step E3.2 — the executed `terminal.*` interruption/restart/convergence
//! matrix.
//!
//! Controlling text: `GwzM5-8R2E-Plan.md` §3 Phase E3;
//! `GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §4 (the terminal activation record:
//! DECISION T-A, the eleven-key table, and the `terminal.authority_release`
//! determination) as amended by `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md`
//! §2 (the T1 widening), §3 (DECISION T-B′), §8 (DECISION T-C′ and T-D's
//! re-grounding) and §6.3 (this step's duty list, verbatim);
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this package
//! flips and §4.3 row E7 for the edge driven here;
//! `GwzM5-8R4bR2ConsumerCheckpoint.md` §12 for the repeated-crash rule.
//!
//! **Ten rows, not eleven.** `terminal.authority_release` never gains a fault
//! boundary — DECISION T-D, re-grounded at E0.2b §8 on reading (a) alone:
//! `RetainedWriteAuthorityV1` is deliberately neither `Copy` nor `Clone`
//! (`coordinator/execution.rs`), so "release" is a move-out, an in-process
//! event no restart can observe, which is exactly the Phase-3 settle's own
//! ground ("not durable edges: no create, write, flush, publish, retire, or
//! reobserve occurs at either"). The family therefore lands as
//! `PartiallyExecuted`, whose per-key siteless proof in
//! `interface_tests/fault_expected_keys.rs` keeps the `Reserved` arm's
//! guarantee for the eleventh key.
//!
//! **Census statement.** 165 total, unchanged; no key minted, none retired;
//! `terminal.*` moves 0/11 → 10/11 with this commit.
//!
//! **Git-directory route statement** (`GwzM5-8R2DSettledTuple.md` §11.3 item 2,
//! consumed at E0.2b §7.7). The duty is to state which route this matrix's
//! Git-directory arm takes, because a Git-directory catalog has no `.gwz`
//! ancestor and a managed parent's workspace root is fixture-placed today.
//! **This family takes neither route: it touches no managed parent at all.**
//! Every name the terminal retirement uses is a catalog-root or action-slot
//! name derived from the admitted action's own digest, and the retirement's two
//! parents are the catalog root and its own retired root — both inside the
//! catalog on either target. So the Step-2.3 `cfg(test)` door
//! (`retain_managed_parent_at_for_test`) is not on this matrix's path, and the
//! §11.3-item-2 workspace-root binding question, routed to E4.2, is not an
//! input to it either.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py`) and out of the
//! injection-site rescan (`interface_tests/fault_expected_keys.rs`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, ambient_authority};
use sha2::{Digest, Sha256};

use crate::checked_artifact::capability::{CheckedFsError, DurableIdentityProvider};
use crate::checked_artifact::catalog::{OpaqueRetainedCatalogV1, recover_or_create};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_terminal_fault, take_armed_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CleanupAliasSetV1, CleanupAliasV1, CleanupRowV1, CleanupWorklistV1,
    DurableLeafFingerprintV1, InfrastructureSlotV1, ManagedBootstrapInputV1, RequestOwnerBindingV1,
    RootEntryNameV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Every `terminal.*` boundary that has a site, in the order one virgin drive
/// crosses them.
///
/// Ten, not eleven (§4.3's determination). The split across the two source
/// files is DECISION T-C′'s, by capability rather than by family: the first
/// five are reads and a flush of the **action directory**, announced from
/// `namespace_mutation.rs`, which owns it; the last five are edges of the
/// **catalog root** and its retired root, announced from
/// `admission_mutation.rs`, which owns them.
const TERMINAL_MATRIX: [Fault; 10] = [
    Fault::TerminalAuthorityReobserve,
    Fault::TerminalPayloadReobserve,
    Fault::TerminalCleanupReobserve,
    Fault::TerminalReservationReobserve,
    Fault::TerminalDirectoryFlush,
    Fault::TerminalRetiredSlotReserve,
    Fault::TerminalActionDirectoryRetire,
    Fault::TerminalRetiredDirectoryReobserve,
    Fault::TerminalCatalogBarrier,
    Fault::TerminalTerminalRevalidate,
];

/// The declared repeatable subset, one boundary per capability half plus the
/// one durable edge that leaves no namespace delta.
///
/// A boundary is repeatable only when the durable state its crash leaves
/// resolves back to the same edge. Boundaries #1-#4 are pure reads of the
/// action directory and #6 is a pure read of the retired root, so all five
/// leave no durable delta at all; #5 is a directory flush, whose crash leaves
/// the same rows resident and therefore re-enters the same sequence. The three
/// crossed here are chosen because each is additionally a row a retry could be
/// tempted to re-derive: the authority row's two homes, the flush's parent, and
/// the derived retirement destination.
const REPEATED_BOUNDARIES: [Fault; 3] = [
    Fault::TerminalAuthorityReobserve,
    Fault::TerminalDirectoryFlush,
    Fault::TerminalRetiredSlotReserve,
];

/// The complement, machine-checked rather than asserted in prose.
///
/// All four sit **after** the rename, which is the commit point: once the
/// action row is resident under the retired root and gone from the catalog
/// root, the next drive converges by observation alone and re-crosses none of
/// them. A boundary wrongly classed single-crossing fires and fails loudly.
const SINGLE_CROSSING_BOUNDARIES: [Fault; 4] = [
    Fault::TerminalActionDirectoryRetire,
    Fault::TerminalRetiredDirectoryReobserve,
    Fault::TerminalCatalogBarrier,
    Fault::TerminalTerminalRevalidate,
];

/// Repeated crashes at one boundary, past the nominal capacity of the sequence:
/// the virgin terminal drive settles in two durable edges, so twelve crashes
/// cross the nominal capacity several times over without cardinality growth.
const REPEATED_CRASH_ROUNDS: usize = 12;

/// The two frozen target variants the matrix must execute on
/// (`GwzM5-8R2D-Plan.md` §4 Step 1.3).
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
    fn new(variant: TargetVariantV1, label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2e-terminal-{label}-{}-{}",
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

    /// The whole durable state this matrix converges: the catalog root's own
    /// rows, the retired root's rows, and the retiring action directory's rows
    /// wherever it currently lives.
    fn census(&self, action: ActionDigestV1) -> (Vec<String>, Vec<String>, Vec<String>) {
        let leaf = RootEntryNameV1::ActiveAction(action).name();
        let active = self.catalog_root().join(&leaf);
        let retired = self.retired_root().join(&leaf);
        let directory = if retired.is_dir() { retired } else { active };
        (
            children(&self.catalog_root()),
            children(&self.retired_root()),
            children(&directory),
        )
    }

    /// The one real admission per fixture, followed by the fixture-placed rows
    /// the terminal retirement's preconditions read.
    ///
    /// **The handoff this returns is the real one.** E0.2b §8's corrected
    /// [P2-4] duty binds this matrix to drive through a real admitted action
    /// (`ActionAdmissionOwnerV1::resume_or_admit`), and the E3 interior review
    /// (F6) found the first shape keeping only the directory identity and
    /// rebuilding a synthetic handoff for every attempt, first included.
    /// `AdmittedActionV1` derives `Clone` and carries no lifetime, so the token
    /// the frozen seam issued is carried out of the session and used for every
    /// attempt — the test-only issuer (`admit_observed_action`) is gone from
    /// this file entirely, and with it the duty's inherited deviation. The
    /// sequence is the production one: admit while the directory is exact, run
    /// the action, retire.
    fn admit(&self, expected: &ActionCapacityReservationV1) -> AdmittedActionV1 {
        let admitted = with_catalog(self, |catalog| {
            ActionAdmissionOwnerV1::from_retained_catalog(catalog).resume_or_admit(expected)
        })
        .expect("the frozen admission seam must admit the action");
        place_completed_action_rows(
            &self
                .catalog_root()
                .join(RootEntryNameV1::ActiveAction(expected.action_digest()).name()),
            expected,
        );
        admitted
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn children(directory: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// One fresh-process catalog session on the requested target: acquire that
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

fn slot_name(slot: BaseActionSlotV1, action: ActionDigestV1) -> String {
    ActionSlotV1::Base(slot).name(action)
}

/// The durable rows a completed action leaves behind, placed by the fixture.
///
/// None of them is a `terminal.*` edge: the authority record's install and
/// retirement are `record.*`'s (Step 2.4), the payload leaves are
/// `durable_leaf.*`'s (Step 2.1), and the cleanup worklist and its three alias
/// retirements are `cleanup.*`'s (R2-E Phase E1, landing in its own package).
/// So the fixture places them and this matrix interrupts only the ten
/// boundaries the terminal retirement itself crosses — the same division
/// `namespace/tests_fault_matrix.rs` draws when it writes its scratch row.
///
/// The shape is the one §4.3 key #3 names as its precondition: every scheduled
/// cleanup row `Complete` under the **frozen** classifier.
///
/// **The fingerprints are real, and they must be.** The first shape of this
/// fixture built every worklist row from a fabricated
/// `DurableObjectIdentityV1::linux_ext4` fact and a `[5; 32]` digest on a macOS
/// host — facts no object on the machine had — so the frozen
/// `classify_cleanup_row` would have returned `Ambiguous` for every row and the
/// residency-only reading was what made the matrix green. The E3 interior
/// review named that (F3). Each alias is written, then **observed** for its
/// durable identity, its length and its content digest, and the worklist row is
/// built from what the observation returned; the classifier's `Complete` arm is
/// then reachable because the facts are the object's own.
fn place_completed_action_rows(action_directory: &Path, expected: &ActionCapacityReservationV1) {
    const ALIAS_BYTES: &[u8] = b"gwz-retired-alias-fixture\n";
    let action = expected.action_digest();
    let directory = cap_std::fs::Dir::open_ambient_dir(action_directory, ambient_authority())
        .expect("the admitted action directory is openable");
    let mut rows = Vec::new();
    for alias in CleanupAliasV1::ALL {
        let slot = match alias {
            CleanupAliasV1::Source => BaseActionSlotV1::RetiredSourceAlias,
            CleanupAliasV1::Goal => BaseActionSlotV1::RetiredGoalAlias,
            CleanupAliasV1::Authority => BaseActionSlotV1::RetiredAuthorityAlias,
        };
        let name = slot_name(slot, action);
        fs::write(action_directory.join(&name), ALIAS_BYTES)
            .expect("the admitted action directory is writable");
        rows.push(CleanupRowV1::new(
            alias,
            observed_alias_fingerprint(&directory, &name, ALIAS_BYTES),
        ));
    }
    let worklist = CleanupWorklistV1::try_new(expected, rows)
        .expect("the fixture worklist matches the reserved cleanup aliases");
    fs::write(
        action_directory.join(slot_name(BaseActionSlotV1::CleanupWorklist, action)),
        worklist
            .encode_canonical()
            .expect("the worklist is canonically encodable"),
    )
    .expect("the admitted action directory is writable");
}

/// The resident alias's own durable identity, length and content digest, taken
/// through the same `HostPlatform` identity provider the production observation
/// path uses. Nothing here is invented.
fn observed_alias_fingerprint(
    directory: &cap_std::fs::Dir,
    name: &str,
    bytes: &[u8],
) -> DurableLeafFingerprintV1 {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(name, &options)
        .expect("the alias is openable no-follow");
    let identity = super::HostPlatform
        .file_identity(&file)
        .expect("the alias has a durable identity on the closed support table");
    DurableLeafFingerprintV1::new(
        identity.durable().clone(),
        bytes.len() as u64,
        Sha256::digest(bytes).into(),
    )
}

/// One fresh-process terminal attempt: recover the catalog and drive the
/// admitted action's terminal retirement to whichever point the resident
/// durable state has not yet reached.
///
/// `admitted` is the token the frozen admission seam issued, carried across
/// every attempt rather than rebuilt: the restart re-recovers the catalog and
/// re-retains the action namespace through the permit, which re-proves the
/// directory's identity against this handoff, so what a fresh session
/// reconstructs is the *capability*, never the admission decision.
fn attempt(fixture: &Fixture, admitted: &AdmittedActionV1) -> Result<(), CheckedFsError> {
    with_catalog(fixture, |catalog| catalog.retire_admitted_action(admitted))
}

/// The executed key list must reconcile against the vocabulary's own
/// `terminal.*` inventory, so a key added to the family without a matrix row
/// fails here rather than silently escaping.
///
/// The one declared exclusion is `terminal.authority_release`, whose
/// determination is §4.3's; naming it here rather than filtering the family
/// blindly is what keeps the exclusion a two-place deliberate edit.
fn reconcile_executed_keys() {
    let mut actual = TERMINAL_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            (value.starts_with("terminal.") && value != "terminal.authority_release")
                .then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert_eq!(
        TERMINAL_MATRIX.len(),
        10,
        "the terminal matrix is ten rows, not eleven: terminal.authority_release names an \
         in-process move-out that no restart can observe (DECISION T-D)"
    );
}

fn suffix(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .1
}

type Census = (Vec<String>, Vec<String>, Vec<String>);

fn settle(
    fixture: &Fixture,
    expected: &ActionCapacityReservationV1,
    admitted: &AdmittedActionV1,
    context: &str,
) -> Census {
    attempt(fixture, admitted)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
    fixture.census(expected.action_digest())
}

/// Interrupt at every `terminal.*` boundary, restart, and converge.
fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    let expected = reservation(0xE7);
    let settled = {
        let fixture = Fixture::new(variant, "settled");
        let admitted = fixture.admit(&expected);
        let settled = settle(&fixture, &expected, &admitted, "baseline");
        // The settled state is the retirement itself: the action row has left
        // the catalog root and is resident under the retired root with its rows
        // intact.
        let leaf = RootEntryNameV1::ActiveAction(expected.action_digest()).name();
        assert!(!settled.0.contains(&leaf), "the active row must be gone");
        assert_eq!(settled.1, vec![leaf], "the retired row must be resident");
        settled
    };

    for key in TERMINAL_MATRIX {
        let stable = key.stable_key();
        let fixture = Fixture::new(variant, suffix(&stable));
        let admitted = fixture.admit(&expected);

        run_next_terminal_fault(key, || panic!("simulated terminal process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, &admitted);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        let resumed = settle(&fixture, &expected, &admitted, &stable);
        assert_eq!(
            resumed, settled,
            "{stable}: the restart did not converge to the settled catalog"
        );

        // Convergence is settled, not merely reached: the next fresh process
        // re-reads the same durable state and mutates nothing.
        let again = settle(&fixture, &expected, &admitted, &stable);
        assert_eq!(
            again, settled,
            "{stable}: the resume mutated the settled catalog"
        );

        println!(
            "{stable} | {} | interrupted=yes | restart=settled | catalog={} retired={} action={} | resume=no-mutation",
            variant.label(),
            resumed.0.len(),
            resumed.1.len(),
            resumed.2.len()
        );
    }
}

/// ConsumerCheckpoint §12 and the RemPlan-4 R2 stop clause: crashing the same
/// boundary far past nominal capacity must never allocate a fresh retry name
/// and must never grow the durable slot set.
fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    let expected = reservation(0xE8);
    let settled = {
        let fixture = Fixture::new(variant, "repeat-settled");
        let admitted = fixture.admit(&expected);
        settle(&fixture, &expected, &admitted, "baseline")
    };

    for key in REPEATED_BOUNDARIES {
        let stable = key.stable_key();
        let fixture = Fixture::new(variant, &format!("r-{}", suffix(&stable)));
        let admitted = fixture.admit(&expected);
        let mut census: Option<Census> = None;
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_terminal_fault(key, || panic!("simulated terminal process stop"));
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = attempt(&fixture, &admitted);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} never reached the boundary"
            );

            let observed = fixture.census(expected.action_digest());
            match &census {
                None => census = Some(observed),
                Some(first) => assert_eq!(
                    &observed, first,
                    "{stable}: round {round} changed the durable slot set"
                ),
            }
        }

        let rows = census.expect("the boundary is crashed at least once");
        let converged = settle(&fixture, &expected, &admitted, &stable);
        assert_eq!(
            converged, settled,
            "{stable}: the catalog did not converge after {REPEATED_CRASH_ROUNDS} crashes"
        );

        println!(
            "{stable} | {} | rounds={REPEATED_CRASH_ROUNDS} | slots-stable=yes | catalog={} retired={} action={} | converged=yes",
            variant.label(),
            rows.0.len(),
            rows.1.len(),
            rows.2.len()
        );
    }
}

/// The single-crossing claim, machine-checked in the Step-3.2 probe's shape
/// (`bootstrap/managed/tests_provider.rs`): crash the boundary once, re-arm it,
/// and require the *next* drive to settle **without** firing it.
fn run_single_crossing_probe(variant: TargetVariantV1) {
    let expected = reservation(0xE9);

    for key in SINGLE_CROSSING_BOUNDARIES {
        let stable = key.stable_key();
        let fixture = Fixture::new(variant, &format!("s-{}", suffix(&stable)));
        let admitted = fixture.admit(&expected);

        run_next_terminal_fault(key, || panic!("simulated terminal process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, &admitted);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable}"
        );

        run_next_terminal_fault(key, || {
            panic!("a single-crossing boundary was re-crossed by the drive after its crash")
        });
        attempt(&fixture, &admitted)
            .unwrap_or_else(|error| panic!("{stable}: the restart must settle: {error:?}"));
        assert!(
            take_armed_fault(),
            "{stable}: the probe's arm vanished without firing"
        );

        println!(
            "{stable} | {} | single-crossing=yes | restart=settled | re-crossed=no",
            variant.label()
        );
    }
}

#[test]
fn terminal_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn terminal_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_terminal_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_terminal_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}

#[test]
fn single_crossing_terminal_boundaries_are_not_recrossed_on_a_workspace_target() {
    run_single_crossing_probe(TargetVariantV1::Workspace);
}

#[test]
fn single_crossing_terminal_boundaries_are_not_recrossed_on_a_git_directory_target() {
    run_single_crossing_probe(TargetVariantV1::GitDirectory);
}

/// DECISION T-A, driven rather than asserted: the retired child's name is the
/// already-derived `RootEntryNameV1::ActiveAction(digest).name()` under a
/// different parent. Nothing is minted, and the two names are byte-identical
/// because `RootEntryNameV1::name` is parent-independent.
#[test]
fn the_retired_child_keeps_the_derived_active_action_name() {
    let expected = reservation(0xEA);
    let fixture = Fixture::new(TargetVariantV1::Workspace, "decision-t-a");
    let admitted = fixture.admit(&expected);
    settle(&fixture, &expected, &admitted, "decision-t-a");

    let leaf = RootEntryNameV1::ActiveAction(expected.action_digest()).name();
    assert_eq!(children(&fixture.retired_root()), vec![leaf]);
}

/// DECISION T-B′ arm (a): a retired root holding a non-`ActiveAction` child
/// refuses the retirement inside the acquisition window, even though the row it
/// is publishing is free. The planted child is an infrastructure-slot name,
/// which the classifier admits into the observation's `rows` rather than
/// refusing — so this is the clause that has to be checked explicitly.
#[test]
fn a_retired_root_holding_an_infrastructure_row_refuses_the_retirement() {
    let expected = reservation(0xEB);
    let fixture = Fixture::new(TargetVariantV1::Workspace, "t-b-prime-rows");
    let admitted = fixture.admit(&expected);
    fs::create_dir(
        fixture
            .retired_root()
            .join(InfrastructureSlotV1::CatalogFormat.name()),
    )
    .expect("the retired root is writable");

    let refusal = attempt(&fixture, &admitted);
    assert!(
        refusal.is_err(),
        "a retired root carrying an infrastructure row must refuse the retirement"
    );
    assert!(
        fixture
            .catalog_root()
            .join(RootEntryNameV1::ActiveAction(expected.action_digest()).name())
            .is_dir(),
        "the refusal must happen before the rename"
    );
}
