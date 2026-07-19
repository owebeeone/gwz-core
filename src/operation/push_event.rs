use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::model;
use crate::runtime::clock::TimestampMs;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    CreateWorkspace,
    InitFromSources,
    AddExistingRepo,
    CreateRepo,
    Materialize,
    Status,
    Snapshot,
    Tag,
    PullHead,
    PullSnapshot,
    Push,
    Capture,
    Commit,
    Stage,
    Ls,
    Forall,
    RepoSync,
    Stash,
    Branch,
    CloneWorkspace,
    ListSnapshots,
    CloneRepoMember,
    DetachRepoMember,
    AttachRepoMember,
    Merge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlannedAction {
    Noop,
    Clone,
    Fetch,
    FastForward,
    Checkout,
    InitRepo,
    AddManifestMember,
    WriteManifest,
    WriteLock,
    WriteSnapshot,
    WriteTag,
    Push,
    Merge,
    Rebase,
    Reset,
    DetachMember,
    AttachMember,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub operation_id: String,
    pub request_id: String,
    pub schema_version: String,
    pub action: ActionKind,
    pub dry_run: bool,
    pub attribution: Option<model::OperationAttribution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationPlan {
    pub operation_id: String,
    pub action: ActionKind,
    pub dry_run: bool,
    pub members: Vec<MemberPlan>,
}

impl OperationPlan {
    pub fn requires_mutation(&self) -> bool {
        self.members.iter().any(|member| member.requires_mutation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberPlan {
    pub member_id: Option<model::MemberId>,
    pub member_path: String,
    pub source_kind: model::SourceKind,
    pub action: PlannedAction,
    pub requires_mutation: bool,
    pub message: Option<String>,
}

pub enum OperationRequest {
    CreateWorkspace(crate::CreateWorkspaceRequest),
    InitFromSources(crate::InitFromSourcesRequest),
    AddExistingRepo(crate::AddExistingRepoRequest),
    CreateRepo(crate::CreateRepoRequest),
    RepoSync(crate::RepoSyncRequest),
    Materialize(crate::MaterializeRequest),
    Status(crate::StatusRequest),
    Snapshot(crate::SnapshotRequest),
    ListSnapshots(crate::ListSnapshotsRequest),
    Tag(crate::TagRequest),
    PullHead(crate::PullHeadRequest),
    PullSnapshot(crate::PullSnapshotRequest),
    Push(crate::PushRequest),
    Capture(crate::CaptureRequest),
    Commit(crate::CommitRequest),
    Stage(crate::StageRequest),
    Ls(crate::LsRequest),
    Stash(crate::StashRequest),
    Branch(crate::BranchRequest),
    CloneWorkspace(crate::CloneWorkspaceRequest),
    CloneRepoMember(crate::CloneRepoMemberRequest),
    DetachRepoMember(crate::DetachRepoMemberRequest),
    AttachRepoMember(crate::AttachRepoMemberRequest),
    Merge(crate::MergeRequest),
}

impl OperationRequest {
    pub fn context(&self, operation_id: impl Into<String>) -> model::ModelResult<OperationContext> {
        let (action, meta) = match self {
            Self::CreateWorkspace(request) => (ActionKind::CreateWorkspace, &request.meta),
            Self::InitFromSources(request) => (ActionKind::InitFromSources, &request.meta),
            Self::AddExistingRepo(request) => (ActionKind::AddExistingRepo, &request.meta),
            Self::CreateRepo(request) => (ActionKind::CreateRepo, &request.meta),
            Self::RepoSync(request) => (ActionKind::RepoSync, &request.meta),
            Self::Materialize(request) => (ActionKind::Materialize, &request.meta),
            Self::Status(request) => (ActionKind::Status, &request.meta),
            Self::Snapshot(request) => (ActionKind::Snapshot, &request.meta),
            Self::ListSnapshots(request) => (ActionKind::ListSnapshots, &request.meta),
            Self::Tag(request) => (ActionKind::Tag, &request.meta),
            Self::PullHead(request) => (ActionKind::PullHead, &request.meta),
            Self::PullSnapshot(request) => (ActionKind::PullSnapshot, &request.meta),
            Self::Push(request) => (ActionKind::Push, &request.meta),
            Self::Capture(request) => (ActionKind::Capture, &request.meta),
            Self::Commit(request) => (ActionKind::Commit, &request.meta),
            Self::Stage(request) => (ActionKind::Stage, &request.meta),
            Self::Ls(request) => (ActionKind::Ls, &request.meta),
            Self::Stash(request) => (ActionKind::Stash, &request.meta),
            Self::Branch(request) => (ActionKind::Branch, &request.meta),
            Self::CloneWorkspace(request) => (ActionKind::CloneWorkspace, &request.meta),
            Self::CloneRepoMember(request) => (ActionKind::CloneRepoMember, &request.meta),
            Self::DetachRepoMember(request) => (ActionKind::DetachRepoMember, &request.meta),
            Self::AttachRepoMember(request) => (ActionKind::AttachRepoMember, &request.meta),
            Self::Merge(request) => (ActionKind::Merge, &request.meta),
        };
        OperationContext::from_meta(operation_id.into(), action, meta)
    }
}

impl OperationContext {
    pub(crate) fn from_meta(
        operation_id: String,
        action: ActionKind,
        meta: &crate::RequestMeta,
    ) -> model::ModelResult<Self> {
        let attribution = meta
            .attribution
            .as_ref()
            .map(attribution_from_protocol)
            .transpose()?;
        Ok(Self {
            operation_id,
            request_id: meta.request_id.clone(),
            schema_version: meta.schema_version.clone(),
            action,
            dry_run: meta.dry_run.unwrap_or(false),
            attribution,
        })
    }
}

pub struct ResponseBuilder;

impl ResponseBuilder {
    pub fn accepted(context: &OperationContext, members: &[MemberPlan]) -> crate::ResponseEnvelope {
        crate::ResponseEnvelope {
            meta: crate::ResponseMeta {
                request_id: context.request_id.clone(),
                schema_version: context.schema_version.clone(),
                action: context.action.into(),
                aggregate_status: crate::AggregateStatus::Accepted,
                operation_id: Some(context.operation_id.clone()),
                message: None,
                attribution: context.attribution.as_ref().map(Into::into),
            },
            members: members.iter().map(member_plan_to_protocol).collect(),
            errors: Vec::new(),
        }
    }

    pub fn result(
        context: &OperationContext,
        report: &ExecutionReport,
        started_at_ms: TimestampMs,
        finished_at_ms: TimestampMs,
    ) -> crate::OperationResult {
        crate::OperationResult {
            operation_id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            action: context.action.into(),
            aggregate_status: aggregate_status(report),
            started_at_ms: started_at_ms.0,
            finished_at_ms: finished_at_ms.0,
            members: report
                .members
                .iter()
                .map(member_execution_to_protocol)
                .collect(),
            errors: report
                .errors
                .iter()
                .map(operation_error_to_protocol)
                .collect(),
            attribution: context.attribution.as_ref().map(Into::into),
        }
    }
}

impl OperationRuntime {
    pub fn new(event_capacity: usize) -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            event_capacity: event_capacity.max(2),
        }
    }

    pub fn submit<F>(
        &self,
        context: OperationContext,
        handler: F,
    ) -> model::ModelResult<crate::ResponseEnvelope>
    where
        F: FnOnce(OperationContext, RuntimeEventSink) -> ExecutionReport + Send + 'static,
    {
        let record = Arc::new(OperationRecord::new(self.event_capacity));
        self.records
            .lock()
            .expect("operation registry poisoned")
            .insert(context.operation_id.clone(), Arc::clone(&record));

        let accepted = ResponseBuilder::accepted(&context, &[]);
        thread::spawn(move || {
            let started_at_ms = now_ms();
            let sink = RuntimeEventSink {
                context: context.clone(),
                record: Arc::clone(&record),
            };
            sink.emit(
                crate::EventKind::OperationStarted,
                crate::Severity::Info,
                None,
                None,
                Some("operation started".to_owned()),
            );
            let report = handler(context.clone(), sink.clone());
            sink.emit(
                crate::EventKind::OperationFinished,
                crate::Severity::Info,
                None,
                None,
                Some("operation finished".to_owned()),
            );
            let result = ResponseBuilder::result(&context, &report, started_at_ms, now_ms());
            record.complete(result);
        });
        Ok(accepted)
    }

    pub fn subscribe(&self, operation_id: &str) -> model::ModelResult<EventSubscription> {
        Ok(EventSubscription {
            record: self.record(operation_id)?,
            next_sequence: 0,
        })
    }

    pub fn try_result(
        &self,
        operation_id: &str,
    ) -> model::ModelResult<Option<crate::OperationResult>> {
        let record = self.record(operation_id)?;
        Ok(record
            .state
            .lock()
            .expect("operation record poisoned")
            .result
            .clone())
    }

    pub fn wait(&self, operation_id: &str) -> model::ModelResult<crate::OperationResult> {
        let record = self.record(operation_id)?;
        let mut state = record.state.lock().expect("operation record poisoned");
        loop {
            if let Some(result) = &state.result {
                return Ok(result.clone());
            }
            state = record
                .complete
                .wait(state)
                .expect("operation record poisoned");
        }
    }

    pub(crate) fn record(&self, operation_id: &str) -> model::ModelResult<Arc<OperationRecord>> {
        self.records
            .lock()
            .expect("operation registry poisoned")
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                model::ModelError::new(
                    model::ErrorCode::OperationNotFound,
                    format!("operation {operation_id} not found"),
                )
            })
    }
}

