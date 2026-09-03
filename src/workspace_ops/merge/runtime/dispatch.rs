use std::path::Path;

use super::super::{
    FileMergeStore, MergeStore, RecordVersion, abort, continue_op, discover_open_before_manifest,
    discover_open_envelope_before_manifest, gc, start, status, v1_lifecycle,
    validate_merge_request,
};
use super::mutation_guard::guarded_workspace_root;
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::ModelResult;
use crate::operation::{EventSink, OperationRequest};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;

/// All environmental dependencies used by the merge lifecycle are explicit.
pub(crate) struct MergeDependencies<'a, B, S, C, I> {
    pub backend: &'a B,
    pub store: &'a S,
    pub clock: &'a C,
    pub ids: &'a mut I,
    pub events: &'a dyn EventSink,
}

/// A1's route from the shared merge dispatch into the v1 record lifecycle.
///
/// The v1 lifecycle is implemented only for the sealed production authority
/// (`MergeAuthorityBackend`), while the dependency-injected v0 seam
/// (`handle_merge_with_dependencies`) is generic over any `GitBackend`. This
/// trait is the seam between the two: the authority path installs
/// `AuthorityV1Router` and reaches the v1 lifecycle; every other backend gets
/// `AbsentV1Router`, which fails closed with a typed refusal rather than
/// handing a v1 record to the v0 engine.
pub(in crate::workspace_ops::merge) trait V1Router {
    /// Create and run one start whose selected record version is v1.
    ///
    /// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1,
    /// 2026-09-03): `filesystem_strict` is threaded from the request, not read
    /// from it here — the v1 start owner is where the crash-recovery decision
    /// is made and where the flag turns a warning back into a refusal.
    fn start(
        &self,
        root: &Path,
        record: super::super::MergeOperationRecord,
        filesystem_strict: bool,
        context: &crate::operation::OperationContext,
        emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse>;

    /// Run the A1 adaptation preflight for a mutating command on an open v0
    /// record, returning whether the record is now v1.
    fn adapt(
        &self,
        root: &Path,
        request: &crate::MergeRequest,
        open: &super::super::OpenRecordEnvelope,
    ) -> ModelResult<bool>;

    /// Serve one command whose open record is v1.
    ///
    /// DR-1 ship (1) W3: a continue decides crash recovery for its own process
    /// and emits at most one diagnostic, so this arm needs the emitter.
    fn command(
        &self,
        root: &Path,
        merge_id: &str,
        request: &crate::MergeRequest,
        context: &crate::operation::OperationContext,
        emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse>;
}

pub(in crate::workspace_ops::merge) struct AuthorityV1Router<'a, B> {
    backend: &'a B,
}

impl<B: MergeAuthorityBackend> V1Router for AuthorityV1Router<'_, B> {
    fn adapt(
        &self,
        root: &Path,
        request: &crate::MergeRequest,
        open: &super::super::OpenRecordEnvelope,
    ) -> ModelResult<bool> {
        adapt_before_mutating(self.backend, root, request, open)
    }

