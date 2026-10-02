use std::collections::HashMap;
use std::sync::Mutex;

use crate::model;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    RemoteIdentity,
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
    Log,
    CloneLocalWorkspace,
    LocalFamily,
    /// `gwz fetch`: observe every selected repository's remote without
    /// integrating (gwz-cli dev-docs/GwzFetchPlan.md).
    Fetch,
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

pub enum OperationRequest {
    RemoteIdentity(crate::RemoteIdentityRequest),
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
    Fetch(crate::FetchRequest),
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
    Log(crate::LogRequest),
    CloneLocalWorkspace(crate::CloneLocalWorkspaceRequest),
    LocalFamily(crate::LocalFamilyRequest),
}

impl OperationRequest {
    pub fn context(&self, operation_id: impl Into<String>) -> model::ModelResult<OperationContext> {
        let (action, meta) = match self {
            Self::RemoteIdentity(request) => (ActionKind::RemoteIdentity, &request.meta),
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
            Self::Fetch(request) => (ActionKind::Fetch, &request.meta),
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
            Self::Log(request) => (ActionKind::Log, &request.meta),
            Self::CloneLocalWorkspace(request) => (ActionKind::CloneLocalWorkspace, &request.meta),
            Self::LocalFamily(request) => (ActionKind::LocalFamily, &request.meta),
        };
        crate::workspace_ops::validate_structural_selection(
            action.into(),
            meta.selection.as_ref(),
        )?;
        let transport_supported = matches!(
            self,
            Self::Push(_)
                | Self::Fetch(_)
                | Self::PullHead(_)
                | Self::PullSnapshot(_)
                | Self::CloneWorkspace(_)
                | Self::CloneRepoMember(_)
                | Self::InitFromSources(_)
        ) || matches!(self, Self::Materialize(request) if request.target.kind != crate::MaterializeTargetKind::Branch)
            || matches!(self, Self::Tag(request) if matches!(request.op, crate::TagOp::Push | crate::TagOp::Fetch) || (matches!(request.op, crate::TagOp::List | crate::TagOp::Delete) && request.remote.is_some()));
        if crate::git::has_transport_options(meta.transport.as_ref()) && !transport_supported {
            return Err(model::ModelError::new(
                model::ErrorCode::UnsupportedOperation,
                "SSH identity options require a network operation; this operation does not use network credentials",
            ));
        }
        OperationContext::from_meta(operation_id.into(), action, meta)
    }
}

impl OperationContext {
    pub(crate) fn from_meta(
        operation_id: String,
        action: ActionKind,
        meta: &crate::RequestMeta,
    ) -> model::ModelResult<Self> {
        if crate::git::has_transport_options(meta.transport.as_ref())
            && !matches!(
                action,
                ActionKind::Push
                    | ActionKind::Fetch
                    | ActionKind::PullHead
                    | ActionKind::PullSnapshot
                    | ActionKind::CloneWorkspace
                    | ActionKind::CloneRepoMember
                    | ActionKind::InitFromSources
                    | ActionKind::Materialize
                    | ActionKind::Tag
            )
        {
            return Err(model::ModelError::new(
                model::ErrorCode::UnsupportedOperation,
                "SSH identity options require a network operation",
            ));
        }
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

impl<'a> EventEmitter<'a> {
    /// Build the invocation lifecycle owner before any fallible model-context
    /// conversion. This copies protocol envelope fields only; it does not
    /// validate or authorize attribution for Git actions.
    pub fn from_request_meta(
        operation_id: impl Into<String>,
        meta: &crate::RequestMeta,
        sink: &'a dyn EventSink,
        progress_min_interval_ms: i64,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            request_id: meta.request_id.clone(),
            attribution: meta.attribution.clone(),
            sequence: Mutex::new(0),
            progress_min_interval_ms: progress_min_interval_ms.max(0),
            last_progress_ms: Mutex::new(HashMap::new()),
            sink,
        }
    }

    pub fn new(
        context: &OperationContext,
        sink: &'a dyn EventSink,
        progress_min_interval_ms: i64,
    ) -> Self {
        Self {
            operation_id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            attribution: context.attribution.as_ref().map(Into::into),
            sequence: Mutex::new(0),
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
        let target_kind = member_id.as_ref().map(|_| crate::TargetKind::Member);
        let mut event = crate::OperationEvent {
            operation_id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            sequence: 0,
            timestamp_ms: 0,
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
        };
        // Only numbering, the timestamp and delivery share the lock: a sequence
        // number taken here reaches the sink before any later one does, and
        // timestamps are read in sequence order. Build the event outside it.
        let mut next = self
            .sequence
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event.sequence = *next;
        event.timestamp_ms = now_ms().0;
        *next += 1;
        self.sink.deliver(event);
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

/// Build a standard `ResponseEnvelope` from request meta + an action. For **CLI-local** ops
/// (e.g. `gwz forall`) that stamp their own envelope without a gwz-core handler — `gwz-core`
/// itself never executes those, this just mints a consistent envelope.
#[allow(
    clippy::needless_update,
    reason = "gwz_transport_candidate adds fields"
)]
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
            transport: None,
            request_id: context.request_id,
            schema_version: context.schema_version,
            action: action.into(),
            aggregate_status,
            operation_id: Some(context.operation_id),
            message: None,
            attribution: context.attribution.as_ref().map(Into::into),
            ..Default::default()
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
            ActionKind::Fetch => Self::Fetch,
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
            ActionKind::Log => Self::Log,
            ActionKind::CloneLocalWorkspace => Self::CloneLocalWorkspace,
            ActionKind::LocalFamily => Self::LocalFamily,
            ActionKind::RemoteIdentity => Self::RemoteIdentity,
        }
    }
}