impl<'a> EventEmitter<'a> {
    pub fn new(
        context: &OperationContext,
        sink: &'a dyn EventSink,
        progress_min_interval_ms: i64,
    ) -> Self {
        Self {
            operation_id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            attribution: context.attribution.as_ref().map(Into::into),
            sequence: AtomicI64::new(0),
            progress_min_interval_ms: progress_min_interval_ms.max(0),
            last_progress_ms: Mutex::new(HashMap::new()),
            sink,
        }
    }

    pub(crate) fn emit(
        &self,
        kind: crate::EventKind,
        severity: crate::Severity,
        member_id: Option<String>,
        member_path: Option<String>,
        message: Option<String>,
        progress: Option<crate::GitTransferProgress>,
    ) {
        self.emit_with_merge_state(
            kind,
            severity,
            member_id,
            member_path,
            message,
            progress,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)] // Mirrors the protocol event envelope fields.
    fn emit_with_merge_state(
        &self,
        kind: crate::EventKind,
        severity: crate::Severity,
        member_id: Option<String>,
        member_path: Option<String>,
        message: Option<String>,
        progress: Option<crate::GitTransferProgress>,
        merge_state: Option<crate::MergeOperationState>,
    ) {
        self.emit_with_payload(
            kind,
            severity,
            member_id,
            member_path,
            message,
            progress,
            merge_state,
            None,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)] // Mirrors the protocol event envelope fields.
    fn emit_with_payload(
        &self,
        kind: crate::EventKind,
        severity: crate::Severity,
        member_id: Option<String>,
        member_path: Option<String>,
        message: Option<String>,
        progress: Option<crate::GitTransferProgress>,
        merge_state: Option<crate::MergeOperationState>,
        merge_member: Option<crate::MergeRepoSummary>,
        artifact_path: Option<String>,
    ) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let target_kind = member_id.as_ref().map(|_| crate::TargetKind::Member);
        self.sink.deliver(crate::OperationEvent {
            operation_id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            sequence,
            timestamp_ms: now_ms().0,
            kind,
            severity,
            member_id,
            member_path,
            message,
            member: None,
            error: None,
            attribution: self.attribution.clone(),
            progress,
            target_kind,
            merge_state,
            merge_member,
            artifact_path,
        });
    }

    pub fn operation_state_changed(&self, state: crate::MergeOperationState) {
        self.emit_with_merge_state(
            crate::EventKind::OperationStateChanged,
            crate::Severity::Info,
            None,
            None,
            Some(format!("merge operation state changed to {state:?}")),
            None,
            Some(state),
        );
    }

    pub fn operation_started(&self) {
        self.emit(
            crate::EventKind::OperationStarted,
            crate::Severity::Info,
            None,
            None,
            Some("operation started".to_owned()),
            None,
        );
    }

    pub fn operation_finished(&self) {
        self.emit(
            crate::EventKind::OperationFinished,
            crate::Severity::Info,
            None,
            None,
            Some("operation finished".to_owned()),
            None,
        );
    }

    pub fn member_started(&self, member_id: &str, member_path: &str) {
        self.emit(
            crate::EventKind::MemberStarted,
            crate::Severity::Info,
            Some(member_id.to_owned()),
            Some(member_path.to_owned()),
            None,
            None,
        );
    }

    pub fn member_progress(
        &self,
        member_id: &str,
        member_path: &str,
        progress: crate::GitTransferProgress,
    ) {
        if !self.should_emit_progress(member_path) {
            return;
        }
        self.emit(
            crate::EventKind::MemberProgress,
            crate::Severity::Info,
            Some(member_id.to_owned()),
            Some(member_path.to_owned()),
            None,
            Some(progress),
        );
    }

    /// Rate-limits per-member progress to one event per
    /// `progress_min_interval_ms`. The first update for a member always passes,
    /// so a fast member still reports at least once.
    pub(crate) fn should_emit_progress(&self, member_path: &str) -> bool {
        if self.progress_min_interval_ms == 0 {
            return true;
        }
        let now = now_ms().0;
        let mut last = self.last_progress_ms.lock().expect("progress map poisoned");
        match last.get(member_path) {
            Some(&prev) if now - prev < self.progress_min_interval_ms => false,
            _ => {
                last.insert(member_path.to_owned(), now);
                true
            }
        }
    }

    pub fn member_finished(&self, member_id: &str, member_path: &str) {
        self.emit(
            crate::EventKind::MemberFinished,
            crate::Severity::Info,
            Some(member_id.to_owned()),
            Some(member_path.to_owned()),
            None,
            None,
        );
    }

    pub fn merge_member_finished(&self, member: crate::MergeRepoSummary) {
        self.emit_with_payload(
            crate::EventKind::MemberFinished,
            crate::Severity::Info,
            Some(member.target_id.clone()),
            Some(member.path.clone()),
            None,
            None,
            None,
            Some(member),
            None,
        );
    }

    pub fn artifact_written(&self, artifact_path: impl Into<String>) {
        let artifact_path = artifact_path.into();
        self.emit_with_payload(
            crate::EventKind::ArtifactWritten,
            crate::Severity::Info,
            None,
            None,
            Some(format!("artifact written: {artifact_path}")),
            None,
            None,
            None,
            Some(artifact_path),
        );
    }
}

