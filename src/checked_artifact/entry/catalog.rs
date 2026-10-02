#![forbid(clippy::disallowed_methods)]

use super::*;

use super::super::catalog::recover_or_create;
/// DR-1 ship (1) W3 — the CATALOG-FREE creation lease's parent half
/// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03).
///
/// Below the bar there is no catalog, so `bootstrap_merge_start_parents`'
/// managed-parent provider cannot install `.gwz/merge` and the preservation
/// bundle prefix. Both are still made through the LEGACY checked boundary's own
/// `prepare_parent` — the v0 store's route — and never by a raw `create_dir_all`
/// and never inside the managed-parent provider seam that
/// `interface_tests/r2d_seam_freeze.rs` freezes. `create_open` still refuses a
/// missing parent (charter §4.1), so this is the step that makes its refusal
/// unreachable on the warned path exactly as the bootstrap does on the other.
pub(crate) fn prepare_merge_start_parents_uncatalogued(
    filesystem: &dyn FileSystem,
    root: &Path,
) -> ModelResult<()> {
    CheckedArtifact::prepare_parent_in(
        filesystem,
        root,
        Path::new(MERGE_RECORD_PARENT),
        ErrorCode::MergeRecoveryRequired,
        "merge record parent",
    )?;
    prepare_merge_store_parents_in(filesystem, root)
}

/// R2-E Phase E4 Step E4.1 (O2): the first production catalog activation.
///
/// `recover_or_create` is `pub(in crate::checked_artifact)`, so its caller must
/// live inside this module tree, and this module is the crate's declared
/// production checked boundary — so the door is here and the operation calls
/// it. Its callers are the arms that mutate a record toward v1 semantics —
/// `v1_lifecycle`'s `V1MutationLease::acquire_activated` (start's creation
/// lease and the forward service loop) and `dispatch.rs`'s A1 adapter, which
/// proves viability here before its durable v0->v1 upgrade.
///
/// **Where the capability is required, and where it is not** (E0.2 §5.2 with
/// E0.2b §6.4's fifth ground, corrected by the E4.1 review's [P1-1]/[P2-1]):
/// `WorkspaceMutatorLock::try_acquire` probes no durable identity, here or
/// after. `gwz repo create`, `init-from-sources`, GC, the mutation guard and
/// `gwz merge --abort` never reach this door — so a refusal always has an exit.
/// An ordinary or `--ff-only` merge reaches it only through the A1 adapter's
/// viability window, where a refusal is never surfaced: the v0 lifecycle stays
/// in command. What refuses, typed, is a `--no-ff` start and the resume of a
/// record already at v1.
///
/// **The retained catalog is dropped.** Activation proves the catalog and
/// leaves it durable; the lease model is that each consumer re-acquires
/// (`coordinator/execution.rs`'s admission session says the same). E4.2
/// converted the merge-record consumers; no further conversion arrives
/// (`GwzM5-8R2E-CapabilityFreeAmendment.md` §7, ADOPTED 2026-09-02 — E4.4-E4.6
/// as chartered do not start, and the three `finalization/execute.rs` forward
/// arms stay raw as the [R2-P3-1] dated residual on the operator's ruling (a)
/// of the same date). Re-pointed at E4.7, 2026-09-02. [2026-09-02, R2-E E4.4-6-B: the E4.2-E4.6 / "awaiting R2-E consumer conversion" range is STALE — E4.4-E4.6 as chartered do not start (GwzM5-8R2E-CapabilityFreeAmendment.md §7); E4.7 EXPIRES or RE-REASONS each, and this package only dates them.]
///
/// **Two scope clauses this door's consumers must not lean on** (E7.2's
/// [R2-P3-1] and its terminal sibling, written at the plan's E4 gate note):
/// a settled barrier ordinal does not imply its target parent's dirents were
/// ever ordered, and a converged-by-observation restart does not imply key #8's
/// retired-root flush or key #9's catalog-root barrier ran on that drive.
/// Converged does not imply flushed; settled does not imply barriered. A
/// consumer that needs either must barrier or flush for itself.
pub(crate) fn activate_workspace_catalog(lease: CatalogMutationLeaseV1<'_>) -> ModelResult<()> {
    recover_or_create(lease)
        .map(|_retained| ())
        .map_err(|cause| render_catalog_refusal(CATALOG_LABEL, cause))
}

