//! Workers and closure (§5.2, §8).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | An operation gets its own worker thread when it starts; one that cannot be created is settled `Failed` (`internal_error`) before it starts, with no effect | §5.2 |
//! | Direct calls run on a pool of direct workers, separate from the running limit, each taking the next waiting direct call when it finishes | §5.2 |
//! | A handler's panic is caught and reported `Failed` (`internal_error`); the session carries on | §5.2 |
//! | A worker reports its terminal through its gate, once; after revocation the report is ignored | O8, §5.6 |
//! | Close: refuse later calls; settle what has not started; cancel what runs; wait at most `close_wait`; revoke, settle and record what still runs as detached; answer what is outstanding; release the spools; answer the close with the report; close the channel | §8 |

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use gwz_session_contract::Lane;

use super::table::{self, Phase, State, cancelled_before_start, close_report, settle};
use super::{
    Call, Cleanup, Code, HostError, HostPorts, Inner, Outcome, Outgoing, Resolved, Terminal, Worker,
};
use crate::detached::WorkerAlive;
use crate::lock;

/// What a worker thread needs, taken from its record under the table lock.
struct Start<P: HostPorts> {
    call_id: u64,
    call: Call,
    resolved: Resolved,
    worker: Worker<P::Session>,
    alive: WorkerAlive,
}

/// Takes a record's gate and marks it running.
fn take<P: HostPorts>(state: &mut State<P>, call_id: u64) -> Option<Start<P>> {
    let record = state.records.get_mut(&call_id)?;
    let gate = record.gate.take()?;
    let resolved = record.resolved.clone()?;
    let alive = WorkerAlive::new();
    record.phase = Phase::Running;
    record.alive = Some(alive.clone());
    let token = gate.token().clone();
    Some(Start {
        call_id,
        call: record.call.clone(),
        resolved,
        worker: Worker {
            gate: Mutex::new(gate),
            token,
            events: record.events.clone(),
            operation_id: record.operation.as_ref().map(|(id, _)| id.clone()),
        },
        alive,
    })
}

/// Runs a call on this thread and reports its terminal through its gate.
fn run<P: HostPorts>(inner: &Arc<Inner<P>>, start: Start<P>) {
    let Start {
        call_id,
        call,
        resolved,
        worker,
        alive,
    } = start;
    let ran = catch_unwind(AssertUnwindSafe(|| {
        inner.ports.run(&call, &resolved, &worker)
    }));
    let outcome = ran.unwrap_or_else(|_| {
        let failure = inner.ports.failure(HostError::new(
            Code::Internal,
            "the handler panicked; its cleanup is unconfirmed",
        ));
        let result = worker.operation_id().map_or_else(Vec::new, |id| {
            inner.ports.settled_result(&call, id, &failure)
        });
        Outcome {
            terminal: Terminal::Failed(failure),
            result,
            cleanup: Cleanup::default(),
        }
    });
    let reported = lock(&worker.gate).report(|_| ()).is_some();
    if reported {
        let outgoing = {
            let mut state = lock(&inner.state);
            let mut outgoing = settle(inner, &mut state, call_id, outcome);
            outgoing.extend(table::schedule(inner, &mut state));
            outgoing
        };
        inner.flush(outgoing);
    }
    drop(worker);
    drop(alive);
}

/// Starts a queued operation on a thread of its own (§5.2).
pub(crate) fn start_operation<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
) -> Vec<Outgoing> {
    let Some(start) = take(state, call_id) else {
        return Vec::new();
    };
    let thread_inner = Arc::clone(inner);
    let spawned = thread::Builder::new()
        .name("gwz-session-operation".into())
        .spawn(move || run(&thread_inner, start));
    match spawned {
        Ok(_) => Vec::new(),
        Err(error) => spawn_failed(inner, state, call_id, &error),
    }
}

/// Starts a direct worker, which runs waiting direct calls until none waits
/// (§5.2). The caller has counted it in `direct_running`.
pub(crate) fn start_direct<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
) -> Vec<Outgoing> {
    let Some(first) = next_direct(state) else {
        state.direct_running = state.direct_running.saturating_sub(1);
        return Vec::new();
    };
    let call_id = first.call_id;
    let thread_inner = Arc::clone(inner);
    let spawned = thread::Builder::new()
        .name("gwz-session-direct".into())
        .spawn(move || {
            let mut next = Some(first);
            while let Some(start) = next {
                run(&thread_inner, start);
                let mut state = lock(&thread_inner.state);
                next = next_direct(&mut state);
                if next.is_none() {
                    state.direct_running = state.direct_running.saturating_sub(1);
                }
            }
        });
    match spawned {
        Ok(_) => Vec::new(),
        Err(error) => {
            state.direct_running = state.direct_running.saturating_sub(1);
            spawn_failed(inner, state, call_id, &error)
        }
    }
}

