use std::path::Path;

use super::super::{
    FileMergeStore, MergeStore, discover_open_envelope_before_manifest, gc, start, status,
    v1_lifecycle, validate_merge_request,
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
        record: super::super::model::v1::MergeOperationRecordV1,
        filesystem_strict: bool,
        context: &crate::operation::OperationContext,
        emitter: &crate::operation::EventEmitter<'_>,
    ) -> ModelResult<crate::MergeResponse>;

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
    fn start(
        &self,
        root: &Path,
        record: super::super::model::v1::MergeOperationRecordV1,
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
    fn start(
        &self,
        _root: &Path,
        _record: super::super::model::v1::MergeOperationRecordV1,
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
    let root = resolve_recovery_root(start, &request)?;
    // **M5d (`GwzM5-8M5d-Charter.md` §2).** The envelope decides, and it is
    // all that is read: a v0 envelope is not decoded, not continued, not
    // aborted, not migrated and not projected as a merge. It is an open
    // operation with exactly one answer, and the A1 adapter that used to
    // stand here is deleted along with the lifecycle it fed.
    if let Some(open) = super::super::classify_open_record(&root)? {
        open.refuse_if_pre_014()?;
        return v1.command(&root, &open.merge_id, &request, &context, emitter);
    }
    // No open record. The three answers below used to live in the v0 engine
    // (`continue_op/execution.rs`, `abort/mod.rs`) and were unreachable from
    // `v1.command`, which only ran when a record existed — so deleting the
    // engine would have deleted them (parity inventory F-13 / X-7 / X-8).
    match request.op {
        crate::MergeOp::Start => unreachable!("start returned above"),
        crate::MergeOp::Status => {
            status::handle_status(&root, request.merge_id.as_deref(), &context)
        }
        crate::MergeOp::Resume => no_open_merge(&request, "there is no open merge to continue"),
        crate::MergeOp::Abort => no_open_merge(&request, "no coordinated merge is open"),
        crate::MergeOp::Gc => gc::handle_gc(
            dependencies.backend,
            dependencies.store,
            &root,
            request.merge_id.as_deref(),
            &context,
        ),
    }
}

/// `--continue` / `--abort` with nothing open.
///
/// The v0 engine answered this in two places, each with its own sentence, and
/// each additionally served a named id that turned out to be already closed by
/// loading it out of `done/`. Both sentences are kept verbatim. The
/// archived-id arm is served by `--status <id>` instead: the v1 lifecycle's
/// terminal answer for a named record already comes from the archived
/// projection, and a `--continue` on a completed record is not a continue.
fn no_open_merge(
    request: &crate::MergeRequest,
    sentence: &str,
) -> ModelResult<crate::MergeResponse> {
    let detail = request.merge_id.as_deref().map_or_else(
        || sentence.to_owned(),
        |merge_id| {
            format!(
                "{sentence}; merge '{merge_id}' is not open. \
                 Use `gwz merge --status {merge_id}` to read it if it is archived"
            )
        },
    );
    Err(crate::model::ModelError::new(
        crate::model::ErrorCode::OperationNotFound,
        detail,
    ))
}

fn resolve_recovery_root(
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
    // By envelope, and only by envelope: root resolution must not depend on
    // being able to decode the record it finds (charter §2).
    if let Some(found) = discover_open_envelope_before_manifest(start)? {
        return Ok(found.root);
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
