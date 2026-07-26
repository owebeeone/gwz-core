use super::*;

/// Continue owns the workspace mutator lock. Its caller must resolve `root`
/// without parsing live workspace metadata when recovery discovery found it.
pub(crate) fn handle_continue<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    let Some(mut record) = store.discover_open(root)? else {
        return closed_or_missing(store, root, request.merge_id.as_deref(), context);
    };
    super::super::validate::validate_open_merge_id(request.merge_id.as_deref(), &record.merge_id)?;
    match record.state {
        OperationState::Finalizing | OperationState::Completed => {
            let completed = super::super::finalize::finalize(
                backend,
                store,
                root,
                &mut record,
                context,
                emitter,
            )?;
            return if completed {
                record.to_response(context)
            } else {
                observed_response(backend, root, record, context)
            };
        }
        OperationState::Executing
        | OperationState::AwaitingResolution
        | OperationState::Halted
        | OperationState::RecoveryRequired => {}
        state => return Err(wrong_state(&record.merge_id, state)),
    }

    reconcile_pending_actions(backend, store, root, &mut record, emitter)?;
    let actions = preflight(backend, root, &record, context.attribution.as_ref())?;
    super::super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Executing,
        emitter,
    )?;

    for (position, action) in actions.iter().enumerate() {
        emitter.member_started(&action.target_id, &action.path);
        if !action.durable {
            set_pending_action(&mut record, action)?;
            super::super::persist_merge_record(store, root, &record, emitter)?;
        }
        let result: Result<Outcome, ActionFailure> = match action.kind {
            ContinueActionKind::Resolve => {
                resolve_conflict(backend, root, &record, action, context)
                    .map_err(ActionFailure::Ordinary)
            }
            ContinueActionKind::Retry(kind) => {
                retry_merge(backend, root, &record, action, kind, context)
            }
        };
        match result {
            Ok(outcome) => {
                apply_outcome(&mut record, &action.target_id, outcome, None)?;
                super::super::persist_merge_record(store, root, &record, emitter)?;
                super::super::emit_merge_member_finished(emitter, &record, &action.target_id)?;
            }
            Err(ActionFailure::RecoveryRequired(error)) => {
                let contextual = error.with_member(&action.target_id, &action.path);
                apply_recovery_failure(&mut record, &action.target_id, &contextual)?;
                super::super::emit_merge_member_finished(emitter, &record, &action.target_id)?;
                return Err(contextual);
            }
            Err(ActionFailure::Ordinary(error)) => {
                let contextual = error.with_member(&action.target_id, &action.path);
                if action.durable {
                    let participant = participant(&record, &action.target_id)?;
                    emitter.merge_member_finished(
                        participant.to_protocol(&action.target_id, &record.source_ref),
                    );
                    return Err(contextual);
                }
                apply_failure(&mut record, &action.target_id, &contextual)?;
                super::super::persist_merge_record(store, root, &record, emitter)?;
                super::super::emit_merge_member_finished(emitter, &record, &action.target_id)?;
                mark_later_planned_unattempted(
                    store,
                    root,
                    &mut record,
                    &actions[position + 1..],
                    emitter,
                )?;
                super::super::persist_operation_transition(
                    store,
                    root,
                    &mut record,
                    OperationState::Halted,
                    emitter,
                )?;
                return observed_response(backend, root, record, context);
            }
        }
    }

    let snapshot = super::super::status::snapshot_status(backend, root, record.clone())?;
    if !snapshot.operation_drift.is_empty()
        || snapshot
            .participants
            .values()
            .any(|participant| !participant.drift.is_empty())
    {
        return observed_response(backend, root, record, context);
    }
    let next = remaining_state(&record);
    if next == OperationState::Finalizing {
        super::super::enter_finalizing(store, root, &mut record, emitter)?;
        let completed =
            super::super::finalize::finalize(backend, store, root, &mut record, context, emitter)?;
        return if completed {
            record.to_response(context)
        } else {
            observed_response(backend, root, record, context)
        };
    } else {
        super::super::persist_operation_transition(store, root, &mut record, next, emitter)?;
    }
    observed_response(backend, root, record, context)
}
