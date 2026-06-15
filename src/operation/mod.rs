use crate::model;
use crate::runtime::clock::TimestampMs;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionReport {
    pub members: Vec<MemberExecution>,
    pub errors: Vec<OperationError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberExecution {
    pub member_id: Option<model::MemberId>,
    pub member_path: String,
    pub source_kind: model::SourceKind,
    pub status: MemberExecutionStatus,
    pub error: Option<OperationError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberExecutionStatus {
    Ok,
    Noop,
    Skipped,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationError {
    pub code: model::ErrorCode,
    pub message: String,
}

impl OperationError {
    pub fn new(code: model::ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
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
        };
        OperationContext::from_meta(operation_id.into(), action, meta)
    }
}

impl OperationContext {
    fn from_meta(
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

fn aggregate_status(report: &ExecutionReport) -> crate::AggregateStatus {
    if report
        .members
        .iter()
        .any(|member| member.status == MemberExecutionStatus::Failed)
    {
        crate::AggregateStatus::Failed
    } else if !report.errors.is_empty()
        || report
            .members
            .iter()
            .any(|member| member.status == MemberExecutionStatus::Rejected)
    {
        crate::AggregateStatus::Rejected
    } else if report
        .members
        .iter()
        .all(|member| member.status == MemberExecutionStatus::Noop)
    {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Ok
    }
}

fn member_plan_to_protocol(member: &MemberPlan) -> crate::MemberResponse {
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

fn member_execution_to_protocol(member: &MemberExecution) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member
            .member_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        member_path: member.member_path.clone(),
        source_kind: member.source_kind.into(),
        status: member.status.into(),
        error: member.error.as_ref().map(operation_error_to_protocol),
        planned: None,
        state: None,
        git_status: None,
        lock_match: None,
    }
}

fn operation_error_to_protocol(error: &OperationError) -> crate::GwsError {
    crate::GwsError {
        code: error.code.into(),
        message: error.message.clone(),
        member_id: None,
        member_path: None,
        detail: None,
    }
}

fn attribution_from_protocol(
    value: &crate::OperationAttribution,
) -> model::ModelResult<model::OperationAttribution> {
    let attribution = model::OperationAttribution {
        actor: value.actor.as_ref().map(|actor| model::OperationActor {
            actor_id: actor.actor_id.clone(),
            display_name: actor.display_name.clone(),
            email: actor.email.clone(),
            authority: actor.authority.clone(),
        }),
        git_author: value.git_author.as_ref().map(git_identity_from_protocol),
        git_committer: value.git_committer.as_ref().map(git_identity_from_protocol),
        credential_ref: value.credential_ref.clone(),
    };
    attribution.validate()?;
    Ok(attribution)
}

fn git_identity_from_protocol(value: &crate::GitObjectIdentity) -> model::GitObjectIdentity {
    model::GitObjectIdentity {
        name: value.name.clone(),
        email: value.email.clone(),
        time_ms: value.time_ms.map(TimestampMs),
        timezone_offset_minutes: value.timezone_offset_minutes,
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

impl From<MemberExecutionStatus> for crate::MemberStatus {
    fn from(value: MemberExecutionStatus) -> Self {
        match value {
            MemberExecutionStatus::Ok => Self::Ok,
            MemberExecutionStatus::Noop => Self::Noop,
            MemberExecutionStatus::Skipped => Self::Skipped,
            MemberExecutionStatus::Rejected => Self::Rejected,
            MemberExecutionStatus::Failed => Self::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        GitObjectIdentity, MemberId, OperationActor, OperationAttribution, SourceKind,
    };
    use crate::runtime::clock::TimestampMs;

    use super::*;

    #[test]
    fn dry_run_plan_reports_member_plans_without_execution() {
        let context = sample_context(true);
        let plan = OperationPlan {
            operation_id: context.operation_id.clone(),
            action: ActionKind::Status,
            dry_run: context.dry_run,
            members: vec![MemberPlan {
                member_id: Some(MemberId::parse_str("mem_01").unwrap()),
                member_path: "repos/example".to_owned(),
                source_kind: SourceKind::Git,
                action: PlannedAction::Noop,
                requires_mutation: false,
                message: Some("status only".to_owned()),
            }],
        };

        assert!(plan.dry_run);
        assert!(!plan.requires_mutation());
        assert_eq!(plan.members[0].action, PlannedAction::Noop);
    }

    #[test]
    fn accepted_response_carries_operation_id_and_attribution() {
        let context = sample_context(false);
        let response = ResponseBuilder::accepted(&context, &[]);

        assert_eq!(response.meta.operation_id.as_deref(), Some("op_0001"));
        assert_eq!(response.meta.request_id, "req-1");
        assert_eq!(response.meta.action, crate::ActionKind::Status);
        assert_eq!(
            response.meta.aggregate_status,
            crate::AggregateStatus::Accepted
        );
        assert_eq!(
            response
                .meta
                .attribution
                .as_ref()
                .and_then(|value| value.actor.as_ref())
                .map(|actor| actor.actor_id.as_str()),
            Some("agent://local/session")
        );
    }

    #[test]
    fn execution_report_assembles_final_operation_result() {
        let context = sample_context(false);
        let report = ExecutionReport {
            members: vec![MemberExecution {
                member_id: Some(MemberId::parse_str("mem_01").unwrap()),
                member_path: "repos/example".to_owned(),
                source_kind: SourceKind::Git,
                status: MemberExecutionStatus::Rejected,
                error: Some(OperationError::new(
                    crate::model::ErrorCode::DivergedMember,
                    "member diverged",
                )),
            }],
            errors: vec![OperationError::new(
                crate::model::ErrorCode::DivergedMember,
                "member diverged",
            )],
        };

        let result = ResponseBuilder::result(&context, &report, TimestampMs(10), TimestampMs(20));

        assert_eq!(result.operation_id, "op_0001");
        assert_eq!(result.aggregate_status, crate::AggregateStatus::Rejected);
        assert_eq!(result.members[0].status, crate::MemberStatus::Rejected);
        assert_eq!(result.errors[0].code, crate::GwsErrorCode::DivergedMember);
        assert_eq!(
            result
                .attribution
                .as_ref()
                .and_then(|value| value.git_committer.as_ref())
                .map(|identity| identity.email.as_str()),
            Some("bot@example.invalid")
        );
    }

    #[test]
    fn dispatch_context_preserves_status_request_meta() {
        let request = crate::StatusRequest {
            meta: crate::RequestMeta {
                request_id: "req-1".to_owned(),
                schema_version: "gws.v0".to_owned(),
                dry_run: Some(true),
                attribution: Some(crate::OperationAttribution::from(&sample_attribution())),
                ..crate::RequestMeta::default()
            },
        };

        let context = OperationRequest::Status(request)
            .context("op_0001")
            .expect("status context");

        assert_eq!(context.action, ActionKind::Status);
        assert_eq!(context.operation_id, "op_0001");
        assert_eq!(context.request_id, "req-1");
        assert!(context.dry_run);
        assert_eq!(
            context
                .attribution
                .as_ref()
                .unwrap()
                .actor
                .as_ref()
                .unwrap()
                .actor_id,
            "agent://local/session"
        );
    }

    fn sample_context(dry_run: bool) -> OperationContext {
        OperationContext {
            operation_id: "op_0001".to_owned(),
            request_id: "req-1".to_owned(),
            schema_version: "gws.v0".to_owned(),
            action: ActionKind::Status,
            dry_run,
            attribution: Some(sample_attribution()),
        }
    }

    fn sample_attribution() -> OperationAttribution {
        OperationAttribution {
            actor: Some(OperationActor::new("agent://local/session")),
            git_author: Some(GitObjectIdentity::new("Agent", "agent@example.invalid")),
            git_committer: Some(GitObjectIdentity::new("Bot", "bot@example.invalid")),
            credential_ref: Some("cred:test".to_owned()),
        }
    }
}
