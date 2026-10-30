use std::sync::Arc;

use gwz_session_contract::Lane;

use crate::service::{HostPorts, Inner, Mode, Outcome, Outgoing, Terminal};

use super::results::view_reply;
use super::state::{Phase, State};

/// Settles a call with `outcome` and answers what waited on it (§4.2, §5.2).
/// Settling twice changes nothing.
pub(crate) fn settle<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    outcome: Outcome<P::Failure>,
) -> Vec<Outgoing> {
    let Some(record) = state.records.get_mut(&call_id) else {
        return Vec::new();
    };
    if record.phase == Phase::Terminal {
        return Vec::new();
    }
    let was_running = record.phase == Phase::Running;
    record.phase = Phase::Terminal;
    record.gate = None;
    // The worker's thread keeps the only strong marker from here on, so the
    // host's detached record ends with the thread (§8 step 5).
    record.alive = None;
    if let Some(events) = record.events.as_ref() {
        events.complete();
    }
    let mut outgoing = Vec::new();
    let cancels = std::mem::take(&mut record.cancels);
    let results = std::mem::take(&mut record.results);
    let responses = std::mem::take(&mut record.responses);
    for cancel in &cancels {
        outgoing.push(inner.reply(*cancel, Lane::Control, inner.ports.cleanup(outcome.cleanup)));
    }
    for waiter in &results {
        outgoing.push(view_reply(inner, call_id, *waiter, &outcome, true));
    }
    for waiter in &responses {
        outgoing.push(view_reply(inner, call_id, *waiter, &outcome, false));
    }
    let submitted = record.call.mode == Mode::Submit && record.operation.is_some();
    let keep = submitted && record.accepted;
    let own = if keep {
        None
    } else {
        Some(match &outcome.terminal {
            Terminal::Completed(response) => inner.reply(call_id, record.lane, response.clone()),
            Terminal::Cancelled(failure) | Terminal::Failed(failure) => {
                inner.error(call_id, record.lane, failure)
            }
        })
    };
    let lane = record.lane;
    let class = record.operation.as_ref().map(|(_, class)| *class);
    let workspace = record
        .resolved
        .as_ref()
        .map(|resolved| (resolved.workspace.clone(), resolved.request_id.clone()));
    state.cleanup_pending = state
        .cleanup_pending
        .saturating_add(outcome.cleanup.pending_local_work);
    state.cleanup_confirmed |= outcome.cleanup.peer_cleanup_confirmed;
    if let Some(record) = state.records.get_mut(&call_id) {
        record.outcome = Some(outcome);
    }
    for cancel in cancels {
        state.answered_on(cancel, Lane::Control);
    }
    for waiter in results.into_iter().chain(responses) {
        state.answered_on(waiter, Lane::Call);
    }
    if let Some((workspace, request_id)) = workspace {
        if state.requests.get(&request_id) == Some(&call_id) {
            state.requests.remove(&request_id);
        }
        if was_running && let Some(class) = class {
            state.running = state.running.saturating_sub(1);
            state
                .busy
                .entry(workspace)
                .or_default()
                .change(class, false);
        }
    }
    if let Some(reply) = own {
        state.answered_on(call_id, lane);
        state.discard(call_id);
        outgoing.push(reply);
    } else {
        state.retained.push_back(call_id);
    }
    inner.settled.notify_all();
    inner.admission.notify_all();
    outgoing
}
