use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use gwz_session_contract::Lane;

use crate::detached::WorkerAlive;
use crate::service::events::{EventLog, Held};
use crate::service::{
    Call, Class, Cleanup, Code, Controls, Gate, HostError, HostPorts, Outcome, Resolved, View,
};

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
    pub(super) fn admits(self, class: Class) -> bool {
        match class {
            Class::Fetch => self.writes == 0,
            Class::Push => self.writes == 0 && self.pushes == 0,
            Class::Write => self.writes == 0 && self.pushes == 0 && self.fetches == 0,
        }
    }

    pub(super) fn change(&mut self, class: Class, up: bool) {
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
    pub(super) next_operation: u64,
    pub(crate) closing: bool,
    pub(crate) ended: bool,
    pub(super) outstanding: usize,
    pub(super) controls: usize,
    pub(crate) records: BTreeMap<u64, Record<P>>,
    /// Live and retained operations, by ID.
    pub(super) operations: HashMap<String, u64>,
    /// The `request_id`s of live operations (§4.3).
    pub(super) requests: HashMap<String, u64>,
    /// Live calls that are neither operations nor direct calls: reads,
    /// results and control calls.
    pub(super) others: HashSet<u64>,
    pub(crate) pending: VecDeque<u64>,
    pub(super) queue: VecDeque<u64>,
    pub(crate) direct_queue: VecDeque<u64>,
    pub(crate) direct_running: usize,
    pub(super) running: usize,
    pub(super) busy: HashMap<String, Busy>,
    pub(super) table_entries: usize,
    /// Terminal submitted records, oldest first, for eviction (§5.4).
    pub(super) retained: VecDeque<u64>,
    pub(crate) held: Vec<Held>,
    pub(crate) closers: Vec<u64>,
    pub(crate) report: Option<Cleanup>,
    pub(crate) close_started: bool,
    pub(super) cleanup_pending: u32,
    pub(super) cleanup_confirmed: bool,
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
    pub(super) fn operation(&self, operation_id: &str) -> Result<u64, HostError> {
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
    pub(super) fn evict(&mut self) -> bool {
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
    pub(super) fn discard(&mut self, call_id: u64) {
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
