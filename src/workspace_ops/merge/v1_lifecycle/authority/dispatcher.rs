use serde::Serialize;

use super::super::super::model::v1::{
    MergeOperationRecordV1, PendingPreservationActionV1, PendingRollbackActionV1,
};
use super::super::super::{OperationState, ParticipantState, PendingMergeAction};
use super::super::checked::StoredV1Record;
use super::super::transition::{OperationTransition, V1Transition};
use super::binding::BoundValue;
use crate::model::{ErrorCode, ModelError, ModelResult};

mod invocation;

pub(in crate::workspace_ops::merge::v1_lifecycle) use invocation::V1Invocation;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum V1LifecycleRequest {
    ResumeStart,
    Continue,
    Abort,
    Preserve,
    Status,
    Archive,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ObservationKind {
    ParticipantPreparation { member_id: String },
    ParticipantAction { member_id: String },
    ParticipantsComplete,
    Acceptance,
    Publication,
    PreservationEntry,
    PreservationCursor,
    RollbackEntry,
    RollbackCursor,
    Recovery,
    Archive,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ObservationKey {
    pub(super) request: V1LifecycleRequest,
    pub(super) kind: Box<ObservationKind>,
    pub(super) owner: String,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) struct BoundObservationRequest(
    pub(super) Box<BoundValue<ObservationKey>>,
);

impl BoundObservationRequest {
    fn issue(
        current: &StoredV1Record,
        request: V1LifecycleRequest,
        kind: ObservationKind,
    ) -> ModelResult<Self> {
        let owner = observation_owner(&kind);
        let key = ObservationKey {
            request,
            kind: Box::new(kind),
            owner: owner.clone(),
        };
        Ok(Self(Box::new(BoundValue::new(
            current,
            &owner,
            "observe",
            "requested",
            key,
        )?)))
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn for_test(
        current: &StoredV1Record,
        request: V1LifecycleRequest,
        kind: ObservationKind,
    ) -> ModelResult<Self> {
        Self::issue(current, request, kind)
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn kind(&self) -> &ObservationKind {
        self.0.value.kind.as_ref()
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn lifecycle(&self) -> V1LifecycleRequest {
        self.0.value.request
    }

    pub(super) fn matches(&self, current: &StoredV1Record, request: V1LifecycleRequest) -> bool {
        self.0.value.request == request
            && self
                .0
                .matches(current, &self.0.value.owner, "observe", "requested")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum PublicationPhysicalAction {
    EvidenceCommit,
    WriteMarker,
    WriteLock,
    WriteBoundary,
    StageIndex,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum PhysicalActionKind {
    Participant {
        member_id: String,
        action: Box<PendingMergeAction>,
    },
    Publication(PublicationPhysicalAction),
    Preservation(PendingPreservationActionV1),
    Rollback(PendingRollbackActionV1),
    Archive,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct PhysicalActionKey {
    pub(super) observation: ObservationKey,
    pub(super) action: PhysicalActionKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ExecutionDiagnostic {
    Success,
    Failed {
        code: ErrorCode,
        message: String,
        detail: Option<String>,
    },
}

pub(in crate::workspace_ops::merge::v1_lifecycle) struct BoundPhysicalAction(
    pub(super) BoundValue<PhysicalActionKey>,
);
pub(in crate::workspace_ops::merge::v1_lifecycle) struct BoundExecutionAttempt(
    pub(super) BoundValue<(PhysicalActionKey, ExecutionDiagnostic)>,
);

impl BoundPhysicalAction {
    #[allow(
        dead_code,
        reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
    )]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn kind(&self) -> &PhysicalActionKind {
        &self.0.value.action
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn authorize(
        &self,
        current: &StoredV1Record,
    ) -> ModelResult<&PhysicalActionKind> {
        self.0
            .matches(
                current,
                &self.0.value.observation.owner,
                "execute",
                "authorized",
            )
            .then_some(&self.0.value.action)
            .ok_or_else(|| dispatch_error("physical action authority is stale"))
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn record_attempt(
        self,
        current: &StoredV1Record,
        diagnostic: ExecutionDiagnostic,
    ) -> ModelResult<BoundExecutionAttempt> {
        if !self.0.matches(
            current,
            &self.0.value.observation.owner,
            "execute",
            "authorized",
        ) {
            return Err(dispatch_error("physical action authority is stale"));
        }
        let key = self.0.value;
        let owner = key.observation.owner.clone();
        Ok(BoundExecutionAttempt(BoundValue::new(
            current,
            &owner,
            "execute",
            "attempted",
            (key, diagnostic),
        )?))
    }
}

impl BoundExecutionAttempt {
    pub(super) fn matches(
        &self,
        current: &StoredV1Record,
        request: &ObservationKey,
        physical: Option<&PhysicalActionKind>,
    ) -> bool {
        self.0.value.0.observation == *request
            && physical.is_none_or(|action| self.0.value.0.action == *action)
            && self
                .0
                .matches(current, &request.owner, "execute", "attempted")
    }

    pub(super) fn action(&self) -> &PhysicalActionKind {
        &self.0.value.0.action
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum V1ResponseDisposition {
    Status,
    Stopped(OperationState),
    Terminal(OperationState),
    ArchiveReady,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) enum V1NextAction {
    Observe(BoundObservationRequest),
    Apply(V1Transition),
    Respond(V1ResponseDisposition),
    Reject(ModelError),
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn next_action(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
) -> ModelResult<V1NextAction> {
    let record = current.record();
    if request == V1LifecycleRequest::Status {
        return Ok(V1NextAction::Respond(V1ResponseDisposition::Status));
    }
    if matches!(
        record.state,
        OperationState::Completed | OperationState::Aborted
    ) {
        return Ok(if request == V1LifecycleRequest::Archive {
            observe(current, request, ObservationKind::Archive)?
        } else {
            V1NextAction::Respond(V1ResponseDisposition::Terminal(record.state))
        });
    }
    if record.state == OperationState::RecoveryRequired {
        return observe(current, request, ObservationKind::Recovery);
    }
    if record.pending_preservation.is_some() {
        return observe(current, request, ObservationKind::PreservationCursor);
    }
    if record.pending_rollback.is_some() {
        return observe(current, request, ObservationKind::RollbackCursor);
    }
    if let Some(member_id) = pending_participant(record) {
        return observe(
            current,
            request,
            ObservationKind::ParticipantAction { member_id },
        );
    }
    match record.state {
        OperationState::Executing => executing(current, request),
        OperationState::AwaitingResolution | OperationState::Halted => match request {
            V1LifecycleRequest::ResumeStart => Ok(V1NextAction::Respond(
                V1ResponseDisposition::Stopped(record.state),
            )),
            V1LifecycleRequest::Continue if record.state == OperationState::AwaitingResolution => {
                let member_id = record
                    .selected_targets
                    .iter()
                    .find(|id| {
                        record
                            .participants
                            .get(*id)
                            .is_some_and(|row| row.state == ParticipantState::Conflicted)
                    })
                    .cloned()
                    .ok_or_else(|| dispatch_error("awaiting-resolution has no conflicted owner"))?;
                observe(
                    current,
                    request,
                    ObservationKind::ParticipantPreparation { member_id },
                )
            }
            V1LifecycleRequest::Continue => Ok(V1NextAction::Apply(V1Transition::Operation(
                Box::new(OperationTransition::BeginExecution),
            ))),
            V1LifecycleRequest::Abort => observe(current, request, ObservationKind::RollbackEntry),
            V1LifecycleRequest::Preserve => {
                observe(current, request, ObservationKind::PreservationEntry)
            }
            _ => reject("request is not legal while the merge is stopped"),
        },
        OperationState::Finalizing => finalizing(current, request),
        OperationState::Preserving => {
            observe(current, request, ObservationKind::PreservationCursor)
        }
        OperationState::RollingBack => observe(current, request, ObservationKind::RollbackCursor),
        OperationState::Completed | OperationState::Aborted | OperationState::RecoveryRequired => {
            unreachable!("handled before state dispatch")
        }
    }
}

fn executing(current: &StoredV1Record, request: V1LifecycleRequest) -> ModelResult<V1NextAction> {
    let record = current.record();
    match request {
        V1LifecycleRequest::Abort => {
            return observe(current, request, ObservationKind::RollbackEntry);
        }
        V1LifecycleRequest::Preserve => {
            return observe(current, request, ObservationKind::PreservationEntry);
        }
        V1LifecycleRequest::Archive => return reject("an open merge cannot be archived"),
        V1LifecycleRequest::Status => unreachable!(),
        V1LifecycleRequest::ResumeStart | V1LifecycleRequest::Continue => {}
    }
    if let Some(member_id) = next_forward_participant(record, request) {
        return observe(
            current,
            request,
            ObservationKind::ParticipantPreparation { member_id },
        );
    }
    if record
        .participants
        .values()
        .any(|row| row.state == ParticipantState::Conflicted)
    {
        return Ok(V1NextAction::Apply(V1Transition::Operation(Box::new(
            OperationTransition::AwaitResolution,
        ))));
    }
    if has_halt_cause(record) {
        return Ok(V1NextAction::Apply(V1Transition::Operation(Box::new(
            OperationTransition::Halt,
        ))));
    }
    observe(current, request, ObservationKind::ParticipantsComplete)
}

fn finalizing(current: &StoredV1Record, request: V1LifecycleRequest) -> ModelResult<V1NextAction> {
    let kind = match request {
        V1LifecycleRequest::Archive => return reject("an open merge cannot be archived"),
        V1LifecycleRequest::Abort => ObservationKind::RollbackEntry,
        V1LifecycleRequest::Preserve => ObservationKind::PreservationEntry,
        _ if current.record().accepted_workspace.is_none() => ObservationKind::Acceptance,
        _ => ObservationKind::Publication,
    };
    observe(current, request, kind)
}

fn pending_participant(record: &MergeOperationRecordV1) -> Option<String> {
    record
        .participants
        .iter()
        .find_map(|(id, row)| row.pending_action.is_some().then(|| id.clone()))
}

fn next_forward_participant(
    record: &MergeOperationRecordV1,
    request: V1LifecycleRequest,
) -> Option<String> {
    record.selected_targets.iter().find_map(|id| {
        record.participants.get(id).and_then(|row| {
            (matches!(
                row.state,
                ParticipantState::Planned
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
            ) || request == V1LifecycleRequest::Continue
                && row.state == ParticipantState::Conflicted)
                .then(|| id.clone())
        })
    })
}

pub(super) fn has_halt_cause(record: &MergeOperationRecordV1) -> bool {
    record.participants.values().any(|row| {
        row.state == ParticipantState::Failed
            || row.state == ParticipantState::Conflicted && row.error.is_some()
    })
}

fn observation_owner(kind: &ObservationKind) -> String {
    match kind {
        ObservationKind::ParticipantPreparation { member_id }
        | ObservationKind::ParticipantAction { member_id } => member_id.clone(),
        ObservationKind::Acceptance
        | ObservationKind::ParticipantsComplete
        | ObservationKind::PreservationEntry
        | ObservationKind::RollbackEntry
        | ObservationKind::Recovery
        | ObservationKind::Archive => "@operation".into(),
        ObservationKind::Publication => "@publication".into(),
        ObservationKind::PreservationCursor => "@preservation".into(),
        ObservationKind::RollbackCursor => "@rollback".into(),
    }
}

fn observe(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    kind: ObservationKind,
) -> ModelResult<V1NextAction> {
    Ok(V1NextAction::Observe(BoundObservationRequest::issue(
        current, request, kind,
    )?))
}

fn reject(detail: &str) -> ModelResult<V1NextAction> {
    Ok(V1NextAction::Reject(dispatch_error(detail)))
}

pub(super) fn dispatch_error(detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 lifecycle dispatch rejected: {detail}"),
    )
}
