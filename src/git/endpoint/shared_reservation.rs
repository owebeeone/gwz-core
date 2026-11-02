//! Endpoint-wide physical reservation accounting for mixed SSH and HTTPS, and
//! the host's job budget they share.
//!
//! Protocol-specific pools still own their physical resources. They must take
//! one reservation here before admitting a physical connect, and release it
//! only after that resource is disposed. This keeps scheme-specific pools from
//! each consuming a separate full-sized host/total budget.

use super::agent_job::Supervisor;
use super::ssh_pool::{Connector, Opening, Resource};
use super::ssh_worker::ChannelResource;
use super::{ssh_channel::GitService, ssh_pump::SshPump};
use gwz_transport::stream::{MessageEndpoint, Stream};
use gwz_transport::{
    pool::{Identity, Key, PoolControl, Site},
    protocol::{Effect, ErrorCode, Failure},
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::{
    io, mem,
    task::{Context, Poll, Waker},
};

mod retire;

/// The pool owner session of one request's opens: `base` (the runtime's, or the mux session's) and the request, so that the
/// pool counts the request alone (State review P2-1). The pool bounds a session at 128 bytes, so a pair that would exceed
/// it keys the request by a fixed-width digest of the pair instead; a collision would only make two requests share a count.
pub(crate) fn request_owner_session(base: &str, request: &str) -> String {
    use std::hash::{Hash, Hasher};
    let plain = format!("{base}/{request}");
    if plain.len() <= 128 {
        return plain;
    }
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    (base, request).hash(&mut digest);
    let tail = format!("/{:016x}", digest.finish());
    let keep = 128 - tail.len();
    let mut end = base.len().min(keep);
    while !base.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{tail}", &base[..end])
}

/// What one transport host's SSH and HTTPS endpoints share: the physical
/// reservations below and the job budget of their setups (`Supervisor`). A new
/// authority is a new host, with a budget of its own.
#[derive(Clone)]
pub(crate) struct Authority {
    state: Arc<Mutex<State>>,
    supervisor: Supervisor,
}

struct State {
    total: usize,
    hosts: BTreeMap<String, usize>,
    /// The physical ceilings the host was built with.
    total_limit: usize,
    per_host_limit: usize,
    /// The limits of the requests that are live, (total, per host) by request. While any is live a reservation needs
    /// fewer than the largest of them in total and on its host (reuse design section 5, the no-clamp rule: a lone
    /// request keeps its own limits, below the base ceilings too, as `install_capacity` gave); overlapping requests
    /// share the larger. When none is live the base ceilings above apply. A request's entry goes when it ends, so no
    /// request's limit outlives it (State review P2-2), and nothing admitted is refused, shrunk or released for
    /// another request's limits. This is stricter than contract section 8's max(256, limit) for the legacy path only,
    /// which phase 3 retires (lane owner's ruling, remediation round 1).
    live: BTreeMap<String, (usize, usize)>,
    /// The sites (scheme, host, port) that hold reservations, by host, with how many each holds: where a host's idle
    /// connections are found when the ceilings fall below what it holds (`retire.rs`).
    sites: retire::Sites,
    /// The controls of the pools whose connections hold the reservations, to retire their surplus idle connections
    /// with (`retire.rs`).
    pools: Vec<PoolControl>,
    /// Connections waiting for a reservation, woken when one is returned.
    waiters: Vec<Waker>,
}

pub(crate) struct Reservation {
    authority: Authority,
    host: String,
    /// The site the reservation was taken for, when its taker named one.
    site: Option<Site>,
}

/// Takes one reservation before it starts a connection, as `Authority`
/// accounts them. A connection that finds the endpoint's reservations full
/// waits for one, with its start in hand, and is woken when one is returned:
/// a full reservation is backpressure, never the open's failure (adaptive
/// concurrency design §7.5).
pub(crate) struct ReservedConnector<C> {
    inner: Arc<Mutex<C>>,
    authority: Authority,
}

/// A connection of a `ReservedConnector`: waiting for its reservation, or
/// started and holding it until its disposal completes.
pub(crate) struct ReservedResource<C: Connector> {
    state: Slot<C>,
}
enum Slot<C: Connector> {
    Waiting(Box<Waiting<C>>),
    Started {
        inner: C::Resource,
        reservation: Option<Reservation>,
    },
    Ended,
}
/// The start of a connection that has not taken its reservation yet.
struct Waiting<C> {
    connector: Arc<Mutex<C>>,
    authority: Authority,
    key: Key,
    identity: Identity,
    deadline: Option<u64>,
    /// The connector's clock when the wait began, to shift the deadline by.
    began_ms: Option<u64>,
    /// Present when the pool started the connection for a live open.
    opening: Option<Opening>,
}

impl<C: Connector> ReservedResource<C> {
    /// The started connection's resource, once it has one.
    pub(crate) fn inner_mut(&mut self) -> Option<&mut C::Resource> {
        match &mut self.state {
            Slot::Started { inner, .. } => Some(inner),
            _ => None,
        }
    }
    /// Starts the connection once its reservation is free. Until then the
    /// connection reports that it waits locally (`waiting_locally`), so the
    /// pool leaves the wait to the open's allocation.
    fn poll_start(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Failure>> {
        let Slot::Waiting(waiting) = &self.state else {
            return Poll::Ready(Ok(()));
        };
        let Some(reservation) = waiting
            .authority
            .reserve_site_or_wait(waiting.key.site(), cx.waker())
        else {
            return Poll::Pending;
        };
        let Slot::Waiting(waiting) = mem::replace(&mut self.state, Slot::Ended) else {
            unreachable!("checked above");
        };
        let Waiting {
            connector,
            key,
            identity,
            deadline,
            began_ms,
            opening,
            ..
        } = *waiting;
        let mut connector = connector.lock().unwrap_or_else(|error| error.into_inner());
        // The time spent waiting was not the network's: the setup has the
        // network time it had when the wait began.
        let deadline = match (deadline, began_ms, connector.now_ms()) {
            (Some(deadline), Some(began), Some(now)) => {
                Some(deadline.saturating_add(now.saturating_sub(began)))
            }
            _ => deadline,
        };
        let started = match opening {
            Some(opening) => connector.start_reported(&key, &identity, deadline, opening),
            None => connector.start(&key, &identity, deadline),
        };
        match started {
            Ok(inner) => {
                self.state = Slot::Started {
                    inner,
                    reservation: Some(reservation),
                };
                Poll::Ready(Ok(()))
            }
            Err(error) => {
                drop(reservation);
                Poll::Ready(Err(error))
            }
        }
    }
}

fn capacity() -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        code: ErrorCode::Capacity,
        effect: Effect::None,
        facts: None,
    }
}