    fn start(
        &self,
        root: &Path,
        record: super::super::MergeOperationRecord,
        filesystem_strict: bool,
        context: &crate::operation::OperationContext,
        emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse> {
        v1_lifecycle::handle_start_durable_v1(
            self.backend,
            root,
            record,
            filesystem_strict,
            context,
            emitter,
        )
    }

    fn command(
        &self,
        root: &Path,
        merge_id: &str,
        request: &crate::MergeRequest,
        context: &crate::operation::OperationContext,
        emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse> {
        v1_lifecycle::handle_v1_command(self.backend, root, merge_id, request, context, emitter)
    }
}

#[cfg(test)]
struct AbsentV1Router;

#[cfg(test)]
impl V1Router for AbsentV1Router {
    /// The dependency-injected v0 seam supplies its own `MergeStore`, so a
    /// filesystem-reading migration would bypass the very store the seam
    /// exists to control — and there is no v1 route to continue onto. No
    /// migration means the v0 lifecycle stays in command, which is the
    /// [P1-1]-safe direction.
    fn adapt(
        &self,
        _root: &Path,
        _request: &crate::MergeRequest,
        _open: &super::super::OpenRecordEnvelope,
    ) -> ModelResult<bool> {
        Ok(false)
    }

    fn start(
        &self,
        _root: &Path,
        _record: super::super::MergeOperationRecord,
        _filesystem_strict: bool,
        _context: &crate::operation::OperationContext,
        _emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse> {
        Err(no_authority("start a v1 merge record"))
    }

    fn command(
        &self,
        _root: &Path,
        _merge_id: &str,
        _request: &crate::MergeRequest,
        _context: &crate::operation::OperationContext,
        _emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse> {
        Err(no_authority("serve a v1 merge record"))
    }
}

#[cfg(test)]
fn no_authority(what: &str) -> crate::model::ModelError {
    crate::model::ModelError::new(
        crate::model::ErrorCode::UnsupportedOperation,
        format!(
            "this backend cannot {what}; the v1 record lifecycle requires the production Git authority"
        ),
    )
}

/// First-class merge service entry. I0 validates and dispatches only; feature
/// milestones replace typed phase errors without changing this public signature.
/// A1 narrowed this bound from `GitBackend` to `MergeAuthorityBackend`.
///
/// The v1 record lifecycle is implemented only for the sealed production
/// authority (`git::MergeAuthorityBackend`, sealed to `Git2Backend`), and the
/// activation routes v1 records into it from here. Every production and test
/// caller already passes `Git2Backend`; the dependency-injected v0 seam
/// (`handle_merge_with_dependencies`) keeps the wider `GitBackend` bound and
/// never reaches a v1 record.
pub fn handle_merge<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    handle_merge_with_events(
        backend,
        start,
        request,
        operation_id,
        &crate::operation::NullSink,
    )
}

pub fn handle_merge_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    let operation_id = operation_id.into();
    let store = FileMergeStore;
    let clock = SystemClock;
    let mut ids = OperationScopedIds::new(&operation_id);
    handle_merge_invocation(
        MergeDependencies {
            backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events,
        },
        start,
        request,
        operation_id,
        true,
        &AuthorityV1Router { backend },
    )
}

/// Dependency-injected lifecycle seam used by the persistence milestones.
#[cfg(test)]
pub(crate) fn handle_merge_with_dependencies<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    handle_merge_invocation(
        dependencies,
        start,
        request,
        operation_id.into(),
        false,
        &AbsentV1Router,
    )
}

/// Run one accepted merge invocation under its single lifecycle owner.
///
/// Public entry points request the authoritative start gate. Dependency-
/// injected tests use the same lifecycle owner but deliberately bypass that
/// filesystem-owned guard so their supplied store remains authoritative.
fn handle_merge_invocation<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: String,
    enforce_start_gate: bool,
    v1: &dyn V1Router,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    let emitter = crate::operation::EventEmitter::from_request_meta(
        operation_id.clone(),
        &request.meta,
        dependencies.events,
        0,
    );
    emitter.operation_started();
    let result = (|| {
        // COUPLED with `validate.rs`'s NoFf refusal (Safety review §2.2 R1/R2).
        // The `request.mode != Some(NoFf)` exclusion that stood here was the
        // other half of that refusal: a NoFf start could not reach record
        // creation, so its custom message was deliberately left unvalidated.
        // Both halves fall in the same edit; every start now validates its
        // custom message before creation, NoFf included.
        if request.op == crate::MergeOp::Start
            && let Some(message) = request.message.as_deref()
        {
            super::super::integration::validate_custom_commit_message(message)?;
        }
        let context = OperationRequest::Merge(request.clone()).context(operation_id)?;
        let (_start_guard, effective_start) =
            if enforce_start_gate && request.op == crate::MergeOp::Start {
                guarded_workspace_root(
                    start,
                    request.meta.workspace.as_ref(),
                    crate::operation::OpenMergeCommand::MergeStart,
                    request.meta.dry_run.unwrap_or(false),
                )?
            } else {
                (None, start.to_path_buf())
            };
        validate_merge_request(&request)?;
        dispatch_merge(
            dependencies,
            &effective_start,
            request,
            context,
            &emitter,
            v1,
            _start_guard,
        )
    })();
    emitter.operation_finished();
    result
}

