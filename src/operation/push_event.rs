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
    Materialize(crate::MaterializeRequest),
    Status(crate::StatusRequest),
    Snapshot(crate::SnapshotRequest),
    Tag(crate::TagRequest),
    PullHead(crate::PullHeadRequest),
    PullSnapshot(crate::PullSnapshotRequest),
    Push(crate::PushRequest),
    Capture(crate::CaptureRequest),
}

impl OperationRequest {
    pub fn context(&self, operation_id: impl Into<String>) -> model::ModelResult<OperationContext> {
        let (action, meta) = match self {
            Self::CreateWorkspace(request) => (ActionKind::CreateWorkspace, &request.meta),
            Self::InitFromSources(request) => (ActionKind::InitFromSources, &request.meta),
            Self::AddExistingRepo(request) => (ActionKind::AddExistingRepo, &request.meta),
            Self::CreateRepo(request) => (ActionKind::CreateRepo, &request.meta),
            Self::Materialize(request) => (ActionKind::Materialize, &request.meta),
            Self::Status(request) => (ActionKind::Status, &request.meta),
            Self::Snapshot(request) => (ActionKind::Snapshot, &request.meta),
            Self::Tag(request) => (ActionKind::Tag, &request.meta),
            Self::PullHead(request) => (ActionKind::PullHead, &request.meta),
            Self::PullSnapshot(request) => (ActionKind::PullSnapshot, &request.meta),
            Self::Push(request) => (ActionKind::Push, &request.meta),
            Self::Capture(request) => (ActionKind::Capture, &request.meta),
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
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
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
        });
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
        progress: None,
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
        lock_match: None,
    }
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
        }
    }
}