impl<C> ReservedConnector<C> {
    pub(crate) fn new(inner: C, authority: Authority) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
            authority,
        }
    }
}

impl<C: Connector> ReservedConnector<C> {
    fn begin(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        opening: Option<Opening>,
    ) -> Result<ReservedResource<C>, Failure> {
        let began_ms = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .now_ms();
        let mut resource = ReservedResource {
            state: Slot::Waiting(Box::new(Waiting {
                connector: self.inner.clone(),
                authority: self.authority.clone(),
                key: key.clone(),
                identity: identity.clone(),
                deadline,
                began_ms,
                opening,
            })),
        };
        // Starts at once when a reservation is free.
        if let Poll::Ready(Err(error)) =
            resource.poll_start(&mut Context::from_waker(Waker::noop()))
        {
            return Err(error);
        }
        Ok(resource)
    }
}

impl<C: Connector> Connector for ReservedConnector<C> {
    type Resource = ReservedResource<C>;
    fn setup_clock_source(
        &self,
    ) -> Option<(std::time::Instant, Arc<dyn Fn() -> u64 + Send + Sync>)> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .setup_clock_source()
    }

    fn set_stall_ms(&mut self, stall_ms: u64) {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_stall_ms(stall_ms);
    }
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        self.begin(key, identity, deadline, None)
    }
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        opening: Opening,
    ) -> Result<Self::Resource, Failure> {
        self.begin(key, identity, deadline, Some(opening))
    }
}