#[derive(Clone)]
pub struct RuntimeEventSink {
    pub(crate) context: OperationContext,
    pub(crate) record: Arc<OperationRecord>,
}

pub(crate) fn push_event(state: &mut OperationState, context: &OperationContext) {
    if state.events.len() < state.event_capacity {
        return;
    }

    state.events.clear();
    let reset = crate::OperationEvent {
        operation_id: context.operation_id.clone(),
        request_id: context.request_id.clone(),
        sequence: state.next_sequence,
        timestamp_ms: now_ms().0,
        kind: crate::EventKind::Reset,
        severity: crate::Severity::Warn,
        member_id: None,
        member_path: None,
        message: Some("event buffer overflow; history incomplete".to_owned()),
        member: None,
        error: None,
        attribution: context.attribution.as_ref().map(Into::into),
        target_kind: None,
        progress: None,
        merge_state: None,
        merge_member: None,
        artifact_path: None,
    };
    state.next_sequence += 1;
    state.events.push_back(reset);
}

pub(crate) fn member_plan_to_protocol(member: &MemberPlan) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member
            .member_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        member_path: member.member_path.clone(),
        source_kind: member.source_kind.into(),
        status: crate::MemberStatus::Planned,
        error: None,
        planned: Some(crate::PlannedChange {
            action: member.action.into(),
            from_ref: None,
            to_ref: None,
            message: member.message.clone(),
        }),
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: None,
    }
}

