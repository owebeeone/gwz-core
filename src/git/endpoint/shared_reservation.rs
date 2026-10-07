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
    pool::{Identity, Key},
    protocol::{Effect, ErrorCode, Failure},
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::{
    io, mem,
    task::{Context, Poll, Waker},
};

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
    total_limit: usize,
    per_host_limit: usize,
    /// Connections waiting for a reservation, woken when one is returned.
    waiters: Vec<Waker>,
}

pub(crate) struct Reservation {
    authority: Authority,
    host: String,
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
            .reserve_or_wait(waiting.key.host.clone(), cx.waker())
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

    fn waiting_locally(&self) -> bool {
        match &self.state {
            Slot::Waiting(_) => true,
            Slot::Started { inner, .. } => inner.waiting_locally(),
            Slot::Ended => false,
        }
    }

    fn poll_idle_lost(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match &mut self.state {
            Slot::Started { inner, .. } => inner.poll_idle_lost(cx),
            // A connection still waiting for its reservation has no session to lose.
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
                waiters: Vec::new(),
            })),
            supervisor: Supervisor::new(),
        }
    }

    /// The job budget of this host's endpoints.
    pub(crate) fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    /// Called after surplus idle owners have completed physical disposal.
    pub(crate) fn install_capacity(&self, total: usize, per_host: usize) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if total == 0
            || per_host == 0
            || state.total > total
            || state.hosts.values().any(|count| *count > per_host)
        {
            return false;
        }
        state.total_limit = total;
        state.per_host_limit = per_host;
        true
    }

    /// A reservation now, or none, with `waker` woken when one is returned.
    /// Refusal and registration happen under one lock that every return also
    /// takes, so a return cannot slip between them unseen.
    pub(crate) fn reserve_or_wait(
        &self,
        host: impl Into<String>,
        waker: &Waker,
    ) -> Option<Reservation> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let taken = state.take(self, host.into());
        if taken.is_none() && !state.waiters.iter().any(|queued| queued.will_wake(waker)) {
            state.waiters.push(waker.clone());
        }
        taken
    }
}

impl State {
    fn take(&mut self, authority: &Authority, host: String) -> Option<Reservation> {
        let host_count = self.hosts.get(&host).copied().unwrap_or(0);
        if self.total >= self.total_limit || host_count >= self.per_host_limit {
            return None;
        }
        self.total += 1;
        self.hosts.insert(host.clone(), host_count + 1);
        Some(Reservation {
            authority: authority.clone(),
            host,
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
        state.take(self, host.into())
    }
    /// The reservations held in total and for `host`.
    pub(crate) fn counts(&self, host: &str) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.total, state.hosts.get(host).copied().unwrap_or(0))
    }
}
mod tests {
    use super::super::ssh_pool::{Connector, Resource};
    use super::super::ssh_worker::ChannelResource;
    use super::super::{ssh_channel::GitService, ssh_pump::SshPump};
    use super::{Authority, ReservedConnector};
    use gwz_transport::{
        pool::{Identity, Key},
        protocol::{Effect, ErrorCode, Failure},
    };
    use std::{
        io,
        task::{Context, Poll, Waker},
    };

