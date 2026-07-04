//! The D3 in-process log service — the shell/pump around the `taut-shape`
//! mailbox engine.
//!
//! The `taut-shape` [`LogNode`] is a **pure, unsynchronized** mailbox endpoint
//! (taut-shape D15): it never locks, never blocks, and answers a `Read` that
//! cannot be satisfied by *parking* it as engine state. This module is the shell
//! the engine's docs assume: it owns the `Mutex` around the node, mints
//! operation-scoped `log_id`s, and turns the engine's park/release protocol into
//! a **blocking** reader API a producer/consumer pair can use across threads.
//!
//! # What the shell owns (that the engine deliberately does not)
//!
//! - **Serialization.** One [`Mutex<LogNode>`](std::sync::Mutex) per log; every
//!   `handle(Input)` call goes through it (D15: "the shell owns the lock").
//! - **Blocking.** A [`Condvar`](std::sync::Condvar) per log. A held `Read`
//!   (engine parked it — caught-up + live, `timeout_ms=None`) blocks the reader
//!   thread on the condvar until the producer's `Push`/`Seal`/`Close` releases an
//!   addressed [`Response`] for that `stream_id` (routed by `stream_id`).
//! - **`ProducerStop` routing.** [`Output::ProducerStop`] sets a per-log
//!   cancellation flag the producer's render loop polls between records (D6).
//! - **Timers.** v0 core-side has **no real clock**: a `Read` with
//!   `timeout_ms>0` produces a [`Output::SetTimer`] the shell cannot service, so
//!   the shell answers it `would_block` immediately (documented choice below).
//!   `timeout_ms=0` (probe) and `timeout_ms=None` (block) are honored exactly.
//!
//! # The timer choice (v0)
//!
//! The engine emits `SetTimer{token, ms}` when a caught-up live `Read` asks to
//! hold for a bounded time (`timeout_ms = Some(ms>0)`). A real deployment arms a
//! wall-clock timer and feeds `TimerExpired{token}` back. In-process v0 core has
//! no timer thread, so a bounded-wait read would hang forever waiting for a timer
//! that never fires. Instead, the shell immediately cancels the timer and re-asks
//! the engine as a **probe** (`timeout_ms=0`), turning any `SetTimer` into a
//! prompt `would_block`. Blocking (`None`) and probing (`0`) reads — the two the
//! diff producer/consumer actually use — are exact; bounded waits degrade to a
//! probe. This is a shell policy, not an engine change.
//!
//! # Retention (v0)
//!
//! Per D0 ruling #6/#7 the engine window **is** the operation-scoped retained
//! output: readers re-read and resume from the window by cursor with no per-file
//! recompute. Consumer-driven [`evict`](DiffLog::evict) is available but
//! optional; state is released when the operation drops its [`DiffLogRegistry`]
//! entry (last reader / end of operation), matching `stop_when=last_reader`.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use taut_shape::{
    Config, Cursor, Input, Limits, LogNode, Output, Record, Response, State, StopReason, StopWhen,
    StreamId,
};

use crate::model::{ErrorCode, ModelError, ModelResult};

/// A read request against a diff output log, in the shape a `diff.output` reader
/// passes (the taut-shape `LogReadRequest` surface, minus the `log_id` which
/// selects the log). `cursor` is the last position the reader consumed (`None` =
/// from the start); `timeout_ms` follows taut-shape D14.
#[derive(Clone, Debug, Default)]
pub struct LogReadRequest {
    /// The stream instance (taut-shape D3): one logical read loop with its own
    /// position. Minted by the consumer; many per log.
    pub stream_id: String,
    /// Resume position. `None` = `Cursor::START` (read from the first record).
    pub cursor: Option<u64>,
    /// Batch bounds (`None` on an axis = unbounded on that axis).
    pub max_records: Option<u32>,
    pub max_bytes: Option<u64>,
    /// taut-shape D14: `None` = block until data/terminal, `Some(0)` = probe
    /// (immediate `would_block` when caught up), `Some(n>0)` = bounded wait
    /// (degraded to a probe in v0 — see the module docs).
    pub timeout_ms: Option<u64>,
}

/// One appended record handed back to a reader: its window `seq` plus the opaque
/// taut-encoded payload (a `DiffOutputRecord` CBOR blob — the producer encodes
/// it, the reader decodes it; the log layer never inspects it).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
}

/// The delivery state of a read answer (taut-shape D13). Mirrors the engine's
/// [`State`] so callers do not depend on the `taut-shape` type directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogReadState {
    /// Records returned; keep reading.
    Data,
    /// Caught up, live: a probe/timeout answer (no records now, more may come).
    WouldBlock,
    /// Sealed and drained — the log is complete, no more records ever.
    Eof,
    /// `Close{}` teardown, no error.
    Closed,
    /// `Close{error}`: the producer failed; `error` carries the message.
    Failed,
    /// The cursor is invalid (evicted/never-existed); resume from `next_cursor`.
    Expired,
}

