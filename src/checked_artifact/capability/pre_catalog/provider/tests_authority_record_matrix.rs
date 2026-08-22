//! R2-D Step 2.4 — the executed `record.*` interruption/restart/convergence
//! matrix.
//!
//! Controlling text: `dev-docs/GwzM5-8R2D-Plan.md` §4 Step 2.4 (activate
//! `record.*` with rows) and §4 Step 5.1 (:441) for the per-key evidence form;
//! `GwzM5-8R2DInterfaceFreeze.md` §3.5 for the activation map this package
//! flips. The matrix, both target variants, and the reconciliation follow
//! `catalog/bootstrap/tests.rs:326-383` by way of the Phase 1 package
//! (`admission/tests_fault_matrix.rs`) and Step 2.1
//! (`tests_leaf_fault_matrix.rs`).
//!
//! Every row drives one complete authority cycle — stream both payloads,
//! issue the observation, install the record, read it back bounded, join the
//! two paths, retire the record — against real payload leaves resident in a
//! real admitted action directory on a real target. So an interruption is a
//! real process stop across a real durable edge, and the restart is a fresh
//! resolution from whatever the crash left on disk.
//!
//! The convergence predicate is the settled action-directory census: the
//! authority record retired onto its scheduled alias, its active slot free and
//! its write-ahead scratch consumed, with both payload leaves untouched. Every
//! window of the cycle must reach that same census, and a second settle must
//! then change nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::authority_record_binding::{
    AuthorityTransactionV1, ObservedLeafWriterClassV1, install_authority_record,
    observe_streamed_payloads, read_resident_authority_record, retire_authority_record,
    validate_terminal_relation,
};
use super::platform::HostPlatform;
use super::tests_authority_record::retain_action;
use super::tests_leaf_observation::{
    BarrierNamespaceV1, ExpectedPayloadV1, census, open_dir, write_leaf,
};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{CheckedFsError, DurableIdentityProvider};
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_record_fault,
};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, ActionSlotV1, AdmittedActionV1,
    BaseActionSlotV1, CheckedAuthorityRecordV1, CleanupAliasSetV1, ManagedBootstrapInputV1,
    RequestOwnerBindingV1, RootEntryNameV1, retained_authority_observation_owner,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Every `record.*` boundary, in the order one uninterrupted cycle crosses
/// them. The install half runs first because the parse half has nothing to
/// read until a record is resident — which is itself the reason Step 2.4 owns
/// both halves rather than only the parse.
const RECORD_MATRIX: [Fault; 13] = [
    Fault::RecordScratchCreate,
    Fault::RecordScratchWrite,
    Fault::RecordScratchFlush,
    Fault::RecordActivePublish,
    Fault::RecordActiveReobserve,
    Fault::RecordBoundedRead,
    Fault::RecordDecode,
    Fault::RecordCanonicalReencode,
    Fault::RecordBindingValidate,
    Fault::RecordTerminalRelationValidate,
    Fault::RecordRetirementReserve,
    Fault::RecordRetireExact,
    Fault::RecordRetiredReobserve,
];

/// Repeated crashes at one boundary, past the nominal capacity of the cycle.
/// One uninterrupted cycle crosses thirteen boundaries, so the round count must
/// exceed thirteen to be "past nominal capacity" in the Phase 1 sense
/// (`admission/tests_fault_matrix.rs`); twelve — carried over from Step 2.1,
/// whose cycle crosses ten — did not.
const REPEATED_CRASH_ROUNDS: usize = 14;

