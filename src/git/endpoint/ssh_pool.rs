//! Physical ownership behind the transport pool's exclusive lease ledger.
//! One endpoint worker drives this host and its timer independently of Git calls.
use super::ssh_setup_context::SetupContext;
use super::{ssh_handoff::UrlExtras, ssh_key_snapshot::Entry as Selected};
use gwz_transport::{
    pool::{Action, Config, ConnectionId, Error, Identity, Key, Lease, Pool, PoolDriver},
    protocol::{Disposition, Effect, ErrorCode, Facts, Failure},
};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
};
pub(crate) type Progress = Arc<Mutex<Facts>>;

/// What the host saw happen to a connection (adaptive concurrency design
/// §4.1's states). Each is reported at the moment the host acts, which is when
/// the server can first be affected, and in the order the host acted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// The connector began the socket connect. `clocked` is false when the
    /// connect has no deadline.
    Started { clocked: bool },
    /// The socket connect completed after `ms` of TCP connecting, a fact of
    /// the host alone: the settle time `Ts` follows it (§4.1), not the longer
    /// setup (a login, a first exchange) that comes after.
    TcpConnected { ms: u64 },
    /// The resource connected, as the pool counts it. (HTTPS is not set up
    /// until its first exchange is answered, which the endpoint reports.)
    Connected,
    /// The connect failed: the server refused or dropped it, or the host
    /// could not finish it. Nothing remains for the server to count.
    SetupEnded,
    /// The client cancelled the connect and has disposed of it.
    Retired,
    /// The host began to dispose of the connection.
    Closing,
    /// The host has disposed of it.
    Disposed,
    /// The server closed an idle connection and the host noticed.
    ServerClosed,
}

/// Told, by the one thread that drives a pool host, of every connection's
/// state change in the order it happened (F10). An observer must not call
/// back into the host.
pub(crate) trait Observer: Send + Sync {
    fn seen(&self, key: &Key, connection: ConnectionId, seen: Seen, now: u64);
    /// The connect began. `tag` is the tag of the request that opened the
    /// connection, which names the member it serves. An observer that reads
    /// no tags takes it as `Seen::Started`.
    fn started(
        &self,
        key: &Key,
        connection: ConnectionId,
        _tag: Option<&str>,
        clocked: bool,
        now: u64,
    ) {
        self.seen(key, connection, Seen::Started { clocked }, now);
    }
}

/// The open a connection's setup serves: where the setup reports progress,
/// and what that open's URL holds beyond its pool key (TR2.18). A connection
/// opened for no live open gets the default, which holds nothing.
#[derive(Clone, Default)]
pub(crate) struct Opening {
    pub(crate) progress: Progress,
    pub(crate) url: Option<Arc<UrlExtras>>,
    /// Authentication selection before private helper-policy pool isolation.
    pub(crate) identity: Option<Identity>,
    /// The open's selected key, which a URL password's open authenticates
    /// with when the server does not take the password.
    pub(crate) selected: Option<Arc<Selected>>,
    pub(crate) setup: Option<Arc<SetupContext>>,
    pub(crate) setup_slot: Arc<Mutex<Option<Arc<SetupContext>>>>,
    pub(crate) path: String,
    pub(crate) allocation_ms: u64,
    pub(crate) interaction_ms: u64,
}

/// Connection setup is nonblocking. An error returned by start owns no remaining
/// physical resources, including after an unwinding panic. Trust must be checked
/// before authentication is offered; setup ownership must transfer atomically.
pub(crate) trait Connector {
    type Resource: Resource;
    fn set_stall_ms(&mut self, _stall_ms: u64) {}
    fn setup_clock_source(
        &self,
    ) -> Option<(std::time::Instant, Arc<dyn Fn() -> u64 + Send + Sync>)> {
        None
    }
    /// Milliseconds on the clock this connector's connect deadlines are
    /// measured in, when it can say. A start that waited on a local budget
    /// shifts its deadline by the time it waited.
    fn now_ms(&self) -> Option<u64> {
        None
    }
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure>;
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        _opening: Opening,
    ) -> Result<Self::Resource, Failure> {
        self.start(key, identity, deadline)
    }
}

