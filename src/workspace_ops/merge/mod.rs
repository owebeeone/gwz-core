mod abort;
mod continue_op;
mod finalize;
#[allow(dead_code)] // Frozen M2b-A1 interface; consumed by finalization in M2b-A2.
pub(crate) mod marker;
mod model;
mod pending;
mod plan;
mod recovery;
mod response;
mod start;
mod status;
mod store;
mod validate;

pub(crate) use model::*;
pub(crate) use recovery::*;
pub(crate) use store::FileMergeStore;
pub(crate) use validate::validate_merge_request;

use std::path::{Path, PathBuf};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventSink, OperationRequest, WorkspaceMutatorLock};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;

/// Persistence seam frozen at I0. M1 provides the filesystem implementation.
/// Initial status discovers only the open record; archived enumeration and
/// id-qualified archived status deliberately remain outside this M1 seam.
#[allow(dead_code)] // Remove when M1 wires the durable merge store.
pub(crate) trait MergeStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        unsupported_store("discover_open")
    }
    fn load(&self, _root: &Path, _merge_id: &str) -> ModelResult<MergeOperationRecord> {
        unsupported_store("load")
    }
    fn write_open(&self, _root: &Path, _record: &MergeOperationRecord) -> ModelResult<()> {
        unsupported_store("write_open")
    }
    fn archive(&self, _root: &Path, _merge_id: &str) -> ModelResult<()> {
        unsupported_store("archive")
    }
    fn gc(&self, _root: &Path, _merge_id: Option<&str>) -> ModelResult<()> {
        unsupported_store("gc")
    }
}

#[allow(dead_code)] // Used by the M1 store seam once its default methods are live.
fn unsupported_store<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("merge store method '{method}' is not implemented"),
    ))
}

/// All environmental dependencies used by the merge lifecycle are explicit.
#[allow(dead_code)] // Remove when M1 routes the service through durable dependencies.
pub(crate) struct MergeDependencies<'a, B, S, C, I> {
    pub backend: &'a B,
    pub store: &'a S,
    pub clock: &'a C,
    pub ids: &'a mut I,
    pub events: &'a dyn EventSink,
}

/// First-class merge service entry. I0 validates and dispatches only; feature
/// milestones replace typed phase errors without changing this public signature.
pub fn handle_merge<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
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
    B: GitBackend,
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
    )
}

/// Central pre-dispatch guard used by synchronous drivers. Recovery discovery
/// intentionally precedes manifest parsing so an invalid in-flight root merge
/// cannot make the gate disappear.
pub fn enforce_workspace_open_merge_gate(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<()> {
    if command.gate_decision() == crate::operation::OpenMergeGateDecision::NotGated {
        return Ok(());
    }
    let store = FileMergeStore;
    let open = if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        store.discover_open(Path::new(root))?
    } else {
        discover_open_before_manifest(&store, start)?.map(|recovery| recovery.record)
    };
    crate::operation::enforce_open_merge_gate(
        open.as_ref().map(|record| record.merge_id.as_str()),
        command,
    )
}

/// Authoritative guard for an existing-workspace mutation.
///
/// The effective request workspace is resolved before locking; the open-merge
/// policy is then checked while the same lock remains held for the caller's
/// mutation. Public mutating handlers migrate to this seam during the M2a
/// remediation wave so direct core callers cannot bypass driver checks.
pub struct WorkspaceMutationGuard {
    root: PathBuf,
    _lock: WorkspaceMutatorLock,
}

impl WorkspaceMutationGuard {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn acquire_workspace_mutation_guard(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
) -> ModelResult<WorkspaceMutationGuard> {
    let root = crate::workspace_ops::resolve_workspace_root(start, workspace)?;
    let lock = WorkspaceMutatorLock::acquire(&root)?;
    let store = FileMergeStore;
    let open = store.discover_open(&root)?;
    crate::operation::enforce_open_merge_gate(
        open.as_ref().map(|record| record.merge_id.as_str()),
        command,
    )?;
    Ok(WorkspaceMutationGuard { root, _lock: lock })
}

/// Resolve and enforce a gated dry-run without taking the mutator lock, or
/// retain the authoritative guard for a real mutation.
pub(crate) fn guarded_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
    command: crate::operation::OpenMergeCommand,
    dry_run: bool,
) -> ModelResult<(Option<WorkspaceMutationGuard>, PathBuf)> {
    if dry_run {
        enforce_workspace_open_merge_gate(start, workspace, command)?;
        return Ok((
            None,
            crate::workspace_ops::resolve_workspace_root(start, workspace)?,
        ));
    }
    let guard = acquire_workspace_mutation_guard(start, workspace, command)?;
    let root = guard.root().to_path_buf();
    Ok((Some(guard), root))
}

