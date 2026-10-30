use std::sync::Arc;

use gwz_session_contract::Lane;

use crate::service::{
    Call, CancelTarget, Class, Cleanup, Code, HostError, HostPorts, Inner, Outcome, Outgoing,
    Terminal,
};

use super::admit::schedule;
use super::settle::settle;
use super::state::{Phase, State};

/// `operation.cancel`, on the reading thread (§4.2, §5.3).
pub(super) fn cancel<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    target: CancelTarget,
) -> Vec<Outgoing> {
    let target = match target {
        CancelTarget::Call(target) if target >= call_id => Err(HostError::new(
            Code::OperationNotFound,
            format!("this session received no call {target}"),
        )),
        CancelTarget::Call(target) if state.others.contains(&target) => Err(HostError::new(
            Code::InvalidRequest,
            format!("call {target} is a read, result or control call"),
        )),
        CancelTarget::Call(target) if state.records.contains_key(&target) => Ok(target),
        CancelTarget::Call(target) => Err(HostError::new(
            Code::OperationExpired,
            format!("call {target} has already been answered or released"),
        )),
        CancelTarget::Operation(operation_id) => state.operation(&operation_id),
    };
    let target = match target {
        Ok(target) => target,
        Err(error) => {
            state.answered_on(call_id, Lane::Control);
            return vec![inner.refuse(call_id, Lane::Control, error)];
        }
    };
    state.others.insert(call_id);
    let Some(record) = state.records.get_mut(&target) else {
        return Vec::new();
    };
    match record.phase {
        Phase::Terminal => {
            let cleanup = record.outcome.as_ref().map(|outcome| outcome.cleanup);
            state.answered_on(call_id, Lane::Control);
            vec![inner.reply(
                call_id,
                Lane::Control,
                inner.ports.cleanup(cleanup.unwrap_or_default()),
            )]
        }
        Phase::Received | Phase::Running => {
            record.cancels.push(call_id);
            record.controls.cancel();
            Vec::new()
        }
        Phase::Queued | Phase::DirectWaiting => {
            record.cancels.push(call_id);
            record.controls.cancel();
            state.queue.retain(|queued| *queued != target);
            state.direct_queue.retain(|queued| *queued != target);
            let outcome = cancelled_before_start(inner, &record.call, record.operation.as_ref());
            let mut outgoing = settle(inner, state, target, outcome);
            outgoing.extend(schedule(inner, state));
            outgoing
        }
    }
}

/// The terminal of a call settled before it ran: `Cancelled`, no effect.
pub(crate) fn cancelled_before_start<P: HostPorts>(
    inner: &Inner<P>,
    call: &Call,
    operation: Option<&(String, Class)>,
) -> Outcome<P::Failure> {
    let failure = inner.ports.failure(HostError::new(
        Code::Cancelled,
        "cancelled before it started",
    ));
    let result = operation.map_or_else(Vec::new, |(operation_id, _)| {
        inner.ports.settled_result(call, operation_id, &failure)
    });
    Outcome {
        terminal: Terminal::Cancelled(failure),
        result,
        cleanup: Cleanup::default(),
    }
}
