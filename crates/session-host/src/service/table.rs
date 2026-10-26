//! The session's table: records at receipt, admission, the classes and the
//! queue, cancellation, results and release (§4.2, §5.1, §5.3, §5.4).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use gwz_session_contract::Lane;

use super::events::{EventLog, Held};
use super::workers;
use super::{
    Call, CancelTarget, Class, Cleanup, Code, Controls, EventsCall, Gate, HostError, HostPorts,
    Inner, Mode, Outcome, Outgoing, ProtocolError, Request, Resolved, Terminal, View,
};
use crate::detached::WorkerAlive;
use crate::gate::CallControls;
use crate::lock;

/// Where a record is (§5.1, §5.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    /// Read, not yet admitted.
    Received,
    /// An admitted operation waiting to start.
    Queued,
    /// A direct call waiting for a direct worker.
    DirectWaiting,
    /// Its worker runs.
    Running,
    /// Settled.
    Terminal,
}

/// A direct or operation call's record (O7).
pub(crate) struct Record<P: HostPorts> {
    pub(crate) call: Call,
    pub(crate) lane: Lane,
    /// The operation's ID and class; `None` for a direct call.
    pub(crate) operation: Option<(String, Class)>,
    pub(crate) controls: Controls<P>,
    pub(crate) gate: Option<Gate<P>>,
    pub(crate) resolved: Option<Resolved>,
    pub(crate) phase: Phase,
    pub(crate) events: Option<Arc<EventLog>>,
    pub(crate) outcome: Option<Outcome<P::Failure>>,
    /// Waiters: `operation.cancel`, `operation.result` and
    /// `operation.response` calls answered at the terminal.
    pub(crate) cancels: Vec<u64>,
    pub(crate) results: Vec<u64>,
    pub(crate) responses: Vec<u64>,
    /// Whether a submit's `accepted` reply was sent.
    pub(crate) accepted: bool,
    /// Whether the method declares a response view beside its result.
    pub(crate) response_view: bool,
    /// Whether each view of the terminal was handed over (§5.4): a record
    /// is delivered once every view its method declares has been.
    pub(crate) result_delivered: bool,
    pub(crate) response_delivered: bool,
    pub(crate) in_table: bool,
    /// The running worker's marker, for the host's detached record, until
    /// the record settles: the worker's thread then owns the only strong
    /// marker, and the detached record holds a weak one.
    pub(crate) alive: Option<WorkerAlive>,
}

impl<P: HostPorts> Record<P> {
    /// Whether every view the method declares has been handed over (§5.4).
    fn delivered(&self) -> bool {
        self.result_delivered && (!self.response_view || self.response_delivered)
    }
}

/// How many operations of each class run on one workspace.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Busy {
    fetches: usize,
    pushes: usize,
    writes: usize,
}

impl Busy {
    fn admits(self, class: Class) -> bool {
        match class {
            Class::Fetch => self.writes == 0,
            Class::Push => self.writes == 0 && self.pushes == 0,
            Class::Write => self.writes == 0 && self.pushes == 0 && self.fetches == 0,
        }
    }

    fn change(&mut self, class: Class, up: bool) {
        let count = match class {
            Class::Fetch => &mut self.fetches,
            Class::Push => &mut self.pushes,
            Class::Write => &mut self.writes,
        };
        if up {
            *count += 1;
        } else {
            *count = count.saturating_sub(1);
        }
    }
}

/// The table: everything the table lock guards.
pub(crate) struct State<P: HostPorts> {
    pub(crate) highest_call: u64,
    next_operation: u64,
    pub(crate) closing: bool,
    pub(crate) ended: bool,
    outstanding: usize,
    controls: usize,
    pub(crate) records: BTreeMap<u64, Record<P>>,
    /// Live and retained operations, by ID.
    operations: HashMap<String, u64>,
    /// The `request_id`s of live operations (§4.3).
    requests: HashMap<String, u64>,
    /// Live calls that are neither operations nor direct calls: reads,
    /// results and control calls.
    others: HashSet<u64>,
    pub(crate) pending: VecDeque<u64>,
    queue: VecDeque<u64>,
    pub(crate) direct_queue: VecDeque<u64>,
    pub(crate) direct_running: usize,
    running: usize,
    busy: HashMap<String, Busy>,
    table_entries: usize,
    /// Terminal submitted records, oldest first, for eviction (§5.4).
    retained: VecDeque<u64>,
    pub(crate) held: Vec<Held>,
    pub(crate) closers: Vec<u64>,
    pub(crate) report: Option<Cleanup>,
    pub(crate) close_started: bool,
    cleanup_pending: u32,
    cleanup_confirmed: bool,
    pub(crate) detached_count: u32,
    /// Reply batches built under this lock and not yet handed to the sink.
    /// Close waits for zero before its report and the channel's close.
    pub(crate) unflushed: usize,
}