pub(crate) fn enforce_open_merge_stage_targets(
    root: &Path,
    targets: &[crate::workspace_ops::StageTarget],
) -> ModelResult<()> {
    let store = FileMergeStore;
    let Some(record) = store.discover_open(root)? else {
        return Ok(());
    };
    let allowed = record
        .participants
        .values()
        .filter(|participant| participant.state == ParticipantState::Conflicted)
        .map(|participant| match participant.target_kind {
            MergeTargetKind::Member => Some(participant.path.as_str()),
            MergeTargetKind::Root => None,
        })
        .collect::<Vec<_>>();
    if targets
        .iter()
        .all(|target| allowed.contains(&target.member_path.as_deref()))
    {
        return Ok(());
    }
    Err(ModelError::new(
        ErrorCode::OpenOperation,
        format!(
            "merge '{}' is open; add may target only its conflicted participants; \
             use merge status to inspect the allowed repositories",
            record.merge_id
        ),
    ))
}

pub(crate) fn persist_operation_transition<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    next: OperationState,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    record.state = record.state.transition(next)?;
    persist_merge_record(store, root, record, emitter)?;
    emitter.operation_state_changed(record.state.into());
    Ok(())
}

pub(crate) fn persist_merge_record<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &MergeOperationRecord,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    store.write_open(root, record)?;
    emitter.artifact_written(open_merge_artifact_path(&record.merge_id));
    Ok(())
}

pub(crate) fn archive_merge_record<S: MergeStore>(
    store: &S,
    root: &Path,
    merge_id: &str,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    store.archive(root, merge_id)?;
    emitter.artifact_written(done_merge_artifact_path(merge_id));
    Ok(())
}

pub(crate) fn emit_merge_member_finished(
    emitter: &crate::operation::EventEmitter<'_>,
    record: &MergeOperationRecord,
    target_id: &str,
) -> ModelResult<()> {
    let participant = record.participants.get(target_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("merge record is missing participant '{target_id}'"),
        )
    })?;
    emitter.merge_member_finished(participant.to_protocol(target_id, &record.source_ref));
    Ok(())
}

fn open_merge_artifact_path(merge_id: &str) -> String {
    format!(".gwz/merge/{merge_id}.yaml")
}

fn done_merge_artifact_path(merge_id: &str) -> String {
    format!(".gwz/merge/done/{merge_id}.yaml")
}

/// M2a's stable handoff into publication. M2b replaces the implementation
/// behind this seam; callers do not publish or advance the accepted lock.
pub(crate) fn enter_finalizing<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &crate::operation::EventEmitter<'_>,
) -> ModelResult<()> {
    persist_operation_transition(store, root, record, OperationState::Finalizing, emitter)
}