/// The boundaries a restart can genuinely re-cross, and — below — every
/// boundary that cannot, each with its own reason.
///
/// **Criterion.** A boundary is re-crossable when the crash leaves the cycle in
/// a window the next resolution re-enters at the same point. The one-shot
/// harness arms exactly one key per attempt (`fault_v1.rs`, `run_next_at` /
/// `hit`), so "re-crossed" means the next fresh resolution reaches that same
/// boundary again.
///
/// **Included (9).** Three install-write boundaries — `scratch_create`,
/// `scratch_write`, `scratch_flush` — because a crash anywhere in the
/// write-ahead scratch leaves the active slot absent, so the next resolution
/// re-enters `write_authority_scratch`, which is `create(true)` plus
/// `set_len(0)` over a compile-time slot name and rewrites rather than choosing
/// a fresh one. Five read-only boundaries reached with the active record
/// resident — `bounded_read`, `decode`, `canonical_reencode`,
/// `binding_validate` (all four inside the same
/// `read_and_bind_authority_record` call) and `terminal_relation_validate` —
/// because the crash mutates nothing. And `retirement_reserve`, which is
/// reached before the retirement edge and likewise mutates nothing.
///
/// **Excluded (4), each because the boundary is unreachable a second time —
/// not because it is untested.** Every one of them has an
/// interruption/restart/convergence row in `RECORD_MATRIX` on both variants;
/// what they cannot have is a *repeated* crash at the same point.
///
/// * `active_publish` — consumes the scratch name onto the active name. The
///   next `attempt` finds the active record resident and skips the install.
/// * `active_reobserve` — sits after that publish inside the install, so once
///   the publish has landed the install is skipped and this boundary is never
///   reached again.
/// * `retire_exact` — consumes the active name onto the retired alias. The next
///   `attempt` sees the settled state and returns at the early exit.
/// * `retired_reobserve` — sits after that rename, so it is behind the same
///   early exit.
///
/// Nine included and four excluded is the whole of `RECORD_MATRIX`; the two
/// lists are reconciled against it by
/// `the_repeatability_taxonomy_accounts_for_every_boundary`, so this comment
/// cannot drift from the constants again.
const REPEATABLE_BOUNDARIES: [Fault; 9] = [
    Fault::RecordScratchCreate,
    Fault::RecordScratchWrite,
    Fault::RecordScratchFlush,
    Fault::RecordBoundedRead,
    Fault::RecordDecode,
    Fault::RecordCanonicalReencode,
    Fault::RecordBindingValidate,
    Fault::RecordTerminalRelationValidate,
    Fault::RecordRetirementReserve,
];

/// The four boundaries a second crash cannot reach, named so the taxonomy is a
/// partition rather than an inclusion list with an anecdote attached.
const UNREPEATABLE_BOUNDARIES: [Fault; 4] = [
    Fault::RecordActivePublish,
    Fault::RecordActiveReobserve,
    Fault::RecordRetireExact,
    Fault::RecordRetiredReobserve,
];

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
            "gwz-r2d-record-matrix-{label}-{}-{}",
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

/// One fresh-process admission on the requested target. Mirrors
/// `tests_leaf_fault_matrix.rs`, which mirrors
/// `admission/tests_fault_matrix.rs:173-200`.
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

/// A fresh-process recovery with no admission edge, exactly as Step 2.1's
/// matrix does: admission runs once, in `prepare`, because its exactness
/// predicate requires the published action directory to hold the resident
/// reservation and no other child.
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

fn reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([0x24; 32]),
        RequestOwnerBindingV1::new([0x2d; 32]),
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

/// Payloads on both sides of the frozen record bound and of the observer's
/// streaming window, so every restart re-streams multi-chunk payloads that a
/// record-sized budget could never have held.
fn source_bytes() -> Vec<u8> {
    (0..40_000_u32).map(|index| (index % 251) as u8).collect()
}

fn goal_bytes() -> Vec<u8> {
    (0..37_000_u32).map(|index| (index % 239) as u8).collect()
}

