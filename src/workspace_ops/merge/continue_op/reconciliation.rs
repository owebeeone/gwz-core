use super::*;

enum ReconciledPendingAction {
    NotStarted,
    ExpectedConflict { conflict_paths: Vec<String> },
    Completed(String),
}

pub(super) fn reconcile_pending_actions<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let target_ids = record.selected_targets.clone();
    let mut reconciliations = Vec::new();
    for target_id in target_ids {
        let participant = participant(record, &target_id)?;
        if participant.pending_action.is_none() {
            continue;
        }
        let result =
            super::super::status::reconcile_pending_action(backend, root, &target_id, participant)?;
        let reconciliation = match result {
            super::super::status::PendingActionReconciliation::NotStarted => {
                ReconciledPendingAction::NotStarted
            }
            super::super::status::PendingActionReconciliation::ExpectedConflict {
                conflict_paths,
            } => ReconciledPendingAction::ExpectedConflict { conflict_paths },
            super::super::status::PendingActionReconciliation::Completed { resulting_commit } => {
                ReconciledPendingAction::Completed(resulting_commit)
            }
            super::super::status::PendingActionReconciliation::Ambiguous { reason, .. } => {
                return Err(ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    format!("pending merge action is ambiguous: {reason}"),
                )
                .with_member(&target_id, &participant.path));
            }
        };
        reconciliations.push((target_id, participant.path.clone(), reconciliation));
    }

    let adopted = reconciliations
        .iter()
        .filter(|(_, _, reconciliation)| {
            !matches!(reconciliation, ReconciledPendingAction::NotStarted)
        })
        .map(|(target_id, path, _)| (target_id.clone(), path.clone()))
        .collect::<Vec<_>>();
    for (target_id, path) in &adopted {
        emitter.member_started(target_id, path);
    }
    for (target_id, _, reconciliation) in reconciliations {
        if !matches!(reconciliation, ReconciledPendingAction::NotStarted) {
            apply_reconciled_pending(record, &target_id, reconciliation)?;
        }
    }
    if !adopted.is_empty() {
        super::super::persist_merge_record(store, root, record, emitter)?;
        for (target_id, _) in &adopted {
            super::super::emit_merge_member_finished(emitter, record, target_id)?;
        }
    }
    Ok(())
}

fn apply_reconciled_pending(
    record: &mut MergeOperationRecord,
    target_id: &str,
    reconciliation: ReconciledPendingAction,
) -> ModelResult<()> {
    let participant = participant(record, target_id)?;
    let pending = participant
        .pending_action
        .as_ref()
        .cloned()
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "pending-action reconciliation has no pending action",
            )
            .with_member(target_id, &participant.path)
        })?;
    match reconciliation {
        ReconciledPendingAction::NotStarted => Ok(()),
        ReconciledPendingAction::ExpectedConflict { conflict_paths } => {
            if pending.kind != PendingMergeActionKind::TrueMerge {
                return Err(invariant(
                    "only a pending true merge can reconcile to a native conflict",
                ));
            }
            apply_outcome(
                record,
                target_id,
                Outcome {
                    state: ParticipantState::Conflicted,
                    resulting_commit: None,
                    expected_merge_head: Some(pending.source_commit),
                    conflict_paths,
                    // Git mutated before the durable outcome was written, so
                    // the original conflict-marker bytes are unavailable.
                    // Empty is the durable "unverified original" sentinel:
                    // continue may still resolve it, but preserve-abort must
                    // never treat later live bytes as the original snapshot.
                    conflict_snapshot: Vec::new(),
                },
                None,
            )
        }
        ReconciledPendingAction::Completed(resulting_commit) => {
            let state = match pending.kind {
                PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
            };
            apply_outcome(
                record,
                target_id,
                Outcome::clean(state, resulting_commit),
                None,
            )
        }
    }
}

pub(super) fn set_pending_action(
    record: &mut MergeOperationRecord,
    action: &ContinueAction,
) -> ModelResult<()> {
    let participant = record
        .participants
        .get_mut(&action.target_id)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{}'", action.target_id),
            )
        })?;
    let intent = super::super::integration::IntegrationIntent::from_record(participant);
    let integration = match (action.kind, &action.prepared) {
        (ContinueActionKind::Resolve, ContinuePrepared::Resolution(prepared)) => {
            super::super::integration::PreparedIntegration::resolution(intent, prepared)
        }
        (ContinueActionKind::Retry(kind), ContinuePrepared::Merge(prepared)) => {
            super::super::integration::PreparedIntegration::from_merge(intent, kind, prepared)
                .map_err(invariant)?
        }
        (ContinueActionKind::Resolve, ContinuePrepared::Merge(_))
        | (ContinueActionKind::Retry(_), ContinuePrepared::Resolution(_)) => {
            return Err(invariant(
                "continue action kind does not match its prepared integration",
            ));
        }
    };
    participant.pending_action = Some(integration.to_pending());
    Ok(())
}