/// R2-E Step E4.2 — the first merge record's parent half (ConsumerCheckpoint
/// §10 row `:273`, "`MergeStore` and `PreservationBundles` when missing";
/// frozen clause one, "both parents durable before record").
///
/// **The row's whole creation authority.** Both prefixes are installed by the
/// managed-parent provider through an admitted `ParentOnly` action over a sealed
/// purpose set, never by a raw `create_dir_all` on the writer's side — which is
/// why `store/rewrite.rs` now REFUSES a missing parent instead of making one.
/// Two leases because the Phase-1 owner CONSUMES the retained catalog: admission
/// ends when it returns, and execution recovers again, after admission created
/// the retained directory the execution walk has to find.
///
/// **The §11.3-item-2(b) answer, recorded against freeze `:672-680`
/// (2026-09-01, E4.2).** *For a Git-directory catalog target, which durable root
/// binds a managed parent's prefix?* Its OWN retained root — the Git directory —
/// never the workspace root; and so no production managed parent exists on that
/// variant at all. Four facts settle it: (i) the purposes' declared components
/// are workspace-relative (`bootstrap/managed.rs`), so under a Git directory
/// they have no `.gwz` ancestor and both merge-start purposes fail their minimum
/// retained-parent count — pinned as production behaviour by
/// `tests_provider.rs`'s
/// `a_git_directory_target_refuses_the_workspace_rooted_managed_paths`;
/// (ii) `CheckedActionRequestV1::for_managed_parents` pins
/// `PreCatalogRootKindV1::Workspace` unconditionally, so no managed-parent
/// action can be identified against a Git-directory root kind at all; (iii) this
/// door's lease is workspace-rooted by construction (`catalog_lease/witness.rs`,
/// `WorkspaceRuntime` arm); (iv) no production caller builds a Git-directory
/// catalog lease. Route (b) therefore stands for TEST topologies, where the
/// parent is fixture-placed, and the owner decision closes as: the workspace
/// root binds it, the other variant carrying no production parent.
pub(crate) fn bootstrap_merge_start_parents(
    workspace_id: &str,
    admission: CatalogMutationLeaseV1<'_>,
    execution: CatalogMutationLeaseV1<'_>,
) -> ModelResult<()> {
    let refuse = |cause| render_catalog_refusal("merge start parents", cause);
    let admitted = admit_merge_start_managed_parents(
        workspace_id,
        recover_or_create(admission).map_err(refuse)?,
    )
    .map_err(refuse)?;
    let Some(admitted) = admitted else {
        return Ok(());
    };
    let catalog = recover_or_create(execution).map_err(refuse)?;
    execute_merge_start_managed_parents(workspace_id, &admitted, &catalog)
        .map(|_proved| ())
        .map_err(refuse)
}