/// Build a standard `ResponseEnvelope` from request meta + an action. For **CLI-local** ops
/// (e.g. `gwz forall`) that stamp their own envelope without a gwz-core handler — `gwz-core`
/// itself never executes those, this just mints a consistent envelope.
pub fn response_envelope_for(
    meta: &crate::RequestMeta,
    action: ActionKind,
    operation_id: impl Into<String>,
    aggregate_status: crate::AggregateStatus,
    errors: Vec<crate::GwzError>,
) -> model::ModelResult<crate::ResponseEnvelope> {
    let context = OperationContext::from_meta(operation_id.into(), action, meta)?;
    Ok(crate::ResponseEnvelope {
        meta: crate::ResponseMeta {
            request_id: context.request_id,
            schema_version: context.schema_version,
            action: action.into(),
            aggregate_status,
            operation_id: Some(context.operation_id),
            message: None,
            attribution: context.attribution.as_ref().map(Into::into),
        },
        members: Vec::new(),
        errors,
    })
}

impl From<ActionKind> for crate::ActionKind {
    fn from(value: ActionKind) -> Self {
        match value {
            ActionKind::CreateWorkspace => Self::CreateWorkspace,
            ActionKind::InitFromSources => Self::InitFromSources,
            ActionKind::AddExistingRepo => Self::AddExistingRepo,
            ActionKind::CreateRepo => Self::CreateRepo,
            ActionKind::Materialize => Self::Materialize,
            ActionKind::Status => Self::Status,
            ActionKind::Snapshot => Self::Snapshot,
            ActionKind::Tag => Self::Tag,
            ActionKind::PullHead => Self::PullHead,
            ActionKind::PullSnapshot => Self::PullSnapshot,
            ActionKind::Push => Self::Push,
            ActionKind::Capture => Self::Capture,
            ActionKind::Commit => Self::Commit,
            ActionKind::Stage => Self::Stage,
            ActionKind::Ls => Self::Ls,
            ActionKind::Forall => Self::Forall,
            ActionKind::RepoSync => Self::RepoSync,
            ActionKind::Stash => Self::Stash,
            ActionKind::Branch => Self::Branch,
            ActionKind::CloneWorkspace => Self::CloneWorkspace,
            ActionKind::ListSnapshots => Self::ListSnapshots,
            ActionKind::CloneRepoMember => Self::CloneRepoMember,
            ActionKind::DetachRepoMember => Self::DetachRepoMember,
            ActionKind::AttachRepoMember => Self::AttachRepoMember,
            ActionKind::Merge => Self::Merge,
        }
    }
}

impl From<PlannedAction> for crate::PlannedAction {
    fn from(value: PlannedAction) -> Self {
        match value {
            PlannedAction::Noop => Self::Noop,
            PlannedAction::Clone => Self::Clone,
            PlannedAction::Fetch => Self::Fetch,
            PlannedAction::FastForward => Self::FastForward,
            PlannedAction::Checkout => Self::Checkout,
            PlannedAction::InitRepo => Self::InitRepo,
            PlannedAction::AddManifestMember => Self::AddManifestMember,
            PlannedAction::WriteManifest => Self::WriteManifest,
            PlannedAction::WriteLock => Self::WriteLock,
            PlannedAction::WriteSnapshot => Self::WriteSnapshot,
            PlannedAction::WriteTag => Self::WriteTag,
            PlannedAction::Push => Self::Push,
            PlannedAction::Merge => Self::Merge,
            PlannedAction::Rebase => Self::Rebase,
            PlannedAction::Reset => Self::Reset,
            PlannedAction::DetachMember => Self::DetachMember,
            PlannedAction::AttachMember => Self::AttachMember,
        }
    }
}