/// Single-owner connecting, idle or active SSH resource. Drop MUST terminate its
/// socket before native destructors, or retain a connecting job under its
/// supervisor. Forced disposal never acknowledges a still-live helper.
pub(crate) trait Resource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>>;
    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>>;
    /// True only for an idle authenticated session after complete channel cleanup.
    fn reusable(&self) -> bool;
    /// Ready when the peer has closed, reset, or sent unsolicited bytes on
    /// this connection while it is idle; Pending registers `cx`. Never
    /// consumes bytes, and is Pending whenever an exchange owns the I/O
    /// (dev-docs/GwzTransportIdleLossDesign.md §4).
    fn poll_idle_lost(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
    /// True while the connecting resource waits on a local budget (a job
    /// place, a shared reservation, a setup slot) before any network work. The
    /// host tells the pool, which pauses the connect clock for the wait and
    /// lets only the open's allocation bound it. A state, not a code.
    fn waiting_locally(&self) -> bool {
        false
    }
    /// How long the socket connect took, once it has completed: the host
    /// reports it once, so the limit machines' settle time follows the
    /// network's round trip and not the setup that follows it.
    fn tcp_connect_ms(&self) -> Option<u64> {
        None
    }
}

/// Tells `observer`, if there is one, what the host saw.
fn notify(
    observer: &Option<Arc<dyn Observer>>,
    now: u64,
    key: &Key,
    connection: ConnectionId,
    seen: Seen,
) {
    if let Some(observer) = observer {
        observer.seen(key, connection, seen, now);
    }
}

enum Phase {
    Connecting,
    Ready,
    Disposing {
        connect_failure: Option<Failure>,
        force: bool,
        /// The peer closed it while idle: report `idle_closed`, not `closed`.
        idle_lost: bool,
    },
    /// Disposed after idle loss, but a checkout had already leased it: kept
    /// until the lease is released and the pool's Close arrives.
    Lost,
}
impl Phase {
    /// The peer closed the connection while it was idle: disposed, or being.
    fn is_lost(&self) -> bool {
        matches!(
            self,
            Phase::Lost
                | Phase::Disposing {
                    idle_lost: true,
                    ..
                }
        )
    }
}
struct Entry<R> {
    key: Key,
    resource: R,
    phase: Phase,
    used: bool,
    /// Whether the pool has been told that the resource waits locally.
    waiting: bool,
    /// Whether its socket connect has been reported.
    tcp_reported: bool,
    _setup: Option<Arc<SetupContext>>,
}
pub(crate) struct PoolHost<C: Connector> {
    // Physical owners are destroyed before driver loss invalidates clients.
    entries: BTreeMap<ConnectionId, Entry<C::Resource>>,
    driver: PoolDriver,
    connector: C,
    action_budget: usize,
    disposal_error: Option<io::Error>,
    stall_ms: Arc<AtomicU64>,
    /// Set by a turn that took an action or moved an entry between phases,
    /// which can leave work for another turn that nothing else will announce.
    progressed: bool,
    /// The waker of the latest turn's caller, which a release wakes: it leaves
    /// the pool a Close for a discarded connection, an action no resource
    /// announces and the pool does not either, because the driver's waiter is
    /// dropped with each turn (`PoolDriver::next_action`).
    caller: Option<Waker>,
    observer: Option<Arc<dyn Observer>>,
    /// The latest turn's time, which the reports carry.
    now: u64,
}

impl<C: Connector> PoolHost<C> {
    pub(crate) fn new(config: Config, connector: C, now: u64) -> Result<(Pool, Self), Error> {
        let action_budget = config.total;
        let (pool, driver) = Pool::new(config)?;
        driver.advance(now);
        Ok((
            pool,
            Self {
                entries: BTreeMap::new(),
                driver,
                connector,
                action_budget,
                disposal_error: None,
                stall_ms: Arc::new(AtomicU64::new(0)),
                progressed: false,
                caller: None,
                observer: None,
                now,
            },
        ))
    }

    /// Reports every connection's state change to `observer` from now on.
    pub(crate) fn set_observer(&mut self, observer: Arc<dyn Observer>) {
        self.observer = Some(observer);
    }