/// R2-E Step E4.2 — O13's substantive half on the creation path.
///
/// Row `:280` asks the v1 checked store for "the same purposes and artifact
/// actions" its converted siblings use; this is the creation verb — a checked
/// replacement whose expected fact is `Missing`, publishing onto an absent leaf
/// inside an already-retained parent.
///
/// **M5d step (3) — the RAW arm (`GwzM5-8M5d-Charter.md` §3, 2026-09-03).**
/// This door is the merge's ONLY forward checked door: `commit` is the
/// record-root exception's raw rewrite and the archive and publications are
/// raw already, so it is the one place a handle-fail volume could stop a
/// merge that has already been told it may continue. Below the handle bar it
/// therefore publishes through the neutral raw primitive instead of the
/// boundary, and the charter's table says so in one line: *record create =
/// raw (`write_atomic_verified`)*.
///
/// **Gated by the DECISION, not by a re-probe.** The caller threads the
/// decision `crash_recovery_decision` already made for this process (charter
/// §3.1's "decide once", extended to this door), so the door and the
/// diagnostic can never disagree about which volume this is. Exactly one
/// shape takes the raw arm — `Unsupported { handles_ok: false }`. `Supported`,
/// `Unsupported { handles_ok: true }` and `None` all keep the checked
/// publication, so a create-door handle failure while the decision said
/// handles were fine stays what it is today: an anomaly error, unchanged text.
///
/// The raw arm keeps the checked arm's two guarantees that a user can
/// observe: the same NO-REPLACE semantics (an existing record is refused, not
/// overwritten) and the same re-read verification (the primitive compares the
/// published bytes back). What it does not keep is the catalog, which does
/// not exist on this volume, and crash recovery, which the charter states is
/// absent rather than degraded here.
pub(crate) fn create_merge_store_record(
    filesystem: &dyn FileSystem,
    ids: &gwz_ids::IdSource,
    root: &Path,
    relative: &Path,
    goal: &[u8],
    crash_recovery: Option<&CrashRecoveryDecision>,
) -> ModelResult<()> {
    if matches!(
        crash_recovery,
        Some(CrashRecoveryDecision::Unsupported {
            handles_ok: false,
            ..
        })
    ) {
        return create_merge_store_record_raw(filesystem, ids, root, relative, goal);
    }
    let artifact = CheckedArtifact::acquire_with_escape_in(
        filesystem,
        CheckedArtifactPolicy::workspace(root),
        relative,
        ErrorCode::MergeRecoveryRequired,
        format!("merge record '{}'", relative.display()),
        IdentityGapEscape::Substrate,
    )?;
    // Row `:273`'s clause said out loud, rather than left as the generic
    // ambiguity `classify_replace_exact` reports for an absent parent.
    if !artifact.parent_is_canonical()? {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "merge record parent is missing or noncanonical; it is bootstrapped before the record",
        ));
    }
    artifact.replace_exact(&CheckedArtifactFact::Missing, goal)
}

#[rustfmt::skip]
/// The catalog doors' error rendering, as a named function.
///
/// E4.1 review [P3-2]: inline, the three arms were unreachable from a test
/// without a filesystem that lacks the capability, so precondition 1's sentence
/// was driven only by hand on a real FAT32 volume. Named, a direct-constructor
/// row pushes each arm through it. `pub(super)` and not `pub(crate)`:
/// `CheckedFsError` is subsystem private and a crate-visible signature over it
/// trips `clippy::private_interfaces` (E4.1(c) flag 5, proven).
pub(in crate::checked_artifact) fn render_catalog_refusal(label: &str, cause: CheckedFsError) -> ModelError {
    match cause {
        CheckedFsError::Unsupported { capability, detail } => ModelError::new(
            ErrorCode::UnsupportedOperation,
            match capability.remedy() {
                // Precondition 1: the one gap a user can act on arrives as
                // the sentence that says what to do, with the substrate's
                // own words kept for diagnosis.
                Some(remedy) => format!("checked {label}: {remedy} (detail: {detail})"),
                None => format!("checked {label} is unsupported: {detail}"),
            },
        ),
        CheckedFsError::Io { operation, source } => ModelError::new(
            ErrorCode::IoError,
            format!("checked {label} {operation}: {source}"),
        ),
        CheckedFsError::Ambiguous { fact, detail } => ModelError::new(
            ErrorCode::IoError,
            format!("checked {label} rejected {fact}: {detail}"),
        ),
    }
}

/// E4.1's activation label, spelled once so door and guard cannot drift.
pub(in crate::checked_artifact) const CATALOG_LABEL: &str = "merge artifact catalog";
