
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
    /// F15: identity of the member this error is about, when it is member-specific —
    /// so it survives into the protocol `GwzError` instead of being dropped.
    pub member_id: Option<model::MemberId>,
    pub member_path: Option<String>,
}

impl OperationError {
    pub fn new(code: model::ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            member_id: None,
            member_path: None,
        }
    }
}

pub(crate) fn aggregate_status(report: &ExecutionReport) -> crate::AggregateStatus {
    let any_failed = report
        .members
        .iter()
        .any(|member| member.status == MemberExecutionStatus::Failed);
    let any_applied = report
        .members
        .iter()
        .any(|member| member.status == MemberExecutionStatus::Ok);
    if any_failed && any_applied {
        // F15: some members were applied while others failed (only reachable under a
        // partial policy — atomic rolls back) — that is Partial, not a blanket Failed.
        crate::AggregateStatus::Partial
    } else if any_failed {
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
        // F15: a member's error must carry that member's identity, not lose it at the
        // boundary — fill from the execution when the error didn't set it itself.
        error: member.error.as_ref().map(|error| {
            let mut protocol = operation_error_to_protocol(error);
            if protocol.member_id.is_none() {
                protocol.member_id = member.member_id.as_ref().map(ToString::to_string);
            }
            if protocol.member_path.is_none() {
                protocol.member_path = Some(member.member_path.clone());
            }
            protocol
        }),
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
        member_id: error.member_id.as_ref().map(ToString::to_string),
        member_path: error.member_path.clone(),
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