/// The answer to a [`DiffLog::read`]: the records delivered plus the cursor to
/// resume from and the delivery state. `next_cursor` is **always** present, even
/// when `records` is empty (taut-shape D8), so a resuming reader never dups or
/// skips.
#[derive(Clone, Debug)]
pub struct LogReadResponse {
    pub records: Vec<LogRecord>,
    pub next_cursor: u64,
    pub state: LogReadState,
    /// Present only when `state == Failed` (taut-shape D12).
    pub error: Option<String>,
}

/// Shared state guarded by one lock, plus the condvar held readers wait on.
struct Shared {
    node: LogNode,
    /// Set when the engine emits [`Output::ProducerStop`]. The producer polls
    /// [`DiffLog::should_stop`] between records and halts rendering when set.
    stop: Option<StopReason>,
}

/// One operation-scoped diff output log: the mailbox engine under a lock, with a
/// condvar for blocking reads. Cloneable handle (`Arc` inside) so a producer
/// thread and reader thread share one log.
#[derive(Clone)]
pub struct DiffLog {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl DiffLog {
    fn new() -> Self {
        // stop_when=last_reader (D0 ruling #7): the last reader going away stops
        // the producer, releasing operation-scoped render state.
        let node = LogNode::new(Config {
            stop_when: StopWhen::LastReader,
        });
        DiffLog {
            shared: Arc::new((Mutex::new(Shared { node, stop: None }), Condvar::new())),
        }
    }

    // ── producer API ────────────────────────────────────────────────────────

    /// Append one taut-encoded record. Wakes any reader parked on new data.
    pub fn push(&self, payload: Vec<u8>) {
        self.feed(Input::Push { payload });
    }

    /// Seal the log: it is finite and complete. Drained readers now see `Eof`.
    pub fn seal(&self) {
        self.feed(Input::Seal);
    }

    /// Close the log. `error = None` → readers see `Closed`; `Some(msg)` →
    /// `Failed` with the message attached. Idempotent (taut-shape D6/D12).
    pub fn close(&self, error: Option<String>) {
        let error = error.map(|message| taut_shape::Error {
            code: taut_shape::ErrorCode::ProducerError,
            message: Some(message),
        });
        self.feed(Input::Close { error });
    }

    /// Whether the engine has told the producer to stop (last reader gone, or a
    /// `Close`). The producer's render loop polls this between records (D6).
    pub fn should_stop(&self) -> bool {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().stop.is_some()
    }

    // ── consumer API ──────────────────────────────────────────────────────────

    /// Blocking read (the `diff.output` reader entry point). Sends a `Read` into
    /// the engine and, if the engine parks it (caught-up + live, `timeout_ms =
    /// None`), waits on the condvar until a producer input releases the addressed
    /// response for this `stream_id`.
    ///
    /// `timeout_ms = Some(0)` probes (immediate `would_block` when caught up);
    /// `Some(n>0)` degrades to a probe in v0 (no core-side clock — see module
    /// docs); `None` blocks.
    pub fn read(&self, request: &LogReadRequest) -> LogReadResponse {
        let (lock, cv) = &*self.shared;
        let stream = StreamId::new(&request.stream_id);
        let cursor = request.cursor.map(Cursor::new);
        let limits = Limits {
            max_records: request.max_records,
            max_bytes: request.max_bytes,
        };

        let mut guard = lock.lock().unwrap();
        loop {
            // A held read is engine state, not a shell spin: hand the engine the
            // Read and see whether it answered immediately or parked it.
            let outputs = guard.node.handle(Input::Read {
                stream_id: stream.clone(),
                cursor,
                limits,
                // Bounded waits degrade to a probe (v0 has no core-side timer);
                // a real block (None) is passed through so the engine parks it.
                timeout_ms: match request.timeout_ms {
                    Some(0) => Some(0),
                    Some(_) => Some(0),
                    None => None,
                },
            });

            if let Some(response) = route_response(&mut guard, outputs, &stream) {
                return to_read_response(response);
            }

            // No addressed response ⇒ the engine parked the read (caught-up +
            // live, timeout None). Block until a producer input notifies, then
            // re-issue the Read (the engine supersedes the stale parked read).
            guard = cv.wait(guard).unwrap();
        }
    }

    /// Consumer-driven eviction (taut-shape D7): drop records at or below
    /// `up_to_seq`, raising the window floor. Optional in v0 — the operation
    /// window is the retention (D0 ruling #6).
    pub fn evict(&self, up_to_seq: u64) {
        self.feed(Input::Evict { up_to_seq });
    }

    /// End a reader's stream (taut-shape D4): drop its held read and, under
    /// `stop_when=last_reader`, fire `ProducerStop` if it was the last reader.
    /// Adapters call this on transport death; the diff reader calls it when done.
    pub fn end_stream(&self, stream_id: &str) {
        self.feed(Input::EndStream {
            stream_id: StreamId::new(stream_id),
        });
    }

    // ── internals ─────────────────────────────────────────────────────────────