/// One fresh-process resolution of the whole authority cycle.
///
/// The resolution is derived from what is resident, never from a caller's
/// belief about how far the previous process reached: a resident retired alias
/// is the settled state, a resident active record resumes at the parse, and
/// anything else installs first.
fn attempt(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) -> Result<(), CheckedFsError> {
    recover(fixture, variant);
    let action_root = fixture.action_root(variant, expected);
    let action = expected.action_digest();
    let parent = retain_action(
        &fixture.catalog_root(variant),
        &RootEntryNameV1::ActiveAction(action).name(),
    );

    if action_root
        .join(slot_name(expected, BaseActionSlotV1::RetiredAuthorityAlias))
        .exists()
    {
        return Ok(());
    }

    let (proof, observation) = {
        let source = ExpectedPayloadV1::new(source_bytes());
        let goal = ExpectedPayloadV1::new(goal_bytes());
        let mut namespace = BarrierNamespaceV1::default();
        let proof = observe_streamed_payloads(
            &parent,
            action,
            ObservedLeafWriterClassV1::GwzWritten,
            &source,
            &goal,
            &mut namespace,
            (0, 1),
        )?;
        let owner =
            retained_authority_observation_owner(AuthorityTransactionV1::from_streamed_proof(
                expected.request_owner_binding(),
                proof.clone(),
            ));
        let observation = owner.observe(expected).map_err(|_| {
            CheckedFsError::ambiguous("authority observation", "the transaction did not issue")
        })?;
        (proof, observation)
    };

    if !action_root
        .join(slot_name(expected, BaseActionSlotV1::Authority))
        .exists()
    {
        let record = CheckedAuthorityRecordV1::issue(&observation).map_err(|_| {
            CheckedFsError::ambiguous("authority record", "the observation did not issue a record")
        })?;
        install_authority_record(&parent, action, &record)?;
    }

    let bound = read_resident_authority_record(&parent, action, expected, &observation)?;
    validate_terminal_relation(&parent, action, &bound, &proof)?;
    retire_authority_record(&parent, action)?;
    Ok(())
}

/// Admits the action once and installs both payload leaves, so every later
/// resolution streams leaves that are already durable.
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
        &source_bytes(),
    );
    write_leaf(
        &action,
        &slot_name(expected, BaseActionSlotV1::GoalPayload),
        &goal_bytes(),
    );
}

fn settle(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    context: &str,
) {
    attempt(fixture, variant, expected)
        .unwrap_or_else(|error| panic!("{context}: the restart must settle: {error:?}"));
}

/// The settled census: the record retired onto its scheduled alias, its active
/// slot free, its write-ahead scratch consumed, both payload leaves untouched.
fn settled_census(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
) -> Vec<(String, u64)> {
    let mut rows = census(&fixture.action_root(variant, expected));
    rows.sort();
    rows
}

fn assert_settled(
    fixture: &MatrixFixture,
    variant: TargetVariantV1,
    expected: &ActionCapacityReservationV1,
    context: &str,
) {
    let rows = settled_census(fixture, variant, expected);
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert!(
        names.contains(&slot_name(expected, BaseActionSlotV1::RetiredAuthorityAlias).as_str()),
        "{context}: the retired authority alias must be resident: {names:?}"
    );
    assert!(
        !names.contains(&slot_name(expected, BaseActionSlotV1::Authority).as_str()),
        "{context}: the active authority slot must be free: {names:?}"
    );
    assert!(
        !names.contains(&slot_name(expected, BaseActionSlotV1::AuthorityScratch).as_str()),
        "{context}: the write-ahead scratch must be consumed: {names:?}"
    );
    let source = slot_name(expected, BaseActionSlotV1::SourcePayload);
    let goal = slot_name(expected, BaseActionSlotV1::GoalPayload);
    assert_eq!(
        rows.iter()
            .find(|(name, _)| *name == source)
            .map(|(_, length)| *length),
        Some(source_bytes().len() as u64),
        "{context}: the source payload leaf is untouched by the record cycle"
    );
    assert_eq!(
        rows.iter()
            .find(|(name, _)| *name == goal)
            .map(|(_, length)| *length),
        Some(goal_bytes().len() as u64),
        "{context}: the goal payload leaf is untouched by the record cycle"
    );
}

/// The executed key list must reconcile against the vocabulary's own
/// `record.*` inventory, so a key added to the family without a matrix row
/// fails here rather than silently escaping
/// (`interface_tests/fault_expected_keys.rs`; the
/// `catalog/bootstrap/tests.rs:358-372` reconciliation).
fn reconcile_executed_keys() {
    let mut actual = RECORD_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    actual.sort();
    let mut declared = Fault::all()
        .into_iter()
        .map(|key| key.stable_key())
        .filter(|key| key.starts_with("record."))
        .collect::<Vec<_>>();
    declared.sort();
    assert_eq!(
        actual, declared,
        "every record.* key must have an executed matrix row in the package that converts its edges"
    );
}