fn dispatch_merge<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    context: crate::operation::OperationContext,
    emitter: &crate::operation::EventEmitter<'_>,
    v1: &dyn V1Router,
    start_guard: Option<super::WorkspaceMutationGuard>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    if request.op == crate::MergeOp::Start {
        return start::handle_start_durable(
            dependencies,
            start,
            &request,
            &context,
            emitter,
            v1,
            start_guard,
        );
    }
    drop(start_guard);
    let root = resolve_recovery_root(dependencies.store, start, &request)?;
    // A1 (Safety review §2.2 R3 / §2.4): the envelope registry decides which
    // lifecycle owns this record, before any lifecycle touches it. A v1 open
    // record goes to the v1 service; a v0 open record stays on the v0
    // lifecycle, which is exactly what keeps [P1-1]'s recoverable v0 crash
    // prefixes recoverable — see `v0_route_owns_v0_records` in
    // `start/../runtime/tests`.
    if let Some(open) = super::super::classify_open_record(&root)? {
        if open.version == RecordVersion::V1 {
            return v1.command(&root, &open.merge_id, &request, &context, emitter);
        }
        if v1.adapt(&root, &request, &open)? {
            return v1.command(&root, &open.merge_id, &request, &context, emitter);
        }
    }
    match request.op {
        crate::MergeOp::Start => unreachable!("start returned above"),
        crate::MergeOp::Status => status::handle_status(
            dependencies.backend,
            dependencies.store,
            &root,
            request.merge_id.as_deref(),
            &context,
        ),
        crate::MergeOp::Resume => continue_op::handle_continue(
            dependencies.backend,
            dependencies.store,
            &root,
            &request,
            &context,
            emitter,
        ),
        crate::MergeOp::Abort => abort::handle_abort(
            dependencies.backend,
            dependencies.store,
            &root,
            &request,
            &context,
            emitter,
        ),
        crate::MergeOp::Gc => gc::handle_gc(
            dependencies.backend,
            dependencies.store,
            &root,
            request.merge_id.as_deref(),
            &context,
        ),
    }
}

/// The A1 adaptation preflight for a mutating command on an open v0 record
/// (Safety review §2.4). Returns whether the record is now v1 and the command
/// belongs to the v1 lifecycle.
///
/// Two conditions of the review are load-bearing here.
///
/// **[P2-1]** — the preflight is gated on the cheap `Finalizing`/normal-mode
/// state pre-classification (`AdaptationPrecheck`), so `B-NOT-STARTED` and
/// `B-PREPARING-EMPTY` — open v0 progress shapes a pre-A1 binary's crash can
/// leave on disk, with zero fixtures — never reach `validate_v0_structure`
/// through this new path. Only one-member `Finalizing` normal-mode shapes are
/// whitelisted at all, so nothing skipped here could have migrated.
///
/// **[P1-1]** — a typed adapter refusal is the *migration's* answer, never
/// the *command's*. The C-1 dispositions make `adapt_open` refuse F-MARKER
/// and F-LOCK with `PublicationPrefixMismatch`; those are exactly the crash
/// prefixes the v0 lifecycle resumes to `Completed` today. Surfacing that
/// refusal as the resume outcome would turn currently-recoverable states into
/// permanent wedges, so every non-`Upgraded` answer — `ValidUnlisted` and
/// every typed refusal alike — leaves the v0 lifecycle in command, which the
/// contract's own text requires: "An existing mutating v0 command remains on
/// the existing v0 lifecycle and may write v0 only when that path's existing
/// preflight authorizes it."
fn adapt_before_mutating<B: MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    request: &crate::MergeRequest,
    open: &super::super::OpenRecordEnvelope,
) -> ModelResult<bool> {
    if !matches!(request.op, crate::MergeOp::Resume | crate::MergeOp::Abort) {
        return Ok(false);
    }
    if open.adaptation != super::super::AdaptationPrecheck::MayAdapt {
        return Ok(false);
    }
    // R2-E E4.1 review [P1-1]: prove the destination lifecycle can serve this
    // record BEFORE the upgrade writes anything, and hold the window open
    // across the write. The guard drops when this function returns, before the
    // command re-acquires it.
    let _window = if request.op == crate::MergeOp::Resume {
        match forward_lifecycle_viability_window(root) {
            Some(guard) => Some(guard),
            None => return Ok(false),
        }
    } else {
        None
    };
    match super::super::upgrade_open_v0(
        backend,
        root,
        &open.merge_id,
        crate::VERSION,
        super::super::AtomicUpgradeFault::None,
    ) {
        Ok(super::super::AtomicUpgradeOutcome::Upgraded { .. }) => Ok(true),
        Ok(super::super::AtomicUpgradeOutcome::ValidUnlisted) | Err(_) => Ok(false),
    }
}

