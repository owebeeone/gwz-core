
use crate::model;



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

pub(crate) fn aggregate_status(report: &ExecutionReport) -> crate::AggregateStatus {
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

pub(crate) fn member_execution_to_protocol(member: &MemberExecution) -> crate::MemberResponse {
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

pub(crate) fn operation_error_to_protocol(error: &OperationError) -> crate::GwzError {
    crate::GwzError {
        code: error.code.into(),
        message: error.message.clone(),
        member_id: None,
        member_path: None,
        detail: None,
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

