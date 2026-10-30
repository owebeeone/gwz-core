use std::collections::HashSet;
use std::sync::Arc;

use gwz_session_contract::Lane;

use crate::lock;
use crate::service::workers;
use crate::service::{
    Call, Class, Cleanup, Code, HostError, HostPorts, Inner, Mode, Outcome, Outgoing, Resolved,
    Terminal,
};

use super::cancel::cancelled_before_start;
use super::settle::settle;
use super::state::{Phase, State};

/// The admission thread: resolves each received call in receipt order, then
/// admits, queues or refuses it (§5.1).
pub(crate) fn admission_loop<P: HostPorts>(inner: &Arc<Inner<P>>) {
    loop {
        let (call_id, call, cancelled) = {
            let mut state = lock(&inner.state);
            loop {
                if state.ended {
                    return;
                }
                if let Some(call_id) = state.pending.pop_front() {
                    let Some(record) = state.records.get(&call_id) else {
                        continue;
                    };
                    if record.phase != Phase::Received {
                        continue;
                    }
                    let cancelled = record.controls.view().token().is_cancelled();
                    break (call_id, record.call.clone(), cancelled);
                }
                state = inner
                    .admission
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let resolved = if cancelled {
            None
        } else {
            Some(inner.ports.resolve(&inner.session, &call))
        };
        let outgoing = {
            let mut state = lock(&inner.state);
            state.unflushed += 1;
            admit(inner, &mut state, call_id, resolved)
        };
        inner.flush(outgoing);
    }
}

/// Admits a resolved call, or settles it (§5.1).
pub(super) fn admit<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    resolved: Option<Result<Resolved, P::Failure>>,
) -> Vec<Outgoing> {
    let Some(record) = state.records.get(&call_id) else {
        return Vec::new();
    };
    if record.phase != Phase::Received {
        return Vec::new();
    }
    let operation = record.operation.clone();
    let cancelled = record.controls.view().token().is_cancelled();
    let call = record.call.clone();
    let resolved = match resolved {
        Some(Ok(resolved)) if !cancelled => resolved,
        Some(Err(failure)) if !cancelled => {
            let outcome = failed_before_start(inner, &call, operation.as_ref(), failure);
            return settle(inner, state, call_id, outcome);
        }
        _ => {
            let outcome = cancelled_before_start(inner, &call, operation.as_ref());
            return settle(inner, state, call_id, outcome);
        }
    };
    let Some((operation_id, class)) = operation else {
        if let Some(record) = state.records.get_mut(&call_id) {
            record.resolved = Some(resolved);
            record.phase = Phase::DirectWaiting;
        }
        state.direct_queue.push_back(call_id);
        return schedule(inner, state);
    };
    let refusal = if state.requests.contains_key(&resolved.request_id) {
        Some(HostError::new(
            Code::InvalidRequest,
            format!(
                "request_id {} matches a live operation's",
                resolved.request_id
            ),
        ))
    } else if matches!(class, Class::Write | Class::Push)
        && inner.detached.running_on(&resolved.workspace)
    {
        Some(HostError::new(
            Code::WorkspaceBusy,
            "a worker detached at a session's close bound still runs on this workspace",
        ))
    } else if state.queue.len() >= inner.limits.queued_operations {
        Some(HostError::new(
            Code::SessionFull,
            "the session's operation queue is full",
        ))
    } else if state.table_entries >= inner.limits.operation_table && !state.evict() {
        Some(HostError::new(
            Code::SessionFull,
            "the session's operation table holds no record it can evict",
        ))
    } else {
        None
    };
    if let Some(error) = refusal {
        let failure = inner.ports.failure(error);
        let outcome = failed_before_start(inner, &call, Some(&(operation_id, class)), failure);
        return settle(inner, state, call_id, outcome);
    }
    state.table_entries += 1;
    state.requests.insert(resolved.request_id.clone(), call_id);
    let mut outgoing = Vec::new();
    let mut accepted = None;
    if call.mode == Mode::Submit {
        accepted = Some(inner.ports.accepted(&call, &resolved, &operation_id));
    }
    if let Some(record) = state.records.get_mut(&call_id) {
        record.in_table = true;
        record.resolved = Some(resolved);
        record.phase = Phase::Queued;
        record.accepted = accepted.is_some();
    }
    if let Some(body) = accepted {
        state.answered_on(call_id, Lane::Call);
        outgoing.push(inner.reply(call_id, Lane::Call, body));
    }
    state.queue.push_back(call_id);
    outgoing.extend(schedule(inner, state));
    outgoing
}

/// The terminal of a call refused before it ran: `Failed`, no effect.
pub(super) fn failed_before_start<P: HostPorts>(
    inner: &Inner<P>,
    call: &Call,
    operation: Option<&(String, Class)>,
    failure: P::Failure,
) -> Outcome<P::Failure> {
    let result = operation.map_or_else(Vec::new, |(operation_id, _)| {
        inner.ports.settled_result(call, operation_id, &failure)
    });
    Outcome {
        terminal: Terminal::Failed(failure),
        result,
        cleanup: Cleanup::default(),
    }
}

/// Starts what the limits and classes allow: queued operations in queue order
/// per workspace, and direct calls on free direct workers (§5.1, §5.2).
pub(crate) fn schedule<P: HostPorts>(inner: &Arc<Inner<P>>, state: &mut State<P>) -> Vec<Outgoing> {
    let mut outgoing = Vec::new();
    let mut blocked: HashSet<String> = HashSet::new();
    let mut index = 0;
    while index < state.queue.len() && state.running < inner.limits.running_operations {
        let call_id = state.queue[index];
        let Some(record) = state.records.get(&call_id) else {
            state.queue.remove(index);
            continue;
        };
        let (Some((_, class)), Some(resolved)) =
            (record.operation.as_ref(), record.resolved.as_ref())
        else {
            state.queue.remove(index);
            continue;
        };
        let class = *class;
        let workspace = resolved.workspace.clone();
        // A worker another session detached since admission: refused, not
        // run beside it (2.0 §3.2).
        if matches!(class, Class::Write | Class::Push) && inner.detached.running_on(&workspace) {
            state.queue.remove(index);
            let call = record.call.clone();
            let operation = record.operation.clone();
            let failure = inner.ports.failure(HostError::new(
                Code::WorkspaceBusy,
                "a worker detached at a session's close bound still runs on this workspace",
            ));
            let outcome = failed_before_start(inner, &call, operation.as_ref(), failure);
            outgoing.extend(settle(inner, state, call_id, outcome));
            continue;
        }
        let admits = state
            .busy
            .get(&workspace)
            .copied()
            .unwrap_or_default()
            .admits(class);
        if blocked.contains(&workspace) || !admits {
            blocked.insert(workspace);
            index += 1;
            continue;
        }
        state.queue.remove(index);
        state.running += 1;
        state.busy.entry(workspace).or_default().change(class, true);
        outgoing.extend(workers::start_operation(inner, state, call_id));
    }
    while state.direct_running < inner.limits.direct_workers && !state.direct_queue.is_empty() {
        state.direct_running += 1;
        outgoing.extend(workers::start_direct(inner, state));
    }
    outgoing
}
