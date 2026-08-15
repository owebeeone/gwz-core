use super::*;

pub(in crate::workspace_ops::merge::v1_lifecycle) struct V1Invocation {
    preparation_owners: Vec<String>,
    attempted_actions: Vec<(PhysicalActionKind, ExecutionDiagnostic)>,
}

impl V1Invocation {
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn new() -> Self {
        Self {
            preparation_owners: Vec::new(),
            attempted_actions: Vec::new(),
        }
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<()> {
        let ObservationKind::ParticipantPreparation { member_id } = request.kind() else {
            return Ok(());
        };
        if current.record().state == OperationState::AwaitingResolution {
            return Ok(());
        }
        if self
            .preparation_owners
            .iter()
            .any(|owner| owner == member_id)
        {
            let row = current.record().participants.get(member_id);
            return Err(row.map_or_else(
                || dispatch_error("repeated preparation owner is missing"),
                |row| {
                    dispatch_error(
                        "participant preparation made no progress within this invocation",
                    )
                    .with_member(member_id, &row.path)
                },
            ));
        }
        self.preparation_owners.push(member_id.clone());
        Ok(())
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn next_action(
        &self,
        current: &StoredV1Record,
        request: V1LifecycleRequest,
    ) -> ModelResult<V1NextAction> {
        let action = next_action(current, request)?;
        let V1NextAction::Observe(ref requested) = action else {
            return Ok(action);
        };
        let ObservationKind::ParticipantPreparation { member_id } = requested.kind() else {
            return Ok(action);
        };
        if !self
            .preparation_owners
            .iter()
            .any(|owner| owner == member_id)
        {
            return Ok(action);
        }
        let row = current
            .record()
            .participants
            .get(member_id)
            .ok_or_else(|| dispatch_error("preparation owner is missing from the record"))?;
        if row.state != ParticipantState::Conflicted {
            return Ok(action);
        }
        let record = current.record();
        let start = record
            .selected_targets
            .iter()
            .position(|target| target == member_id)
            .map_or(record.selected_targets.len(), |position| position + 1);
        if let Some(next) = record.selected_targets[start..]
            .iter()
            .find(|target| self.is_unvisited_candidate(record, target, request))
        {
            return observe(
                current,
                request,
                ObservationKind::ParticipantPreparation {
                    member_id: next.to_string(),
                },
            );
        }
        Ok(V1NextAction::Apply(V1Transition::Operation(Box::new(
            OperationTransition::AwaitResolution,
        ))))
    }

    fn is_unvisited_candidate(
        &self,
        record: &MergeOperationRecordV1,
        target: &str,
        request: V1LifecycleRequest,
    ) -> bool {
        record.participants.get(target).is_some_and(|row| {
            matches!(
                row.state,
                ParticipantState::Planned
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
            ) || request == V1LifecycleRequest::Continue
                && row.state == ParticipantState::Conflicted
                && !self.preparation_owners.iter().any(|owner| owner == target)
        })
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn before_execute(
        &self,
        action: &PhysicalActionKind,
    ) -> ModelResult<()> {
        let Some((_, diagnostic)) = self
            .attempted_actions
            .iter()
            .find(|(attempted, _)| attempted == action)
        else {
            return Ok(());
        };
        Err(match diagnostic {
            ExecutionDiagnostic::Failed { code, message, .. } => {
                ModelError::new(*code, message.clone())
            }
            ExecutionDiagnostic::Success => dispatch_error(
                "owned action made no observable progress; refusing a second execution",
            ),
        })
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn record_execution(
        &mut self,
        action: PhysicalActionKind,
        diagnostic: ExecutionDiagnostic,
    ) {
        self.attempted_actions.push((action, diagnostic));
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn after_commit(
        &self,
        current: &StoredV1Record,
    ) -> Option<V1ResponseDisposition> {
        let record = current.record();
        if record.state == OperationState::RecoveryRequired {
            return Some(V1ResponseDisposition::Stopped(record.state));
        }
        let preparation_failed = self.preparation_owners.iter().any(|member_id| {
            record.participants.get(member_id).is_some_and(|row| {
                row.state == ParticipantState::Failed
                    && row.error.is_some()
                    && row.pending_action.is_none()
            })
        });
        let action_failed = self.attempted_actions.iter().any(|(action, _)| {
            let PhysicalActionKind::Participant { member_id, .. } = action else {
                return false;
            };
            record
                .participants
                .get(member_id)
                .is_some_and(|row| row.error.is_some() && row.pending_action.is_some())
        });
        let awaiting_resolution = record.state == OperationState::AwaitingResolution
            && self.preparation_owners.iter().any(|member_id| {
                record
                    .participants
                    .get(member_id)
                    .is_some_and(|row| row.state == ParticipantState::Conflicted)
            });
        (record.state == OperationState::Halted && (preparation_failed || action_failed)
            || awaiting_resolution)
            .then_some(V1ResponseDisposition::Stopped(record.state))
    }
}
