//! Nonblocking endpoint-side bridge for the v2 placement mux.
//!
//! The bridge owns only request state and bounded message attachments. Physical
//! SSH work remains in [`ssh_worker::Endpoint`], whose worker thread continues
//! pumping every attached exchange while this object is stepped by the host.

use super::{
    agent_job::{self, Job},
    setup_retry::{self, AllocationClock, Decision, Jitter, Operations, Outcome, Phase},
    ssh_channel::GitService as NativeService,
    ssh_worker::{BridgeContext, Endpoint, EndpointAttachment, PendingOpen},
};
use gwz_transport::{
    pool::{Capacity, Key},
    protocol::{
        Destination, Effect, Envelope, ErrorCode, Failure, GitService, Identity, IdentityMode,
        MessageKind, Opened,
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::{Path, PathBuf},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

mod admission;
mod attachments;
mod cancel;
mod completion;
mod outbound;

use outbound::envelope_for;

const MAX_REQUESTS: usize = 64;
const MAX_QUEUED_INPUT: usize = 16;
const MAX_OUTBOUND: usize = 64;
type RequestKey = (String, i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointError {
    InvalidRequest,
    Capacity,
    Duplicate,
    Shutdown,
    Protocol,
    WouldBlock,
}

struct Request {
    stream_id: i64,
    session_id: String,
    version: i64,
    attachment: Option<EndpointAttachment>,
    queued_input: VecDeque<Envelope>,
    terminal: bool,
    /// The facts of this member's setup attempts so far, which its one reply
    /// carries: progress on its diagnostic row (the retry plan's §5).
    facts: Option<gwz_transport::protocol::Facts>,
}
struct OpenJob {
    key: RequestKey,
    pool_key: Key,
    reply: PendingOpen,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Option<u64>,
    abandoned: bool,
    /// The Open as it arrived, which a retried attempt starts from again.
    envelope: Envelope,
}
struct QueuedOpen {
    key: RequestKey,
    pool_key: Key,
    envelope: Envelope,
    /// Its allocation clock, which stops while its key holds it.
    allocation: AllocationClock,
}
struct CheckJob {
    key: RequestKey,
    job: Job<()>,
    deadline: u64,
    cancelled: bool,
}
pub(crate) struct Outbound {
    pub(crate) request: String,
    pub(crate) envelope: Envelope,
}

/// Endpoint-local runtime driven by one host supervisor thread.
pub(crate) struct PlacementEndpoint {
    endpoint: Endpoint,
    home: PathBuf,
    endpoint_id: String,
    trust_owner: String,
    now_ms: u64,
    requests: BTreeMap<RequestKey, Request>,
    opens: Vec<OpenJob>,
    queued_opens: VecDeque<QueuedOpen>,
    checks: Vec<CheckJob>,
    outbound: VecDeque<Outbound>,
    terminal_outbound: VecDeque<Outbound>,
    shutting_down: bool,
    faulted: bool,
    /// The host's waker, which each bridge wakes when it has a message.
    waker: Option<Waker>,
    /// Each operation's setup retry machines, by pool key (the retry plan's
    /// §5); an attempt's member is its request key.
    retries: Operations<Key, RequestKey>,
    jitter: Jitter,
}
impl Drop for PlacementEndpoint {
    fn drop(&mut self) {
        self.shutdown();
    }
}
impl PlacementEndpoint {
    pub(crate) fn pool(&self) -> &gwz_transport::pool::Pool {
        self.endpoint.pool()
    }
    pub(crate) fn set_request_capacity(&self, capacity: usize) {
        self.endpoint.set_request_capacity(capacity);
    }
    pub(crate) fn new(
        endpoint: Endpoint,
        home: PathBuf,
        endpoint_id: String,
        trust_owner: String,
    ) -> io::Result<Self> {
        if endpoint_id.is_empty()
            || trust_owner.is_empty()
            || !home.is_absolute()
            || home.to_str().is_none_or(|value| value.contains('\0'))
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            endpoint,
            home,
            endpoint_id,
            trust_owner,
            now_ms: 0,
            requests: BTreeMap::new(),
            opens: Vec::new(),
            queued_opens: VecDeque::new(),
            checks: Vec::new(),
            outbound: VecDeque::new(),
            terminal_outbound: VecDeque::new(),
            shutting_down: false,
            faulted: false,
            waker: None,
            retries: Operations::new(),
            jitter: Jitter::random(),
        })
    }
    /// The operation's `--max-retries`, which its admission installs before
    /// its first open. An operation never given one retries three times.
    pub(crate) fn set_max_retries(&mut self, request: &str, max_retries: u32) {
        self.retries.set_max_retries(request, max_retries);
    }

    /// Advance bounded checks, open completions, and each live message bridge.
    pub(crate) fn step(&mut self, now_ms: u64, cx: &mut Context<'_>) -> Result<(), EndpointError> {
        self.now_ms = self.now_ms.max(now_ms);
        self.waker = Some(cx.waker().clone());
        let now_ms = self.now_ms;
        self.finish_checks(now_ms, cx);
        self.finish_opens(now_ms);
        self.start_queued(now_ms)?;
        self.flush_attachments();
        if self.faulted {
            Err(EndpointError::Capacity)
        } else {
            Ok(())
        }
    }

    pub(crate) fn pending_request(&self, request: &str) -> bool {
        self.requests.keys().any(|key| key.0 == request)
            || self.opens.iter().any(|job| job.key.0 == request)
            || self.checks.iter().any(|job| job.key.0 == request)
    }
    pub(crate) fn pending_request_count(&self, request: &str) -> usize {
        self.requests.keys().filter(|key| key.0 == request).count()
            + self.opens.iter().filter(|job| job.key.0 == request).count()
            + self
                .checks
                .iter()
                .filter(|job| job.key.0 == request)
                .count()
    }
    pub(crate) fn pending(&self) -> usize {
        self.requests.len()
            + self.opens.len()
            + self.checks.len()
            + self.outbound.len()
            + self.terminal_outbound.len()
            + self.endpoint.pending_requests()
            + self.endpoint.shutdown_status().pending_connections
    }
    fn now(&self) -> u64 {
        self.now_ms
    }
}

fn request_state(envelope: &Envelope) -> Request {
    Request {
        stream_id: envelope.stream_id,
        session_id: envelope.session_id.clone(),
        version: envelope.version,
        attachment: None,
        queued_input: VecDeque::new(),
        terminal: false,
        facts: None,
    }
}
/// The most opens in flight across every host: the pool's total and request
/// ceilings, the endpoint's `MAX_REQUESTS`, and half the process-wide budget
/// of supervised jobs. An open holds no job while it waits for the worker and
/// runs one at a time, its key read or its setup, so the other half stays for
/// identity checks and for other endpoints.
fn open_ceiling(capacity: Capacity) -> usize {
    capacity
        .total
        .min(capacity.max_requests)
        .min(MAX_REQUESTS)
        .min(agent_job::LIMIT / 2)
}
/// An open's failure, and whether a started setup failed: only the worker's
/// typed failure says so; every other refusal ends the open before a setup.
fn open_failure(error: &io::Error) -> (Failure, Phase) {
    if let Some(failure) = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<super::ssh_worker::EndpointOpenFailure>())
    {
        return (failure.failure.clone(), failure.phase);
    }
    let code = match error.kind() {
        io::ErrorKind::TimedOut => ErrorCode::Timeout,
        io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound => ErrorCode::Unavailable,
        io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
        io::ErrorKind::WouldBlock => ErrorCode::Capacity,
        io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe => ErrorCode::CarrierLost,
        _ => ErrorCode::Io,
    };
    let failure = Failure {
        setup_cause: None,
        code,
        effect: Effect::None,
        facts: None,
    };
    (failure, Phase::Other)
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl PlacementEndpoint {
            pub(crate) fn set_jitter(&mut self, jitter: Jitter) {
                self.jitter = jitter;
            }
            /// The attempts that have started and not yet ended.
            pub(crate) fn attempts_in_flight_for_test(&self) -> usize {
                self.opens.len()
            }
        }
        #[path = "placement_endpoint_tests.rs"]
        mod check_tests;
        mod retry_tests;
    }
}