/// Dispatch a unified commit-log request through the operation seam.
pub use super::commit_log::{
    CommitLogOutputRegistry, CommitLogReadRequest, CommitLogReadResponse, CommitLogReadState,
};

pub fn handle_log(
    start: &std::path::Path,
    request: crate::LogRequest,
    operation_id: impl Into<String>,
    output_registry: &CommitLogOutputRegistry,
) -> model::ModelResult<crate::LogResponse> {
    super::commit_log::handle_log(start, request, operation_id, output_registry)
}

#[cfg(test)]
mod transport_selection_tests {
    #[test]
    fn structural_requests_refuse_unused_selection_but_accept_empty_envelopes() {
        for selected in [false, true] {
            let meta = crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: if selected {
                        vec!["@root".into()]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                }),
                ..Default::default()
            };
            let requests = [
                super::OperationRequest::CreateWorkspace(crate::CreateWorkspaceRequest {
                    meta: meta.clone(),
                    ..Default::default()
                }),
                super::OperationRequest::InitFromSources(crate::InitFromSourcesRequest {
                    meta: meta.clone(),
                    ..Default::default()
                }),
                super::OperationRequest::AddExistingRepo(crate::AddExistingRepoRequest {
                    meta: meta.clone(),
                    ..Default::default()
                }),
                super::OperationRequest::CreateRepo(crate::CreateRepoRequest {
                    meta: meta.clone(),
                    ..Default::default()
                }),
                super::OperationRequest::CloneRepoMember(crate::CloneRepoMemberRequest {
                    meta: meta.clone(),
                    ..Default::default()
                }),
                super::OperationRequest::ListSnapshots(crate::ListSnapshotsRequest { meta }),
            ];
            for request in requests {
                let result = request.context("unused-selection");
                if selected {
                    assert_eq!(
                        result.unwrap_err().code,
                        crate::model::ErrorCode::InvalidRequest
                    );
                } else {
                    assert!(result.is_ok());
                }
            }
        }
    }

    #[test]
    fn local_requests_refuse_nonempty_transport_options() {
        let request = super::OperationRequest::Status(crate::StatusRequest {
            meta: crate::RequestMeta {
                transport: Some(crate::TransportOptions {
                    default_identity: Some("unused-key".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(request.context("local-auth-refusal").is_err());
        let meta = crate::RequestMeta {
            transport: Some(crate::TransportOptions {
                default_identity: Some("unused-key".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        for request in [
            super::OperationRequest::Tag(crate::TagRequest {
                meta: meta.clone(),
                op: crate::TagOp::Create,
                remote: Some("origin".into()),
                ..Default::default()
            }),
            super::OperationRequest::Materialize(crate::MaterializeRequest {
                meta: meta.clone(),
                target: crate::MaterializeTarget {
                    kind: crate::MaterializeTargetKind::Branch,
                    name: Some("local".into()),
                    commit: None,
                },
            }),
        ] {
            assert!(request.context("local-suboperation").is_err());
        }
        assert!(
            super::OperationContext::from_meta("forall".into(), super::ActionKind::Forall, &meta)
                .is_err()
        );
    }
}