impl<P: HostPorts> Default for State<P> {
    fn default() -> Self {
        Self {
            highest_call: 0,
            next_operation: 1,
            closing: false,
            ended: false,
            outstanding: 0,
            controls: 0,
            records: BTreeMap::new(),
            operations: HashMap::new(),
            requests: HashMap::new(),
            others: HashSet::new(),
            pending: VecDeque::new(),
            queue: VecDeque::new(),
            direct_queue: VecDeque::new(),
            direct_running: 0,
            running: 0,
            busy: HashMap::new(),
            table_entries: 0,
            retained: VecDeque::new(),
            held: Vec::new(),
            closers: Vec::new(),
            report: None,
            close_started: false,
            cleanup_pending: 0,
            cleanup_confirmed: false,
            detached_count: 0,
            unflushed: 0,
        }
    }
}

/// Whether `request_id` names a live operation of the session (§4.3).
pub(crate) fn is_live<P: HostPorts>(state: &State<P>, request_id: &str) -> bool {
    state.requests.contains_key(request_id)
}

/// Marks the views whose replies were handed over (§5.4).
pub(crate) fn mark_delivered<P: HostPorts>(state: &mut State<P>, delivered: &[(u64, View)]) {
    for (call_id, view) in delivered {
        if let Some(record) = state.records.get_mut(call_id) {
            match view {
                View::Result => {
                    record.result_delivered = true;
                }
                View::Response => {
                    record.response_delivered = true;
                }
            }
        }
    }
}

impl<P: HostPorts> State<P> {
    /// A reply to a call on `lane` has been sent: the call is no longer
    /// outstanding.
    pub(crate) fn answered_on(&mut self, call_id: u64, lane: Lane) {
        self.others.remove(&call_id);
        match lane {
            Lane::Call => {
                self.outstanding = self.outstanding.saturating_sub(1);
            }
            Lane::Control => {
                self.controls = self.controls.saturating_sub(1);
            }
        }
    }

    /// A held read's reply has been sent.
    pub(crate) fn answered(&mut self, call_id: u64) {
        self.answered_on(call_id, Lane::Call);
    }

    /// Whether `operation_id` is one this session issued.
    fn issued(&self, operation_id: &str) -> bool {
        operation_id
            .strip_prefix("op-")
            .and_then(|serial| serial.parse::<u64>().ok())
            .is_some_and(|serial| serial >= 1 && serial < self.next_operation)
    }

    /// The call that created `operation_id`'s record, or the refusal for an
    /// operation the session never issued or no longer holds.
    fn operation(&self, operation_id: &str) -> Result<u64, HostError> {
        match self.operations.get(operation_id) {
            Some(call_id) => Ok(*call_id),
            None if self.issued(operation_id) => Err(HostError::new(
                Code::OperationExpired,
                format!("operation {operation_id} was released or evicted"),
            )),
            None => Err(HostError::new(
                Code::OperationNotFound,
                format!("this session issued no operation {operation_id}"),
            )),
        }
    }

    /// The running operations and direct calls.
    pub(crate) fn running(&self) -> Vec<u64> {
        self.records
            .iter()
            .filter(|(_, record)| record.phase == Phase::Running)
            .map(|(call_id, _)| *call_id)
            .collect()
    }

    /// The calls that have not started: received, queued, or waiting for a
    /// direct worker.
    pub(crate) fn not_started(&self) -> Vec<u64> {
        self.records
            .iter()
            .filter(|(_, record)| {
                matches!(
                    record.phase,
                    Phase::Received | Phase::Queued | Phase::DirectWaiting
                )
            })
            .map(|(call_id, _)| *call_id)
            .collect()
    }

    /// The close report (§8 step 8).
    pub(crate) fn close_report(&self) -> Cleanup {
        Cleanup {
            pending_local_work: self.cleanup_pending.saturating_add(self.detached_count),
            peer_cleanup_confirmed: self.cleanup_confirmed && self.detached_count == 0,
        }
    }

    /// Evicts the oldest delivered terminal record, if there is one (§5.4).
    fn evict(&mut self) -> bool {
        let Some(index) = self.retained.iter().position(|call_id| {
            self.records
                .get(call_id)
                .is_some_and(|record| record.delivered())
        }) else {
            return false;
        };
        if let Some(call_id) = self.retained.remove(index) {
            self.discard(call_id);
        }
        true
    }

    /// Removes a record and what indexes it.
    fn discard(&mut self, call_id: u64) {
        if let Some(record) = self.records.remove(&call_id) {
            if let Some((operation_id, _)) = record.operation.as_ref() {
                self.operations.remove(operation_id);
            }
            if record.in_table {
                self.table_entries = self.table_entries.saturating_sub(1);
            }
            self.retained.retain(|retained| *retained != call_id);
        }
    }
}

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
fn create<P: HostPorts>(
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
fn read_events<P: HostPorts>(
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

/// `operation.result` (`result` true) or `operation.response` (§4.2).
fn view<P: HostPorts>(
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
fn view_reply<P: HostPorts>(
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

/// `operation.cancel`, on the reading thread (§4.2, §5.3).
fn cancel<P: HostPorts>(
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

/// `operation.release` (§4.2, §5.4).
fn release<P: HostPorts>(
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
fn admit<P: HostPorts>(
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
fn failed_before_start<P: HostPorts>(
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