fn next_direct<P: HostPorts>(state: &mut State<P>) -> Option<Start<P>> {
    while let Some(call_id) = state.direct_queue.pop_front() {
        if let Some(start) = take(state, call_id) {
            return Some(start);
        }
    }
    None
}

fn spawn_failed<P: HostPorts>(
    inner: &Arc<Inner<P>>,
    state: &mut State<P>,
    call_id: u64,
    error: &std::io::Error,
) -> Vec<Outgoing> {
    let failure = inner.ports.failure(HostError::new(
        Code::Internal,
        format!("no worker thread could be created: {error}"),
    ));
    let result = state
        .records
        .get(&call_id)
        .and_then(|record| {
            record
                .operation
                .as_ref()
                .map(|(id, _)| inner.ports.settled_result(&record.call, id, &failure))
        })
        .unwrap_or_default();
    settle(
        inner,
        state,
        call_id,
        Outcome {
            terminal: Terminal::Failed(failure),
            result,
            cleanup: Cleanup::default(),
        },
    )
}

/// Closes the session (§8) on a thread of its own: `session.close` when
/// `closer` names its call, channel closure otherwise. A second close while
/// one runs only waits for its report.
pub(crate) fn close<P: HostPorts>(inner: &Arc<Inner<P>>, closer: Option<u64>) {
    {
        let mut state = lock(&inner.state);
        if state.close_started {
            return;
        }
        state.close_started = true;
        state.closing = true;
    }
    let thread_inner = Arc::clone(inner);
    let spawned = thread::Builder::new()
        .name("gwz-session-close".into())
        .spawn(move || run_close(&thread_inner, closer.is_some()));
    if spawned.is_err() {
        run_close(inner, closer.is_some());
    }
}

fn run_close<P: HostPorts>(inner: &Arc<Inner<P>>, replies: bool) {
    // Steps 2 and 3: settle what has not started, cancel what runs.
    let outgoing = {
        let mut state = lock(&inner.state);
        state.pending.clear();
        let mut outgoing = Vec::new();
        for call_id in state.not_started() {
            if let Some(record) = state.records.get(&call_id) {
                record.controls.cancel();
                let outcome =
                    cancelled_before_start(inner, &record.call, record.operation.as_ref());
                outgoing.extend(settle(inner, &mut state, call_id, outcome));
            }
        }
        state.direct_queue.clear();
        for call_id in state.running() {
            if let Some(record) = state.records.get(&call_id) {
                record.controls.cancel();
            }
        }
        outgoing
    };
    inner.flush(outgoing);
    // Step 4: wait for the workers, up to the close bound.
    let deadline = Instant::now() + inner.limits.close_wait;
    let mut state = lock(&inner.state);
    while !state.running().is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        state = inner
            .settled
            .wait_timeout(state, left)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0;
    }
    // Step 5: revoke, settle and record what still runs as detached.
    let mut outgoing = Vec::new();
    for call_id in state.running() {
        let Some(record) = state.records.get(&call_id) else {
            continue;
        };
        record.controls.revoke();
        if let (Some(alive), Some(resolved)) = (record.alive.as_ref(), record.resolved.as_ref()) {
            inner.detached.record(&resolved.workspace, alive);
        }
        let operation_id = record.operation.as_ref().map(|(id, _)| id.clone());
        let (failure, result) = inner.ports.detached(&record.call, operation_id.as_deref());
        state.detached_count = state.detached_count.saturating_add(1);
        outgoing.extend(settle(
            inner,
            &mut state,
            call_id,
            Outcome {
                terminal: Terminal::Cancelled(failure),
                result,
                cleanup: Cleanup::default(),
            },
        ));
    }
    // Step 6: held reads return what remains.
    for held in std::mem::take(&mut state.held) {
        let batch = held.log.read(&held.read, inner.limits.read_bytes);
        let body = inner.ports.events(&held.read, &batch);
        state.answered(held.call_id);
        outgoing.push(inner.reply(held.call_id, Lane::Call, body));
    }
    drop(state);
    inner.flush(outgoing);
    // Step 7: the session's spools.
    inner.ports.release_spools(&inner.session);
    // Step 8: the close report, to every `session.close` waiting for it.
    let outgoing = {
        let mut state = lock(&inner.state);
        let report = state.close_report();
        state.report = Some(report);
        let closers = std::mem::take(&mut state.closers);
        let mut outgoing = Vec::new();
        if replies {
            for call_id in closers {
                state.answered_on(call_id, Lane::Control);
                outgoing.push(close_report(inner, call_id, report));
            }
        }
        outgoing
    };
    inner.flush(outgoing);
    // Step 9: the channel closes after the replies above were handed over.
    inner.sink.close_channel();
    let mut state = lock(&inner.state);
    state.ended = true;
    drop(state);
    inner.settled.notify_all();
    inner.admission.notify_all();
    inner.reads.notify();
}
