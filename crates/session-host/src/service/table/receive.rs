use std::sync::Arc;
use std::time::Instant;

use gwz_session_contract::Lane;

use crate::gate::CallControls;
use crate::lock;
use crate::service::events::{EventLog, Held};
use crate::service::workers;
use crate::service::{
    Call, Class, Cleanup, Code, EventsCall, HostError, HostPorts, Inner, Outgoing, ProtocolError,
    Request,
};

use super::cancel::cancel;
use super::results::{release, view};
use super::state::{Phase, Record, State};

/// Reads one call (§5.1): creates its record, answers what never waits, and
/// queues the rest for admission.
pub(crate) fn receive<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    call: Call,
) -> Result<Vec<Outgoing>, ProtocolError> {
    let mut state = lock(&inner.state);
    if call.call_id <= state.highest_call {
        return Err(ProtocolError(format!(
            "call_id {} is not greater than {}, the highest received",
            call.call_id, state.highest_call
        )));
    }
    state.highest_call = call.call_id;
    if state.ended {
        return Ok(Vec::new());
    }
    let call_id = call.call_id;
    // The lane comes from the method's name, before anything is decoded, so
    // a malformed control call's refusal takes the control lane (§3).
    let lane = inner.ports.lane(&call.method);
    let request = match inner.ports.classify(&call) {
        Ok(request) => request,
        Err(error) => {
            return Ok(vec![inner.refuse(call_id, lane, error)]);
        }
    };
    if state.closing && request != Request::Close {
        return Ok(vec![inner.refuse(
            call_id,
            lane,
            HostError::new(Code::InvalidRequest, "session closing"),
        )]);
    }
    let full = match lane {
        Lane::Call => state.outstanding >= inner.limits.outstanding_calls,
        Lane::Control => state.controls >= inner.limits.control_reserve,
    };
    if full {
        return Ok(vec![inner.refuse(
            call_id,
            lane,
            HostError::new(
                Code::SessionFull,
                "the session's outstanding calls are at their limit",
            ),
        )]);
    }
    match lane {
        Lane::Call => {
            state.outstanding += 1;
        }
        Lane::Control => {
            state.controls += 1;
        }
    }
    match request {
        Request::Direct => {
            create(inner, &mut state, call, None);
            inner.admission.notify_all();
            Ok(Vec::new())
        }
        Request::Operation {
            class,
            response_view,
        } => {
            create(inner, &mut state, call, Some((class, response_view)));
            inner.admission.notify_all();
            Ok(Vec::new())
        }
        Request::Events(events) => Ok(read_events(inner, &mut state, call_id, events)),
        Request::Spool => {
            drop(state);
            let answer = inner.ports.spool(&inner.session, &call);
            let mut state = lock(&inner.state);
            state.answered_on(call_id, lane);
            Ok(vec![match answer {
                Ok(body) => inner.reply(call_id, lane, body),
                Err(failure) => inner.error(call_id, lane, &failure),
            }])
        }
        Request::Result { operation_id } => {
            Ok(view(inner, &mut state, call_id, &operation_id, true))
        }
        Request::Response { operation_id } => {
            Ok(view(inner, &mut state, call_id, &operation_id, false))
        }
        Request::Cancel(target) => Ok(cancel(inner, &mut state, call_id, target)),
        Request::Release { operation_id } => Ok(release(inner, &mut state, call_id, &operation_id)),
        Request::Close => {
            state.others.insert(call_id);
            if let Some(report) = state.report {
                state.answered_on(call_id, Lane::Control);
                return Ok(vec![close_report(inner, call_id, report)]);
            }
            state.closers.push(call_id);
            let start = !state.close_started;
            drop(state);
            if start {
                workers::close(inner, Some(call_id));
            }
            Ok(Vec::new())
        }
    }
}

/// The close report's reply: ordinary class, so it is exempt from priority
/// (`GwzSplitCommandChannel.md` §3.3), on the control lane.
pub(crate) fn close_report<P: HostPorts>(
    inner: &Inner<P>,
    call_id: u64,
    report: Cleanup,
) -> Outgoing {
    Outgoing {
        frame: inner.ports.reply(call_id, inner.ports.cleanup(report)),
        lane: Lane::Control,
        target: call_id,
        control: false,
        delivers: None,
    }
}

/// Creates a direct or operation call's record, token and gate at receipt
/// (O7) and queues it for admission.
pub(super) fn create<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call: Call,
    class: Option<(Class, bool)>,
) {
    let (controls, gate) = CallControls::new(&inner.session);
    let response_view = class.is_some_and(|(_, response_view)| response_view);
    let operation = class.map(|(class, _)| {
        let operation_id = format!("op-{}", state.next_operation);
        state.next_operation += 1;
        (operation_id, class)
    });
    let events = operation.as_ref().map(|(operation_id, _)| {
        Arc::new(EventLog::new(
            inner.limits.event_log,
            inner.ports.reset_event(operation_id),
            Arc::clone(&inner.reads),
        ))
    });
    if let Some((operation_id, _)) = operation.as_ref() {
        state.operations.insert(operation_id.clone(), call.call_id);
    }
    let call_id = call.call_id;
    state.records.insert(
        call_id,
        Record {
            call,
            lane: Lane::Call,
            operation,
            controls,
            gate: Some(gate),
            resolved: None,
            phase: Phase::Received,
            events,
            outcome: None,
            cancels: Vec::new(),
            results: Vec::new(),
            responses: Vec::new(),
            accepted: false,
            response_view,
            result_delivered: false,
            response_delivered: false,
            in_table: false,
            alive: None,
        },
    );
    state.pending.push_back(call_id);
}

/// An `events.subscribe` call, on the reading thread (§4.2).
pub(super) fn read_events<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    events: EventsCall,
) -> Vec<Outgoing> {
    let operation_id = match &events {
        EventsCall::Read(read) => read.operation_id.clone(),
        EventsCall::End { operation_id, .. } => operation_id.clone(),
    };
    let log = state.operation(&operation_id).map(|owner| {
        state
            .records
            .get(&owner)
            .and_then(|record| record.events.clone())
    });
    let log = match log {
        Ok(Some(log)) => log,
        Ok(None) => {
            state.answered_on(call_id, Lane::Call);
            return vec![inner.refuse(
                call_id,
                Lane::Call,
                HostError::new(Code::InvalidRequest, "the call has no event log"),
            )];
        }
        Err(error) => {
            state.answered_on(call_id, Lane::Call);
            return vec![inner.refuse(call_id, Lane::Call, error)];
        }
    };
    match events {
        EventsCall::End {
            operation_id,
            stream_id,
        } => {
            state.answered_on(call_id, Lane::Call);
            vec![inner.reply(
                call_id,
                Lane::Call,
                inner.ports.events_ended(&operation_id, &stream_id),
            )]
        }
        EventsCall::Read(read) => {
            let batch = log.read(&read, inner.limits.read_bytes);
            let wait = inner.read_wait(read.wait);
            if !batch.records.is_empty() || batch.complete || wait.is_zero() {
                state.answered_on(call_id, Lane::Call);
                return vec![inner.reply(call_id, Lane::Call, inner.ports.events(&read, &batch))];
            }
            state.others.insert(call_id);
            state.held.push(Held {
                call_id,
                read,
                log,
                deadline: Instant::now() + wait,
            });
            inner.reads.notify();
            Vec::new()
        }
    }
}