impl<C: Connector> Resource for ReservedResource<C> {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        match self.poll_start(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.state {
            Slot::Started { inner, .. } => inner.poll_connected(cx),
            _ => Poll::Ready(Err(capacity())),
        }
    }

    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        match &mut self.state {
            // Nothing was started and no reservation was taken.
            Slot::Waiting(_) | Slot::Ended => {
                self.state = Slot::Ended;
                Poll::Ready(Ok(()))
            }
            Slot::Started {
                inner, reservation, ..
            } => match inner.poll_dispose(cx, force) {
                Poll::Ready(Ok(())) => {
                    let _ = reservation.take();
                    Poll::Ready(Ok(()))
                }
                other => other,
            },
        }
    }

    fn reusable(&self) -> bool {
        match &self.state {
            Slot::Started { inner, .. } => inner.reusable(),
            _ => false,
        }
    }

    fn reuse_evidence(&self) -> Option<Arc<super::agent_auth::Evidence>> {
        match &self.state {
            Slot::Started { inner, .. } => inner.reuse_evidence(),
            _ => None,
        }
    }

    fn waiting_locally(&self) -> bool {
        match &self.state {
            Slot::Waiting(_) => true,
            Slot::Started { inner, .. } => inner.waiting_locally(),
            Slot::Ended => false,
        }
    }

    fn local_budget(&self) -> Option<super::setup_retry::LocalBudget> {
        match &self.state {
            Slot::Waiting(_) => Some(super::setup_retry::LocalBudget::Reservation),
            Slot::Started { inner, .. } => inner.local_budget(),
            Slot::Ended => None,
        }
    }

    fn tcp_connect_ms(&self) -> Option<u64> {
        match &self.state {
            Slot::Started { inner, .. } => inner.tcp_connect_ms(),
            _ => None,
        }
    }

    fn poll_idle_lost(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.poll_idle_lost(cx),
            // A connection still waiting for its reservation has no session to lose.
            _ => Poll::Pending,
        }
    }

    fn poll_hung_up(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.poll_hung_up(cx),
            _ => Poll::Pending,
        }
    }
}

impl<C: Connector + Send + 'static> ChannelResource for ReservedResource<C>
where
    C::Resource: ChannelResource,
{
    fn observation(&self) -> (bool, gwz_transport::protocol::Facts) {
        match &self.state {
            Slot::Started { inner, .. } => inner.observation(),
            _ => (false, gwz_transport::protocol::Facts::default()),
        }
    }

    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: GitService,
        path: &str,
    ) -> io::Result<()> {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.start_exchange(stream, endpoint, service, path),
            _ => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the connection has not started",
            )),
        }
    }

    fn pump(&mut self) -> Option<&mut SshPump<super::ssh_channel::SshChannel>> {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.pump(),
            _ => None,
        }
    }

    fn reclaim(&mut self) -> bool {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.reclaim(),
            _ => false,
        }
    }
}

impl<C: Connector> Drop for ReservedResource<C> {
    fn drop(&mut self) {
        // A host dropping an entry is not proof that the physical connector
        // stopped. Keep the permit leaked in that abnormal path; releasing it
        // here could let another scheme exceed the aggregate ceiling while
        // the old socket/helper is still live. Normal disposal takes the
        // reservation in poll_dispose after actual completion.
        if let Slot::Started { reservation, .. } = &mut self.state
            && let Some(reservation) = reservation.take()
        {
            std::mem::forget(reservation);
        }
    }
}