    pub(crate) fn stall_slot(&self) -> Arc<AtomicU64> {
        self.stall_ms.clone()
    }

    /// The borrow cannot outlive the worker; callers must never move/clone native
    /// ownership out of the resource. Stale/cancelled leases cannot touch it.
    pub(crate) fn resource(&mut self, lease: &Lease) -> Result<&mut C::Resource, Error> {
        let entry = self
            .entries
            .get_mut(&lease.connection()?)
            .ok_or(Error::Stale)?;
        if !matches!(entry.phase, Phase::Ready) {
            return Err(Error::WrongState);
        }
        Ok(&mut entry.resource)
    }

    /// Whether the lease's connection was found closed while idle, a checkout
    /// having won the race with the loss (§4 of
    /// dev-docs/GwzTransportIdleLossDesign.md): `Some(reused)`, true when an
    /// earlier lease had used it. Its disposal may still be pending: the pool
    /// counts it Idle until that ends, and the connection is lost all the same.
    pub(crate) fn lost(&self, lease: &Lease) -> Option<bool> {
        let entry = self.entries.get(&lease.connection().ok()?)?;
        entry.phase.is_lost().then_some(entry.used)
    }

    pub(crate) fn allocation_reused(&mut self, lease: &Lease) -> Result<bool, Error> {
        let entry = self
            .entries
            .get_mut(&lease.connection()?)
            .ok_or(Error::Stale)?;
        if !matches!(entry.phase, Phase::Ready) {
            return Err(Error::WrongState);
        }
        Ok(std::mem::replace(&mut entry.used, true))
    }

    /// Ends `lease`, and wakes the caller of the turns: what the release leaves
    /// (a Close for a discarded connection, the discard a refused reuse
    /// schedules) is the next turn's to take, and a caller asleep between
    /// turns would take it only when it next woke.
    pub(crate) fn release(&mut self, lease: Lease, disposition: Disposition) -> Result<(), Error> {
        let released = self.release_lease(lease, disposition);
        if let Some(caller) = &self.caller {
            caller.wake_by_ref();
        }
        released
    }

    fn release_lease(&mut self, lease: Lease, disposition: Disposition) -> Result<(), Error> {
        let entry = self.entries.get(&lease.connection()?).ok_or(Error::Stale)?;
        if entry.phase.is_lost() {
            return lease.release(Disposition::Discarded);
        }
        let reusable = self.resource(&lease)?.reusable();
        if disposition == Disposition::Reusable && !reusable {
            // Lease Drop schedules discard; capacity remains reserved.
            return Err(Error::WrongState);
        }
        lease.release(disposition)
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.driver.next_deadline()
    }

    pub(crate) fn physical_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn shutdown_complete(&self) -> bool {
        self.entries.is_empty() && self.driver.shutdown_complete()
    }

    pub(crate) fn take_disposal_error(&mut self) -> Option<io::Error> {
        self.disposal_error.take()
    }