/// Proves the v1 forward lifecycle can serve this record and holds the proof
/// while the upgrade writes, or answers `None` — one more non-`Upgraded`
/// answer, leaving the v0 lifecycle in command to complete the record.
///
/// **E4.1 review [P1-1]:** the upgrade is durable and one-way, so upgrading
/// into a lifecycle that then refuses is what wedged an interrupted ORDINARY
/// merge — the class the [P1-1] doctrine above rejects. **Only `Resume` asks:**
/// abort routes to the reverse service and its capability-free lease, so that
/// route cannot be made unviable here and an abort never creates a catalog.
/// **Scoped by path** (2026-09-02, CapabilityFreeAmendment §6): an abort that
/// re-verifies a checked artifact still takes the legacy identity probe there.
/// **The lock:** a catalog lease is borrowable only from a held one, and the
/// adapter runs after `drop(start_guard)`; the guard is held across
/// `upgrade_open_v0`, which takes none of its own, and released before the
/// command re-acquires it. A lock we cannot take is itself a `None` — the v0
/// route takes the same lock and reports contention in its own voice.
///
/// **Residual, disclosed (review R4):** the forward service re-proves
/// activation one layer down, so a catalog lost between the two — a race, not a
/// capability — refuses after the upgrade. Not a wedge: `gwz merge --abort`
/// clears it, and this abort is capability-free BY PATH (§6, 2026-09-02): it
/// touches no checked artifact. Driven.
/// **DR-1 ship (1) W3 leaves this UNTOUCHED** (`GwzM5-8DR1-WarnOrRefuse-Charter.md`
/// §3.1, 2026-09-03). The crash-recovery decision governs `--no-ff` starts and
/// v1 continues; this is the v0->v1 ADAPTER's window, on the ordinary/`--ff-only`
/// route that does not reach the decision point at all until M5c. Its answer is
/// already "probe, do not refuse" — a failed activation keeps the v0 lifecycle
/// in command — which is the precedent the decision point generalises, not a
/// second place to make the same decision. Making it consult the decision would
/// route an ordinary merge onto the v1 lifecycle on a volume the catalog cannot
/// bind, which ship (1) explicitly does not do.
fn forward_lifecycle_viability_window(
    root: &Path,
) -> Option<crate::operation::WorkspaceMutatorLock> {
    let guard = crate::operation::WorkspaceMutatorLock::acquire(root).ok()?;
    crate::checked_artifact::entry::activate_workspace_catalog(guard.catalog_mutation_lease())
        .ok()?;
    Some(guard)
}

fn resolve_recovery_root<S: MergeStore>(
    store: &S,
    start: &Path,
    request: &crate::MergeRequest,
) -> ModelResult<std::path::PathBuf> {
    if let Some(root) = request
        .meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.root.as_ref())
    {
        return Ok(std::path::PathBuf::from(root));
    }
    // A1: version-agnostic first. The v0 store's own decoder installs v0
    // only, so an open v1 record must be discovered by its envelope or root
    // resolution would fail before the dispatch could route it.
    if let Some(found) = discover_open_envelope_before_manifest(start)? {
        return Ok(found.root);
    }
    if let Some(recovery) = discover_open_before_manifest(store, start)? {
        return Ok(recovery.root);
    }
    crate::workspace_ops::resolve_workspace_root(start, request.meta.workspace.as_ref())
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> crate::runtime::clock::TimestampMs {
        crate::operation::now_ms()
    }
}

struct OperationScopedIds {
    suffix: String,
    next: u64,
}

impl OperationScopedIds {
    fn new(operation_id: &str) -> Self {
        let suffix = operation_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        Self { suffix, next: 0 }
    }
}

impl IdProvider for OperationScopedIds {
    fn next_id(&mut self, prefix: &str) -> crate::runtime::ids::GeneratedId {
        self.next += 1;
        crate::runtime::ids::GeneratedId::new(format!("{prefix}_{}_{:04}", self.suffix, self.next))
    }
}