impl Authority {
    pub(crate) fn new(total: usize, per_host: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                total: 0,
                hosts: BTreeMap::new(),
                total_limit: total,
                per_host_limit: per_host,
                live: BTreeMap::new(),
                sites: retire::Sites::default(),
                pools: Vec::new(),
                waiters: Vec::new(),
            })),
            supervisor: Supervisor::new(),
        }
    }

    /// The job budget of this host's endpoints.
    pub(crate) fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    /// `request` was admitted, and registered, with `total` and `per_host` limits. Nothing is installed, resized,
    /// released or refused for them: while it is live a new reservation needs fewer than the largest limits of the live
    /// requests in total and on its host, which keeps a lone request's limits exactly (the no-clamp rule) and bounds
    /// overlapping requests by the larger. The request's own limit is held by the pools, whose requests carry it.
    /// Waiting connections are woken when the ceilings rise.
    pub(crate) fn admit_operation(&self, request: &str, total: usize, per_host: usize) {
        let waiters = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let before = state.ceilings();
            state.live.insert(request.to_owned(), (total, per_host));
            let after = state.ceilings();
            if after.0 > before.0 || after.1 > before.1 {
                mem::take(&mut state.waiters)
            } else {
                Vec::new()
            }
        };
        for waiter in waiters {
            waiter.wake();
        }
        // A lone request below the base lowers the ceilings under what idle connections hold (State review P2-3).
        self.retire_surplus();
    }

    /// `request` ended (cancelled or finished): its limits no longer bound the host. A request that was never
    /// admitted, or has ended already, changes nothing.
    pub(crate) fn end_operation(&self, request: &str) {
        let lowered = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.live.remove(request).is_some()
        };
        if lowered {
            // The ceilings fell to the remaining requests' (or to the base): the idle connections beyond them go
            // (State review P2-3).
            self.retire_surplus();
        }
    }

    /// A reservation now, or none, with `waker` woken when one is returned.
    /// Refusal and registration happen under one lock that every return also
    /// takes, so a return cannot slip between them unseen.
    pub(crate) fn reserve_or_wait(
        &self,
        host: impl Into<String>,
        waker: &Waker,
    ) -> Option<Reservation> {
        self.reserve_for(host.into(), None, waker)
    }
    /// `reserve_or_wait` for a connection to `site`, which the reservation remembers so that the connection's idle
    /// retirement can find it.
    pub(crate) fn reserve_site_or_wait(&self, site: Site, waker: &Waker) -> Option<Reservation> {
        self.reserve_for(site.host.clone(), Some(site), waker)
    }
    fn reserve_for(&self, host: String, site: Option<Site>, waker: &Waker) -> Option<Reservation> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let taken = state.take(self, host, site);
        if taken.is_none() && !state.waiters.iter().any(|queued| queued.will_wake(waker)) {
            state.waiters.push(waker.clone());
        }
        taken
    }
}

impl State {
    /// The ceilings a reservation is judged against: (total, per host). The live requests' largest limits, or the
    /// base when none is live.
    fn ceilings(&self) -> (usize, usize) {
        if self.live.is_empty() {
            return (self.total_limit, self.per_host_limit);
        }
        self.live
            .values()
            .fold((0, 0), |(total, per_host), limits| {
                (total.max(limits.0), per_host.max(limits.1))
            })
    }
    fn take(
        &mut self,
        authority: &Authority,
        host: String,
        site: Option<Site>,
    ) -> Option<Reservation> {
        let host_count = self.hosts.get(&host).copied().unwrap_or(0);
        let (total_limit, per_host_limit) = self.ceilings();
        if self.total >= total_limit || host_count >= per_host_limit {
            return None;
        }
        self.total += 1;
        self.hosts.insert(host.clone(), host_count + 1);
        if let Some(site) = &site {
            self.sites.add(site);
        }
        Some(Reservation {
            authority: authority.clone(),
            host,
            site,
        })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let waiters = {
            let mut state = self
                .authority
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.total = state.total.saturating_sub(1);
            if let Some(count) = state.hosts.get_mut(&self.host) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.hosts.remove(&self.host);
                }
            }
            if let Some(site) = &self.site {
                state.sites.remove(site);
            }
            mem::take(&mut state.waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
    }
}

cfg_if::cfg_if! { if #[cfg(test)] {
impl Authority {
    /// A reservation now, or none: what `reserve_or_wait` does without a wait.
    pub(crate) fn try_reserve(&self, host: impl Into<String>) -> Option<Reservation> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.take(self, host.into(), None)
    }
    /// The ceilings a reservation is judged against now: (total, per host).
    pub(crate) fn ceilings(&self) -> (usize, usize) {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).ceilings()
    }
    /// How many admitted requests are live.
    pub(crate) fn live_operations(&self) -> usize {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).live.len()
    }
    /// The reservations held in total and for `host`.
    pub(crate) fn counts(&self, host: &str) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.total, state.hosts.get(host).copied().unwrap_or(0))
    }
}
mod tests;
} }