/// Dependency-injected lifecycle seam used by the persistence milestones.
#[allow(dead_code)] // Production enters through the public gate; focused tests inject dependencies.
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
    handle_merge_invocation(dependencies, start, request, operation_id.into(), false)
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
        dispatch_merge(dependencies, &effective_start, request, context, &emitter)
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
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    match request.op {
        crate::MergeOp::Start => {
            start::handle_start_durable(dependencies, start, &request, &context, emitter)
        }
        crate::MergeOp::Status => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            status::handle_status(dependencies.backend, dependencies.store, &root, &context)
        }
        crate::MergeOp::Resume => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            continue_op::handle_continue(
                dependencies.backend,
                dependencies.store,
                &root,
                &request,
                &context,
                emitter,
            )
        }
        crate::MergeOp::Abort => {
            let root = resolve_recovery_root(dependencies.store, start, &request)?;
            abort::handle_abort(
                dependencies.backend,
                dependencies.store,
                &root,
                &request,
                &context,
                emitter,
            )
        }
        op => Err(ModelError::new(
            ErrorCode::MergePhaseUnsupported,
            format!("merge operation '{op:?}' is not available"),
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::NullSink;
    use crate::runtime::clock::{FixedClock, TimestampMs};
    use crate::runtime::ids::SequentialIdProvider;
    use crate::workspace_ops::tests::TempDir;
    use std::fs;
    use std::sync::Mutex;

    #[derive(Default)]
    struct CollectingSink(Mutex<Vec<crate::OperationEvent>>);

    impl crate::operation::EventSink for CollectingSink {
        fn deliver(&self, event: crate::OperationEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    struct DirtyOnMemberStartSink {
        member: PathBuf,
        events: Mutex<Vec<crate::OperationEvent>>,
    }

    impl crate::operation::EventSink for DirtyOnMemberStartSink {
        fn deliver(&self, event: crate::OperationEvent) {
            if event.kind == crate::EventKind::MemberStarted {
                fs::write(self.member.join("README.md"), "changed after planning\n").unwrap();
            }
            self.events.lock().unwrap().push(event);
        }
    }

    struct EmptyStore;
    impl MergeStore for EmptyStore {
        fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
            Ok(None)
        }
    }

    struct FailingStore;
    impl MergeStore for FailingStore {
        fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
            Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "injected merge-store failure",
            ))
        }
    }

    fn assert_lifecycle(
        sink: &CollectingSink,
        expected_operation_id: &str,
        expected_request_id: &str,
    ) {
        let events = sink.0.lock().unwrap();
        assert_eq!(
            events.iter().map(|event| event.kind).collect::<Vec<_>>(),
            [
                crate::EventKind::OperationStarted,
                crate::EventKind::OperationFinished,
            ]
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert!(events.iter().all(|event| {
            event.operation_id == expected_operation_id && event.request_id == expected_request_id
        }));
    }

    fn request() -> crate::MergeRequest {
        crate::MergeRequest {
            meta: crate::RequestMeta {
                request_id: "req".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                workspace: Some(crate::WorkspaceRef {
                    root: Some(".".to_owned()),
                    workspace_id: None,
                }),
                ..crate::RequestMeta::default()
            },
            op: crate::MergeOp::Status,
            source_ref: None,
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
        }
    }

    #[test]
    fn invalid_attribution_is_bracketed_by_one_outer_lifecycle() {
        let backend = crate::git::Git2Backend::new();
        let store = EmptyStore;
        let clock = FixedClock::new(TimestampMs(1));
        let sink = CollectingSink::default();
        let mut ids = SequentialIdProvider::new();
        let mut invalid = request();
        invalid.meta.attribution = Some(crate::OperationAttribution {
            git_author: Some(crate::GitObjectIdentity {
                name: String::new(),
                email: "author@example.invalid".to_owned(),
                time_ms: None,
                timezone_offset_minutes: None,
            }),
            ..crate::OperationAttribution::default()
        });
        let supplied_attribution = invalid.meta.attribution.clone();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &sink,
        };

        let error =
            handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_invalid")
                .unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.message.contains("git_identity.name"));
        assert_lifecycle(&sink, "op_invalid", "req");
        assert!(
            sink.0
                .lock()
                .unwrap()
                .iter()
                .all(|event| event.attribution == supplied_attribution)
        );
    }

    #[test]
    fn injected_store_failure_is_bracketed_by_one_outer_lifecycle() {
        let backend = crate::git::Git2Backend::new();
        let store = FailingStore;
        let clock = FixedClock::new(TimestampMs(1));
        let sink = CollectingSink::default();
        let root = TempDir::new("merge-lifecycle-store-failure");
        fs::create_dir_all(root.path().join(".gwz/merge")).unwrap();
        let mut ids = SequentialIdProvider::new();
        let mut status = request();
        status.meta.workspace = None;
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &sink,
        };

        let error = handle_merge_with_dependencies(dependencies, root.path(), status, "op_store")
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
        assert_eq!(error.message, "injected merge-store failure");
        assert_lifecycle(&sink, "op_store", "req");
    }

    #[test]
    fn every_open_merge_state_and_start_mode_is_bracketed_by_one_outer_lifecycle() {
        for state in [
            "awaiting_resolution",
            "halted",
            "recovery_required",
            "finalizing",
        ] {
            for dry_run in [false, true] {
                let root = TempDir::new(&format!("merge-lifecycle-{state}-{dry_run}"));
                let directory = root.path().join(".gwz/merge");
                fs::create_dir_all(&directory).unwrap();
                fs::write(
                    directory.join("merge_1.yaml"),
                    format!(
                        r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_1
operation_id: op_1
state: {state}
source_ref: feature/x
created_at: now
baseline: {{ lock_sha256: lock, manifest_sha256: manifest }}
selected_targets: []
participants: {{}}
"#
                    ),
                )
                .unwrap();
                let backend = crate::git::Git2Backend::new();
                let sink = CollectingSink::default();
                let operation_id = format!("op_{state}_{dry_run}");
                let mut start = request();
                start.op = crate::MergeOp::Start;
                start.source_ref = Some("feature/x".to_owned());
                start.meta.dry_run = dry_run.then_some(true);
                start.meta.workspace = Some(crate::WorkspaceRef {
                    root: Some(root.path().to_string_lossy().into_owned()),
                    workspace_id: None,
                });

                let error = handle_merge_with_events(
                    &backend,
                    root.path(),
                    start,
                    operation_id.clone(),
                    &sink,
                )
                .unwrap_err();

                assert_eq!(error.code, ErrorCode::OpenOperation, "{state} {dry_run}");
                assert!(error.message.contains("merge_1"), "{state} {dry_run}");
                assert_lifecycle(&sink, &operation_id, "req");
            }
        }
    }

    #[test]
    fn successful_member_events_share_the_outer_sequence_and_finish_last() {
        let root = TempDir::new("merge-lifecycle-success");
        let backend = crate::git::Git2Backend::new();
        let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
            root.path(),
            &backend,
            "merge-lifecycle-success-source",
        );
        let manifest = crate::artifact::read_manifest(root.path()).unwrap();
        let member = root.path().join(&manifest.members[0].path);
        let before = backend.head(&member).unwrap().commit.unwrap();
        backend
            .branch_create(&member, "feature/source", "HEAD")
            .unwrap();
        backend.switch_branch(&member, "feature/source").unwrap();
        crate::workspace_ops::tests::commit_file(
            &member,
            "source.txt",
            "source\n",
            "source",
            &[git2::Oid::from_str(&before).unwrap()],
        )
        .unwrap();
        backend.switch_branch(&member, "main").unwrap();
        let sink = CollectingSink::default();
        let mut start = request();
        start.op = crate::MergeOp::Start;
        start.source_ref = Some("feature/source".to_owned());
        start.meta.workspace = Some(crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        });

        let response =
            handle_merge_with_events(&backend, root.path(), start, "op_success", &sink).unwrap();

        assert_eq!(response.state, crate::MergeOperationState::Completed);
        let events = sink.0.lock().unwrap();
        assert_eq!(
            events.first().map(|event| event.kind),
            Some(crate::EventKind::OperationStarted)
        );
        assert_eq!(
            events.last().map(|event| event.kind),
            Some(crate::EventKind::OperationFinished)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == crate::EventKind::OperationStarted)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == crate::EventKind::OperationFinished)
                .count(),
            1
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::MemberStarted)
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == crate::EventKind::MemberFinished)
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (0..events.len() as i64).collect::<Vec<_>>()
        );
    }

    #[test]
    fn backend_preflight_failure_is_bracketed_by_one_outer_lifecycle() {
        let root = TempDir::new("merge-lifecycle-backend-failure");
        let backend = crate::git::Git2Backend::new();
        let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
            root.path(),
            &backend,
            "merge-lifecycle-backend-failure-source",
        );
        let sink = CollectingSink::default();
        let mut start = request();
        start.op = crate::MergeOp::Start;
        start.source_ref = Some("missing/source".to_owned());
        start.meta.workspace = Some(crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        });

        let error = handle_merge_with_events(&backend, root.path(), start, "op_backend", &sink)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::GitCommandFailed);
        assert!(error.message.contains("revspec"));
        assert_lifecycle(&sink, "op_backend", "req");
    }

    #[test]
    fn member_scoped_failure_keeps_intermediate_events_and_finishes_last_once() {
        let root = TempDir::new("merge-lifecycle-member-failure");
        let backend = crate::git::Git2Backend::new();
        let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
            root.path(),
            &backend,
            "merge-lifecycle-member-failure-source",
        );
        let manifest = crate::artifact::read_manifest(root.path()).unwrap();
        let member = root.path().join(&manifest.members[0].path);
        let before = backend.head(&member).unwrap().commit.unwrap();
        backend
            .branch_create(&member, "feature/source", "HEAD")
            .unwrap();
        backend.switch_branch(&member, "feature/source").unwrap();
        crate::workspace_ops::tests::commit_file(
            &member,
            "source.txt",
            "source\n",
            "source",
            &[git2::Oid::from_str(&before).unwrap()],
        )
        .unwrap();
        backend.switch_branch(&member, "main").unwrap();
        let sink = DirtyOnMemberStartSink {
            member,
            events: Mutex::new(Vec::new()),
        };
        let mut start = request();
        start.op = crate::MergeOp::Start;
        start.source_ref = Some("feature/source".to_owned());
        start.meta.workspace = Some(crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        });

        let response =
            handle_merge_with_events(&backend, root.path(), start, "op_member_failure", &sink)
                .unwrap();

        assert_eq!(response.state, crate::MergeOperationState::Halted);
        assert_eq!(
            response.repos[0].state,
            crate::MergeParticipantState::Failed
        );
        assert_eq!(
            response.repos[0].error.as_ref().map(|error| error.code),
            Some(crate::GwzErrorCode::MergeDrift)
        );
        let events = sink.events.lock().unwrap();
        assert_eq!(
            events.first().map(|event| event.kind),
            Some(crate::EventKind::OperationStarted)
        );
        assert_eq!(
            events.last().map(|event| event.kind),
            Some(crate::EventKind::OperationFinished)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == crate::EventKind::OperationStarted)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == crate::EventKind::OperationFinished)
                .count(),
            1
        );
        let member_started = events
            .iter()
            .position(|event| event.kind == crate::EventKind::MemberStarted)
            .unwrap();
        let member_finished = events
            .iter()
            .position(|event| event.kind == crate::EventKind::MemberFinished)
            .unwrap();
        assert!(member_started < member_finished);
        assert!(member_finished < events.len() - 1);
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (0..events.len() as i64).collect::<Vec<_>>()
        );
    }

    #[test]
    fn handler_validates_before_dispatch() {
        let backend = crate::git::Git2Backend::new();
        let store = EmptyStore;
        let clock = FixedClock::new(TimestampMs(1));
        let mut ids = SequentialIdProvider::new();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &NullSink,
        };
        let mut invalid = request();
        invalid.op = crate::MergeOp::Start;
        let error = handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_1")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeValidationFailed);

        let mut ids = SequentialIdProvider::new();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &NullSink,
        };
        let response =
            handle_merge_with_dependencies(dependencies, Path::new("."), request(), "op_2")
                .unwrap();
        assert_eq!(response.state, crate::MergeOperationState::Idle);
        assert!(!response.open);
    }

    #[test]
    fn every_merge_invocation_closes_its_event_stream() {
        let backend = crate::git::Git2Backend::new();
        let store = EmptyStore;
        let clock = FixedClock::new(TimestampMs(1));
        let sink = CollectingSink::default();

        let mut ids = SequentialIdProvider::new();
        let mut invalid = request();
        invalid.op = crate::MergeOp::Start;
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &sink,
        };
        handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_bad")
            .unwrap_err();

        let mut ids = SequentialIdProvider::new();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &sink,
        };
        handle_merge_with_dependencies(dependencies, Path::new("."), request(), "op_status")
            .unwrap();

        let events = sink.0.lock().unwrap();
        let invocations = events
            .split_inclusive(|event| event.kind == crate::EventKind::OperationFinished)
            .collect::<Vec<_>>();
        assert_eq!(invocations.len(), 2);
        for invocation in invocations {
            assert_eq!(
                invocation.first().map(|event| event.kind),
                Some(crate::EventKind::OperationStarted)
            );
            assert_eq!(
                invocation.last().map(|event| event.kind),
                Some(crate::EventKind::OperationFinished)
            );
            assert_eq!(
                invocation
                    .iter()
                    .map(|event| event.sequence)
                    .collect::<Vec<_>>(),
                (0..invocation.len() as i64).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn public_handler_exposes_the_frozen_service_entry() {
        let backend = crate::git::Git2Backend::new();
        let response = handle_merge(&backend, Path::new("."), request(), "op_1").unwrap();
        assert_eq!(response.state, crate::MergeOperationState::Idle);
    }

    #[test]
    fn workspace_gate_discovers_open_record_and_blocks_only_disallowed_rows() {
        let root = TempDir::new("merge-open-gate");
        let directory = root.path().join(".gwz/merge");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("merge_1.yaml"),
            r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_1
operation_id: op_1
state: awaiting_resolution
source_ref: feature/x
created_at: now
baseline: { lock_sha256: lock, manifest_sha256: manifest }
selected_targets: []
participants: {}
"#,
        )
        .unwrap();
        let workspace = crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        };

        let error = enforce_workspace_open_merge_gate(
            root.path(),
            Some(&workspace),
            crate::operation::OpenMergeCommand::Push,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation);
        assert!(error.message.contains("merge_1"));
        assert!(
            enforce_workspace_open_merge_gate(
                root.path(),
                Some(&workspace),
                crate::operation::OpenMergeCommand::Status,
            )
            .is_ok()
        );
    }

    #[test]
    fn conditional_stage_accepts_a_recorded_conflicted_root_only() {
        let root = TempDir::new("merge-root-stage-gate");
        let directory = root.path().join(".gwz/merge");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("merge_root.yaml"),
            r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_root
operation_id: op_1
state: awaiting_resolution
source_ref: feature/x
created_at: now
baseline: { lock_sha256: lock, manifest_sha256: manifest }
selected_targets: ['@root']
participants:
  '@root':
    path: .
    target_kind: root
    target_branch: main
    before_commit: before
    source_commit: source
    commit_message: merge
    state: conflicted
    expected_merge_head: source
    conflict_paths: [gwz.conf/gwz.yml]
"#,
        )
        .unwrap();

        let root_target = crate::workspace_ops::StageTarget {
            member_path: None,
            pathspecs: vec!["gwz.conf/gwz.yml".to_owned()],
            explicit: true,
        };
        assert!(enforce_open_merge_stage_targets(root.path(), &[root_target]).is_ok());

        let member_target = crate::workspace_ops::StageTarget {
            member_path: Some("repos/app".to_owned()),
            pathspecs: vec!["README.md".to_owned()],
            explicit: true,
        };
        assert_eq!(
            enforce_open_merge_stage_targets(root.path(), &[member_target])
                .unwrap_err()
                .code,
            ErrorCode::OpenOperation
        );
    }

    #[test]
    fn authoritative_guard_retains_mutator_lock_until_drop() {
        let root = TempDir::new("merge-retained-guard");
        let workspace = crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        };
        let guard = acquire_workspace_mutation_guard(
            root.path(),
            Some(&workspace),
            crate::operation::OpenMergeCommand::Push,
        )
        .unwrap();
        assert!(
            crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
                .unwrap()
                .is_none()
        );
        drop(guard);
        assert!(
            crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn dry_run_guard_checks_the_effective_root_without_taking_the_mutator_lock() {
        let root = TempDir::new("merge-dry-run-no-lock");
        let workspace = crate::WorkspaceRef {
            root: Some(root.path().to_string_lossy().into_owned()),
            workspace_id: None,
        };

        let (guard, resolved) = guarded_workspace_root(
            Path::new("/unrelated/cwd"),
            Some(&workspace),
            crate::operation::OpenMergeCommand::MergeStart,
            true,
        )
        .unwrap();

        assert!(guard.is_none());
        assert_eq!(resolved, root.path());
        assert!(
            crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
                .unwrap()
                .is_some()
        );
    }
}