    /// Bounded turn. Advance BEFORE processing completions: exact deadline wins.
    /// A turn that made progress wakes `cx`'s waker, so that its caller takes
    /// the turn that follows (a resource that failed to connect is disposed of
    /// on it, a connection started by an action is first polled on it); a turn
    /// that made none does not. Resources wake the waker as their own work
    /// completes. The caller must also come at the pool's next deadline.
    pub(crate) fn step(&mut self, cx: &mut Context<'_>, now: u64) -> Result<(), Error> {
        self.step_reported(cx, now, |_| Opening::default())
    }
    /// `step`, where `opening` names the open each new connection serves.
    pub(crate) fn step_reported(
        &mut self,
        cx: &mut Context<'_>,
        now: u64,
        mut opening: impl FnMut(ConnectionId) -> Opening,
    ) -> Result<(), Error> {
        if !self
            .caller
            .as_ref()
            .is_some_and(|w| w.will_wake(cx.waker()))
        {
            self.caller = Some(cx.waker().clone());
        }
        self.now = now;
        self.driver.advance(now);
        self.actions(cx, &mut opening)?;
        let ids: Vec<_> = self.entries.keys().copied().collect();
        for id in ids {
            let _ = self.driver.service_setup_clock(id, cx.waker());
            let entry = self.entries.get_mut(&id).expect("worker-owned entry");
            // Disposed in this same pass: the pool still counts it Idle.
            if matches!(entry.phase, Phase::Ready) && entry.resource.poll_idle_lost(cx).is_ready() {
                entry.phase = Phase::Disposing {
                    connect_failure: None,
                    force: false,
                    idle_lost: true,
                };
                self.progressed = true;
            }
            match &mut entry.phase {
                Phase::Connecting => {
                    let polled = entry.resource.poll_connected(cx);
                    // The pool is told the resource's state, as it changes: a
                    // wait on a local budget is bounded by the open's
                    // allocation and is not the network's time. A refusal
                    // means the request has already ended.
                    let waiting = polled.is_pending() && entry.resource.waiting_locally();
                    if waiting != entry.waiting {
                        entry.waiting = waiting;
                        let _ = if waiting {
                            self.driver.begin_local_wait(id)
                        } else {
                            self.driver.end_local_wait(id)
                        };
                    }
                    if !entry.tcp_reported
                        && let Some(ms) = entry.resource.tcp_connect_ms()
                    {
                        entry.tcp_reported = true;
                        notify(
                            &self.observer,
                            self.now,
                            &entry.key,
                            id,
                            Seen::TcpConnected { ms },
                        );
                    }
                    match polled {
                        Poll::Ready(Ok(identity)) => {
                            entry.phase = Phase::Ready;
                            self.progressed = true;
                            self.driver.connected(id, Ok(identity))?;
                            notify(&self.observer, self.now, &entry.key, id, Seen::Connected);
                        }
                        Poll::Ready(Err(failure)) => {
                            entry.phase = Phase::Disposing {
                                connect_failure: Some(failure),
                                force: false,
                                idle_lost: false,
                            };
                            self.progressed = true;
                        }
                        Poll::Pending => {}
                    }
                }
                Phase::Disposing {
                    connect_failure,
                    force,
                    idle_lost,
                } => {
                    match entry.resource.poll_dispose(cx, *force) {
                        Poll::Ready(Ok(())) if *idle_lost => {
                            self.progressed = true;
                            // Destruction precedes the capacity acknowledgement.
                            // A checkout that won keeps the lease: its exchange
                            // finds the entry Lost, and its release and the
                            // pool's Close end it.
                            let key = entry.key.clone();
                            match self.driver.idle_closed(id) {
                                Ok(()) => {
                                    self.entries.remove(&id);
                                }
                                Err(Error::WrongState) => entry.phase = Phase::Lost,
                                Err(error) => return Err(error),
                            }
                            notify(&self.observer, self.now, &key, id, Seen::ServerClosed);
                        }
                        Poll::Ready(Ok(())) => {
                            self.progressed = true;
                            let failure = connect_failure.clone();
                            let key = entry.key.clone();
                            // Destruction precedes the capacity acknowledgement.
                            self.entries.remove(&id);
                            let seen = match &failure {
                                Some(failure) if failure.code == ErrorCode::Cancelled => {
                                    Seen::Retired
                                }
                                Some(_) => Seen::SetupEnded,
                                None => Seen::Disposed,
                            };
                            if let Some(failure) = failure {
                                self.driver.connected(id, Err(failure))?;
                            } else {
                                self.driver.closed(id)?;
                            }
                            notify(&self.observer, self.now, &key, id, seen);
                        }
                        Poll::Ready(Err(error)) => {
                            self.progressed = true;
                            *force = true;
                            if self.disposal_error.is_none() {
                                self.disposal_error = Some(error);
                            }
                        }
                        Poll::Pending => {}
                    }
                }
                Phase::Ready | Phase::Lost => {}
            }
        }
        self.actions(cx, &mut opening)?;
        if std::mem::take(&mut self.progressed) {
            cx.waker().wake_by_ref();
        }
        Ok(())
    }