fn suffix(stable: &str) -> String {
    stable.replace('.', "-")
}

fn run_interruption_matrix(variant: TargetVariantV1) {
    reconcile_executed_keys();
    for key in RECORD_MATRIX {
        let stable = key.stable_key();
        let fixture = MatrixFixture::new(&format!("{}-{}", variant.label(), suffix(&stable)));
        let expected = reservation();
        prepare(&fixture, variant, &expected);

        run_next_record_fault(key, move || {
            panic!("simulated authority-record process stop");
        });
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt(&fixture, variant, &expected);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {stable} on {}",
            variant.label()
        );

        settle(&fixture, variant, &expected, &stable);
        assert_settled(&fixture, variant, &expected, &stable);
        let first = settled_census(&fixture, variant, &expected);

        settle(&fixture, variant, &expected, &format!("{stable} (resume)"));
        let second = settled_census(&fixture, variant, &expected);
        assert_eq!(
            first,
            second,
            "{stable}: a settled resume must mutate nothing on {}",
            variant.label()
        );

        println!(
            "record matrix | {} | {stable} | first run: interrupted | fresh-process retry: \
             settled | terminal rows: {} | resume: unchanged",
            variant.label(),
            first.len()
        );
    }
}

fn run_repeated_boundary_crashes(variant: TargetVariantV1) {
    for key in REPEATABLE_BOUNDARIES {
        let stable = key.stable_key();
        let fixture =
            MatrixFixture::new(&format!("{}-repeat-{}", variant.label(), suffix(&stable)));
        let expected = reservation();
        prepare(&fixture, variant, &expected);

        let mut rounds = Vec::new();
        for round in 0..REPEATED_CRASH_ROUNDS {
            run_next_record_fault(key, move || {
                panic!("simulated authority-record process stop");
            });
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = attempt(&fixture, variant, &expected);
            }));
            assert!(
                interrupted.is_err(),
                "{stable}: round {round} did not re-cross the boundary on {}",
                variant.label()
            );
            let mut rows = census(&fixture.action_root(variant, &expected))
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>();
            rows.sort();
            rounds.push(rows);
        }
        assert!(
            rounds.windows(2).all(|pair| pair[0] == pair[1]),
            "{stable}: repeated crashes must keep the slot name set stable on {}: {rounds:?}",
            variant.label()
        );

        settle(&fixture, variant, &expected, &stable);
        assert_settled(&fixture, variant, &expected, &stable);
        println!(
            "record repeat | {} | {stable} | {REPEATED_CRASH_ROUNDS} rounds | stable slots | \
             settles after",
            variant.label()
        );
    }
}

/// The taxonomy is a partition of the executed matrix: every boundary is either
/// repeatable or explicitly named unrepeatable, exactly once. This is what the
/// review found missing — a comment and a constant that could disagree.
#[test]
fn the_repeatability_taxonomy_accounts_for_every_boundary() {
    let mut partition = REPEATABLE_BOUNDARIES
        .iter()
        .chain(UNREPEATABLE_BOUNDARIES.iter())
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    partition.sort();
    let mut executed = RECORD_MATRIX
        .iter()
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    executed.sort();
    assert_eq!(
        partition, executed,
        "every executed record.* boundary must be classified exactly once as repeatable or \
         unrepeatable"
    );
}

#[test]
fn record_interruption_restart_convergence_matrix_on_a_workspace_target() {
    run_interruption_matrix(TargetVariantV1::Workspace);
}

#[test]
fn record_interruption_restart_convergence_matrix_on_a_git_directory_target() {
    run_interruption_matrix(TargetVariantV1::GitDirectory);
}

#[test]
fn repeated_same_boundary_crashes_keep_stable_slots_on_a_workspace_target() {
    run_repeated_boundary_crashes(TargetVariantV1::Workspace);
}

#[test]
fn repeated_same_boundary_crashes_keep_stable_slots_on_a_git_directory_target() {
    run_repeated_boundary_crashes(TargetVariantV1::GitDirectory);
}
