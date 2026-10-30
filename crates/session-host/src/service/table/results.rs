use std::sync::Arc;

use gwz_session_contract::Lane;

use crate::service::{Code, HostError, HostPorts, Inner, Outcome, Outgoing, Terminal, View};

use super::state::{Phase, State};

/// `operation.result` (`result` true) or `operation.response` (§4.2).
pub(super) fn view<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    operation_id: &str,
    result: bool,
) -> Vec<Outgoing> {
    let owner = match state.operation(operation_id) {
        Ok(owner) => owner,
        Err(error) => {
            state.answered_on(call_id, Lane::Call);
            return vec![inner.refuse(call_id, Lane::Call, error)];
        }
    };
    let Some(record) = state.records.get_mut(&owner) else {
        state.answered_on(call_id, Lane::Call);
        return vec![inner.refuse(
            call_id,
            Lane::Call,
            HostError::new(Code::OperationExpired, "the operation's record is gone"),
        )];
    };
    match record.outcome.as_ref() {
        Some(outcome) => {
            let reply = view_reply(inner, owner, call_id, outcome, result);
            state.answered_on(call_id, Lane::Call);
            vec![reply]
        }
        None => {
            if result {
                record.results.push(call_id);
            } else {
                record.responses.push(call_id);
            }
            state.others.insert(call_id);
            Vec::new()
        }
    }
}

/// The reply to a view of `owner`'s terminal, which delivers that view once
/// it is handed over (§5.4).
pub(super) fn view_reply<P: HostPorts>(
    inner: &Inner<P>,
    owner: u64,
    call_id: u64,
    outcome: &Outcome<P::Failure>,
    result: bool,
) -> Outgoing {
    let (mut reply, view) = if result {
        (
            inner.reply(call_id, Lane::Call, outcome.result.clone()),
            View::Result,
        )
    } else {
        let reply = match &outcome.terminal {
            Terminal::Completed(response) => inner.reply(call_id, Lane::Call, response.clone()),
            Terminal::Cancelled(failure) | Terminal::Failed(failure) => {
                inner.error(call_id, Lane::Call, failure)
            }
        };
        (reply, View::Response)
    };
    reply.delivers = Some((owner, view));
    reply
}

/// `operation.release` (§4.2, §5.4).
pub(super) fn release<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    operation_id: &str,
) -> Vec<Outgoing> {
    state.answered_on(call_id, Lane::Call);
    let owner = match state.operation(operation_id) {
        Ok(owner) => owner,
        Err(error) => {
            return vec![inner.refuse(call_id, Lane::Call, error)];
        }
    };
    let live = state
        .records
        .get(&owner)
        .is_none_or(|record| record.phase != Phase::Terminal);
    if live {
        return vec![inner.refuse(
            call_id,
            Lane::Call,
            HostError::new(
                Code::OpenOperation,
                format!("operation {operation_id} is still live"),
            ),
        )];
    }
    state.discard(owner);
    vec![inner.reply(call_id, Lane::Call, inner.ports.released())]
}
