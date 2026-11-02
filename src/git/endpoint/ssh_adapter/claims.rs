//! Which connections the host has leased an exchange on that the worker has not yet begun, and which the worker has
//! retired (plan Phase 1 integration fix-up, Code P3-2 = State P3-2). The SSH and HTTPS adapters each keep one.
//!
//! The host leases an idle connection and queues the exchange's start in one step (`start_exchange`), while the worker
//! may retire that same connection in its next pass (a failed reuse proof retires the idle connections that share the
//! key or the host key; an idle watch that fires retires its connection). Both steps take this one lock, so exactly
//! one of them wins: a connection the host has leased is never retired, and one the worker has retired is refused to
//! the host (`ErrorKind::NotConnected`, which the host takes as a lost lease and asks again for a new connection)
//! instead of failing the exchange on a connection that is going.
//!
//! A retirement stands until the host has taken the connection's end report from the outbox (`Shared::poll_completion`
//! forgets it there), not only until the disposal: the host counts the connection idle until it reads that report, so
//! between the disposal and the read it can still lease it (findings F8 follow-up). A lease stands until the worker
//! takes up the exchange's start (`begun`), or, when the start is never queued, until `start_exchange` returns
//! (`release`): no failed start leaves a lease that would keep the connection from being retired.
//!
//! The claims see only the leases `start_exchange` makes. A pool lease the host holds while a reuse proof runs is not
//! one of them, and its start can come after the host has read the connection's end report (the verdict and the report
//! read in one pass): by then the claims are forgotten, and it is `SchemeRouter`, whose route goes with that report,
//! that refuses the start (fix round, CODE-P2-1).
use gwz_transport::pool::ConnectionId;
use std::{collections::BTreeSet, sync::Mutex};

/// The claims over the connections `K` names (a pool `ConnectionId`; any ordered key serves the rows).
pub(super) struct Claims<K = ConnectionId> {
    state: Mutex<State<K>>,
}
struct State<K> {
    /// Leased by the host; the worker has not yet begun the exchange.
    leased: BTreeSet<K>,
    /// Retired by the worker; its disposal has not yet been reported.
    retired: BTreeSet<K>,
}
impl<K> Default for Claims<K> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                leased: BTreeSet::new(),
                retired: BTreeSet::new(),
            }),
        }
    }
}
impl<K: Ord + Copy> Claims<K> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<K>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The host leases `connection` and is about to queue the exchange's start. False: the worker retired it first.
    pub(super) fn lease(&self, connection: K) -> bool {
        let mut state = self.lock();
        if state.retired.contains(&connection) {
            return false;
        }
        state.leased.insert(connection);
        true
    }
    /// The worker has taken the exchange's start for `connection`: the lease no longer stands between it and retirement,
    /// for the connection is not idle now.
    pub(super) fn begun(&self, connection: K) {
        self.release(connection);
    }
    /// The host's lease of `connection` ends without a start queued (the start failed, or no worker takes it): the
    /// connection is idle as it was, and may be retired or leased again.
    pub(super) fn release(&self, connection: K) {
        self.lock().leased.remove(&connection);
    }
    /// The worker retires `connection`, an idle connection. False: the host leased it first, so it stays.
    pub(super) fn retire(&self, connection: K) -> bool {
        let mut state = self.lock();
        if state.leased.contains(&connection) {
            return false;
        }
        state.retired.insert(connection);
        true
    }
    /// `connection` is gone, and the host has read so.
    pub(super) fn forget(&self, connection: K) {
        let mut state = self.lock();
        state.leased.remove(&connection);
        state.retired.remove(&connection);
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl<K: Ord + Copy> Claims<K> {
            /// Whether a lease of `connection` stands.
            pub(super) fn leased(&self, connection: K) -> bool {
                self.lock().leased.contains(&connection)
            }
            /// Whether nothing at all is recorded.
            pub(super) fn is_empty(&self) -> bool {
                let state = self.lock();
                state.leased.is_empty() && state.retired.is_empty()
            }
            /// How many retirements stand.
            pub(super) fn retired(&self) -> usize {
                self.lock().retired.len()
            }
        }

        mod tests {
            use super::Claims;

            fn id(n: u64) -> u64 {
                n
            }

            #[test]
            fn a_leased_connection_is_never_retired() {
                let claims = Claims::default();
                assert!(claims.lease(id(1)));
                assert!(!claims.retire(id(1)), "the host leased it first");
                // Once the worker has begun the exchange the connection is not idle, and a later idle one may retire.
                claims.begun(id(1));
                assert!(claims.retire(id(1)));
            }

            #[test]
            fn a_retired_connection_is_refused_to_the_host() {
                let claims = Claims::default();
                assert!(claims.retire(id(2)));
                assert!(!claims.lease(id(2)), "the worker retired it first");
                assert!(claims.lease(id(3)), "another connection is unaffected");
            }

            #[test]
            fn a_released_lease_leaves_the_connection_retirable() {
                let claims = Claims::default();
                assert!(claims.lease(id(5)));
                // The start failed before it was queued: nothing holds the connection back from retirement.
                claims.release(id(5));
                assert!(claims.retire(id(5)));
                assert!(!claims.lease(id(5)), "retired");
            }

            #[test]
            fn a_forgotten_connection_carries_nothing() {
                let claims = Claims::default();
                assert!(claims.retire(id(4)));
                claims.forget(id(4));
                assert!(claims.lease(id(4)));
                claims.forget(id(4));
                assert!(claims.retire(id(4)));
            }
        }
    }
}