    struct FakeConnector {
        dispose_pending: bool,
        fail: bool,
    }
    struct FakeResource {
        dispose_pending: bool,
    }
    impl Connector for FakeConnector {
        type Resource = FakeResource;
        fn start(
            &mut self,
            _key: &Key,
            _identity: &Identity,
            _deadline: Option<u64>,
        ) -> Result<Self::Resource, Failure> {
            if self.fail {
                return Err(Failure {
                    detail: None,
                    setup_cause: None,
                    code: ErrorCode::Io,
                    effect: Effect::None,
                    facts: None,
                });
            }
            Ok(FakeResource {
                dispose_pending: self.dispose_pending,
            })
        }
    }
    impl Resource for FakeResource {
        fn poll_connected(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<Option<Identity>, Failure>> {
            Poll::Ready(Ok(Some(Identity::Ambient)))
        }
        fn poll_dispose(&mut self, _cx: &mut Context<'_>, _force: bool) -> Poll<io::Result<()>> {
            if self.dispose_pending {
                self.dispose_pending = false;
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }
        fn reusable(&self) -> bool {
            false
        }
        fn poll_idle_lost(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    impl ChannelResource for FakeResource {
        fn observation(&self) -> (bool, gwz_transport::protocol::Facts) {
            (false, gwz_transport::protocol::Facts::default())
        }
        fn start_exchange(
            &mut self,
            _stream: gwz_transport::stream::Stream,
            _endpoint: gwz_transport::stream::MessageEndpoint,
            _service: GitService,
            _path: &str,
        ) -> io::Result<()> {
            Ok(())
        }
        fn pump(&mut self) -> Option<&mut SshPump<super::super::ssh_channel::SshChannel>> {
            None
        }
        fn reclaim(&mut self) -> bool {
            true
        }
    }

    #[test]
    fn ssh_and_https_share_host_and_total_limits() {
        let authority = Authority::new(2, 1);
        let ssh = authority.try_reserve("github.example").unwrap();
        assert!(authority.try_reserve("github.example").is_none());
        let https = authority.try_reserve("gitlab.example:443").unwrap();
        assert!(authority.try_reserve("other.example:443").is_none());
        assert_eq!(authority.counts("github.example"), (2, 1));
        drop(ssh);
        assert!(authority.try_reserve("github.example").is_some());
        drop(https);
    }

    #[test]
    fn disposal_releases_the_shared_slot_for_another_scheme() {
        let authority = Authority::new(1, 1);
        let ssh = authority.try_reserve("github.example").unwrap();
        assert!(authority.try_reserve("github.example").is_none());
        drop(ssh);
        let https = authority.try_reserve("github.example").unwrap();
        assert_eq!(authority.counts("github.example"), (1, 1));
        drop(https);
        assert_eq!(authority.counts("github.example"), (0, 0));
    }

    #[test]
    fn reservation_survives_pending_disposal_until_completion() {
        let authority = Authority::new(1, 1);
        let mut ssh = ReservedConnector::new(
            FakeConnector {
                dispose_pending: true,
                fail: false,
            },
            authority.clone(),
        );
        let key = Key::ssh("git", "github.example", 22);
        let mut resource = ssh.start(&key, &Identity::Ambient, None).unwrap();
        assert_eq!(resource.observation().0, false);
        assert!(resource.reclaim());
        assert!(resource.pump().is_none());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            resource.poll_dispose(&mut cx, false),
            Poll::Pending
        ));
        let mut https = ReservedConnector::new(
            FakeConnector {
                dispose_pending: false,
                fail: false,
            },
            authority.clone(),
        );
        // The endpoint-wide reservation is full: the second scheme's
        // connection waits for it, it is not refused (§7.5).
        let mut waiting = https
            .start(&Key::https("github.example", 443), &Identity::Https, None)
            .expect("a full reservation queues the connection; it is not refused");
        assert!(matches!(
            waiting.poll_connected(&mut cx),
            Poll::Pending
        ));
        assert_eq!(authority.counts("github.example"), (1, 1));
        assert!(matches!(
            resource.poll_dispose(&mut cx, false),
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            waiting.poll_connected(&mut cx),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(authority.counts("github.example"), (1, 1));
        assert!(matches!(
            waiting.poll_dispose(&mut cx, false),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(authority.counts("github.example"), (0, 0));
    }

    #[test]
    fn a_waiting_connection_is_woken_by_a_released_reservation_and_disposes_without_one() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        struct Count(AtomicUsize);
        impl std::task::Wake for Count {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let authority = Authority::new(1, 1);
        let held = authority.try_reserve("host").unwrap();
        let mut connector = ReservedConnector::new(
            FakeConnector { dispose_pending: false, fail: false },
            authority.clone(),
        );
        let mut waiting = connector
            .start(&Key::https("host", 443), &Identity::Https, None)
            .unwrap();
        let woken = Arc::new(Count(AtomicUsize::new(0)));
        let waker = Waker::from(woken.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(waiting.poll_connected(&mut cx), Poll::Pending));
        assert_eq!(woken.0.load(Ordering::SeqCst), 0);
        drop(held);
        assert_eq!(woken.0.load(Ordering::SeqCst), 1, "the release wakes the waiter");
        // A waiter disposed before it was served never held a reservation.
        assert!(matches!(waiting.poll_dispose(&mut cx, false), Poll::Ready(Ok(()))));
        assert_eq!(authority.counts("host"), (0, 0));
    }

    #[test]
    fn shrinking_authority_waits_for_actual_disposal_across_schemes() {
        let authority = Authority::new(2, 2);
        let ssh = authority.try_reserve("host").unwrap();
        let https = authority.try_reserve("host").unwrap();
        assert!(!authority.install_capacity(1, 1));
        assert_eq!(authority.counts("host"), (2, 2));
        drop(https);
        assert!(authority.install_capacity(1, 1));
        assert!(authority.try_reserve("host").is_none());
        drop(ssh);
        assert!(authority.try_reserve("host").is_some());
    }

    #[test]
    fn idle_loss_reaches_the_host_through_the_reservation() {
        let mut connector = ReservedConnector::new(
            FakeConnector {
                dispose_pending: false,
                fail: false,
            },
            Authority::new(1, 1),
        );
        let mut resource = connector
            .start(&Key::ssh("git", "github.example", 22), &Identity::Ambient, None)
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(resource.poll_idle_lost(&mut cx).is_ready());
    }

    #[test]
    fn a_connection_waiting_for_its_reservation_is_not_idle() {
        // The wrapped resource would report itself lost (`FakeResource` always
        // does), but a connection that has not started has no session to lose.
        let authority = Authority::new(1, 1);
        let held = authority.try_reserve("host").unwrap();
        let mut connector = ReservedConnector::new(
            FakeConnector {
                dispose_pending: false,
                fail: false,
            },
            authority.clone(),
        );
        let mut waiting = connector
            .start(&Key::ssh("git", "host", 22), &Identity::Ambient, None)
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(waiting.waiting_locally());
        assert!(waiting.poll_idle_lost(&mut cx).is_pending());
        drop(held);
        assert!(matches!(waiting.poll_connected(&mut cx), Poll::Ready(Ok(_))));
        assert!(!waiting.waiting_locally());
        assert!(waiting.poll_idle_lost(&mut cx).is_ready());
    }

    #[test]
    fn failed_connect_releases_unowned_reservation() {
        let authority = Authority::new(1, 1);
        let mut connector = ReservedConnector::new(
            FakeConnector {
                dispose_pending: false,
                fail: true,
            },
            authority.clone(),
        );
        assert!(
            connector
                .start(&Key::https("github.example", 443), &Identity::Https, None)
                .is_err()
        );
        assert!(authority.try_reserve("github.example").is_some());
    }
}

} }