    fn actions(
        &mut self,
        cx: &mut Context<'_>,
        opening: &mut impl FnMut(ConnectionId) -> Opening,
    ) -> Result<(), Error> {
        for _ in 0..self.action_budget {
            let action = match pin!(self.driver.next_action()).poll(cx) {
                Poll::Ready(Some(action)) => action,
                _ => return Ok(()),
            };
            self.progressed = true;
            match action {
                Action::Connect {
                    connection,
                    key,
                    identity,
                    network_deadline,
                    tag,
                } => match catch_unwind(AssertUnwindSafe(|| {
                    let mut reported = opening(connection);
                    if let Some(remaining) = self.driver.opening_allocation_remaining(connection) {
                        reported.allocation_ms = reported.allocation_ms.min(remaining);
                    }
                    let stall = self.stall_ms.load(Ordering::Relaxed);
                    self.connector.set_stall_ms(stall);
                    if let Some((origin, source)) = self.connector.setup_clock_source() {
                        let clock = self
                            .driver
                            .install_setup_clock(connection, source, stall)
                            .map_err(|_| Failure {
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                ..Default::default()
                            })?;
                        clock
                            .register_driver(Arc::new(cx.waker().clone()))
                            .deliver();
                        reported.setup = Some(SetupContext::new(clock, origin));
                        *reported
                            .setup_slot
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = reported.setup.clone();
                    }
                    let setup = reported.setup.clone();
                    self.connector
                        .start_reported(&key, &identity, network_deadline, reported)
                        .map(|resource| (resource, setup))
                })) {
                    Err(panic) => {
                        // The dequeued Connect has no physical owner after start
                        // unwinds. Settle that ledger entry before stopping the
                        // worker; already-owned entries remain in this host.
                        let _ = self.driver.connected(
                            connection,
                            Err(Failure {
                                detail: None,
                                setup_cause: None,
                                facts: None,
                                code: ErrorCode::Io,
                                effect: Effect::None,
                            }),
                        );
                        resume_unwind(panic);
                    }
                    Ok(result) => match result {
                        Ok((resource, setup)) => {
                            if let Some(observer) = &self.observer {
                                observer.started(
                                    &key,
                                    connection,
                                    tag.as_deref(),
                                    network_deadline.is_some(),
                                    self.now,
                                );
                            }
                            self.entries.insert(
                                connection,
                                Entry {
                                    key: key.clone(),
                                    resource,
                                    phase: Phase::Connecting,
                                    used: false,
                                    waiting: false,
                                    tcp_reported: false,
                                    _setup: setup,
                                },
                            );
                        }
                        Err(failure) => self.driver.connected(connection, Err(failure))?,
                    },
                },
                Action::CancelConnect { connection, .. } | Action::AbortConnect { connection } => {
                    let force = matches!(action, Action::AbortConnect { .. });
                    let entry = self.entries.get_mut(&connection).ok_or(Error::Stale)?;
                    match &mut entry.phase {
                        Phase::Disposing { force: prior, .. } => *prior |= force,
                        _ => {
                            entry.phase = Phase::Disposing {
                                connect_failure: Some(Failure {
                                    detail: None,
                                    setup_cause: None,
                                    facts: None,
                                    code: ErrorCode::Cancelled,
                                    effect: Effect::None,
                                }),
                                force,
                                idle_lost: false,
                            };
                        }
                    }
                }
                Action::Close { connection, .. } | Action::Abort { connection } => {
                    let force = matches!(action, Action::Abort { .. });
                    let entry = self.entries.get_mut(&connection).ok_or(Error::Stale)?;
                    notify(
                        &self.observer,
                        self.now,
                        &entry.key,
                        connection,
                        Seen::Closing,
                    );
                    // Also after idle loss: the pool's Close is acknowledged
                    // with `closed`, never a second `idle_closed`.
                    entry.phase = Phase::Disposing {
                        connect_failure: None,
                        force,
                        idle_lost: false,
                    };
                }
            }
        }
        cx.waker().wake_by_ref();
        Ok(())
    }
}

impl<C: Connector> Drop for PoolHost<C> {
    fn drop(&mut self) {
        let mut cx = Context::from_waker(std::task::Waker::noop());
        for entry in self.entries.values_mut() {
            let _ = entry.resource.poll_dispose(&mut cx, true);
        }
        // Normal worker shutdown retains this entire host until disposal.
        // Emergency Drop cancels supervised jobs; it never acknowledges reuse.
        self.entries.clear();
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod events_tests;
    }
}