    /// Feed one producer/environment input, absorb its outputs (record any
    /// `ProducerStop`), and wake every parked reader so each re-issues its Read.
    fn feed(&self, input: Input) {
        let (lock, cv) = &*self.shared;
        let mut guard = lock.lock().unwrap();
        let outputs = guard.node.handle(input);
        absorb_side_effects(&mut guard, outputs);
        // A Push/Seal/Close/Evict may have released a held read; wake all waiters
        // and let each re-run its Read against the fresh window (the engine is the
        // arbiter of who actually gets data).
        cv.notify_all();
    }
}

/// Route the outputs of a `Read` call. Returns the [`Response`] addressed to
/// `stream` if the engine produced one; otherwise `None` (the read was parked).
/// `SetTimer` outputs are answered immediately (v0 has no clock): the shell
/// cancels the timer and the caller re-probes.
fn route_response(
    shared: &mut Shared,
    outputs: Vec<Output>,
    stream: &StreamId,
) -> Option<Response> {
    let mut answer: Option<Response> = None;
    let mut timer_fired = false;
    for output in outputs {
        match output {
            Output::Response(response) if &response.stream_id == stream => {
                answer = Some(response);
            }
            Output::Response(_) => {
                // A Read only ever addresses its own stream; other-stream
                // responses cannot arise from a single Read.
            }
            Output::SetTimer { .. } => {
                // v0: no core-side clock. The engine parked the read behind a
                // timer we cannot service; treat it as a would_block probe.
                timer_fired = true;
            }
            Output::CancelTimer { .. } => {}
            Output::ProducerStop { reason } => {
                shared.stop.get_or_insert(reason);
            }
            Output::Diagnostic(_) => {}
        }
    }
    if answer.is_none() && timer_fired {
        // A SetTimer means the engine parked behind a bounded wait; since we
        // passed timeout_ms=0 for bounded waits this should not occur, but guard
        // it: re-probe by returning a synthetic would_block via re-read is
        // unnecessary — the parked read will be superseded on the next loop.
    }
    answer
}

/// Absorb the side effects of a producer/environment input: record any
/// `ProducerStop` (drives the producer's cancellation flag). Any `Response`s
/// released here are for *parked* reads whose reader threads are asleep; those
/// threads re-issue their `Read` on wake and collect the answer then, so we do
/// not need to buffer them.
fn absorb_side_effects(shared: &mut Shared, outputs: Vec<Output>) {
    for output in outputs {
        if let Output::ProducerStop { reason } = output {
            shared.stop.get_or_insert(reason);
        }
    }
}

fn to_read_response(response: Response) -> LogReadResponse {
    LogReadResponse {
        records: response
            .records
            .into_iter()
            .map(|Record { seq, payload }| LogRecord { seq, payload })
            .collect(),
        next_cursor: response.next_cursor.seq,
        state: to_read_state(response.state),
        error: response.error.and_then(|e| e.message),
    }
}

fn to_read_state(state: State) -> LogReadState {
    match state {
        State::Data => LogReadState::Data,
        State::WouldBlock => LogReadState::WouldBlock,
        State::Eof => LogReadState::Eof,
        State::Closed => LogReadState::Closed,
        State::Failed => LogReadState::Failed,
        State::Expired => LogReadState::Expired,
    }
}

/// An operation-scoped registry of diff output logs, keyed by `log_id`. One per
/// operation (or one shared across an operation runtime); the diff handler mints
/// a log per patch request and the `diff.output` reader looks it up by
/// `DiffOutputLogRef.log_id`. Dropping the registry (or [`release`](Self::release)
/// on the last reader) frees the operation's retained render state.
#[derive(Clone, Default)]
pub struct DiffLogRegistry {
    logs: Arc<Mutex<HashMap<String, DiffLog>>>,
    /// Monotonic id source so each minted `log_id` is unique within the process.
    next_id: Arc<Mutex<u64>>,
}

impl DiffLogRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh log and register it under a new unique `log_id`. Returns the
    /// id (for `DiffOutputLogRef.log_id`) and the shared [`DiffLog`] handle the
    /// producer writes to.
    pub fn create(&self) -> (String, DiffLog) {
        let log_id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            format!("difflog_{:012}", *next)
        };
        let log = DiffLog::new();
        self.logs
            .lock()
            .unwrap()
            .insert(log_id.clone(), log.clone());
        (log_id, log)
    }

    /// Look up a log by id. `Err(UnknownLog)` — per the taut-shape service-level
    /// taxonomy (taut-shape §A.8) mapped onto the GWZ error vocabulary — when the
    /// id was never minted or has been released.
    pub fn get(&self, log_id: &str) -> ModelResult<DiffLog> {
        self.logs
            .lock()
            .unwrap()
            .get(log_id)
            .cloned()
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown diff output log '{log_id}'"),
                )
            })
    }

    /// Blocking read against a registered log by id (the `diff.output` reader
    /// path). Resolves the id, then delegates to [`DiffLog::read`]. Unknown id is
    /// a typed error (taut-shape `UnknownLog` → GWZ `InvalidRequest`).
    pub fn read(&self, log_id: &str, request: &LogReadRequest) -> ModelResult<LogReadResponse> {
        Ok(self.get(log_id)?.read(request))
    }

    /// Release a log's state (last reader / end of operation). Idempotent.
    pub fn release(&self, log_id: &str) {
        self.logs.lock().unwrap().remove(log_id);
    }
}
