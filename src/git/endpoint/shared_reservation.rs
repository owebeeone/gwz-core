//! Endpoint-wide physical reservation accounting for mixed SSH and HTTPS.
//!
//! Protocol-specific pools still own their physical resources. They must take
//! one reservation here before admitting a physical connect, and release it
//! only after that resource is disposed. This keeps scheme-specific pools from
//! each consuming a separate full-sized host/total budget.

use super::ssh_pool::{Connector, Resource};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{Effect, ErrorCode, Failure},
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::{
    io,
    task::{Context, Poll},
};

#[derive(Clone)]
pub(crate) struct Authority {
    state: Arc<Mutex<State>>,
    total: usize,
    per_host: usize,
}

struct State {
    total: usize,
    hosts: BTreeMap<String, usize>,
}

pub(crate) struct Reservation {
    authority: Authority,
    host: String,
}

pub(crate) struct ReservedConnector<C> {
    inner: C,
    authority: Authority,
}

pub(crate) struct ReservedResource<R> {
    inner: R,
    reservation: Option<Reservation>,
}

impl<R> ReservedResource<R> {
    pub(crate) fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}

impl<C> ReservedConnector<C> {
    pub(crate) fn new(inner: C, authority: Authority) -> Self {
        Self { inner, authority }
    }
}

impl<C: Connector> Connector for ReservedConnector<C> {
    type Resource = ReservedResource<C::Resource>;

    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        let host = key.host.clone();
        let reservation = self.authority.try_reserve(host).ok_or(Failure {
            code: ErrorCode::Capacity,
            effect: Effect::None,
            facts: None,
        })?;
        match self.inner.start(key, identity, deadline) {
            Ok(inner) => Ok(ReservedResource {
                inner,
                reservation: Some(reservation),
            }),
            Err(error) => {
                drop(reservation);
                Err(error)
            }
        }
    }
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        progress: super::ssh_pool::Progress,
    ) -> Result<Self::Resource, Failure> {
        let reservation = self
            .authority
            .try_reserve(key.host.clone())
            .ok_or(Failure {
                code: ErrorCode::Capacity,
                effect: Effect::None,
                facts: None,
            })?;
        match self.inner.start_reported(key, identity, deadline, progress) {
            Ok(inner) => Ok(ReservedResource {
                inner,
                reservation: Some(reservation),
            }),
            Err(error) => {
                drop(reservation);
                Err(error)
            }
        }
    }
}

impl<R: Resource> Resource for ReservedResource<R> {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        self.inner.poll_connected(cx)
    }

    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        match self.inner.poll_dispose(cx, force) {
            Poll::Ready(Ok(())) => {
                let _ = self.reservation.take();
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }

    fn reusable(&self) -> bool {
        self.inner.reusable()
    }
}

impl<R> Drop for ReservedResource<R> {
    fn drop(&mut self) {
        // A host dropping an entry is not proof that the physical connector
        // stopped. Keep the permit leaked in that abnormal path; releasing it
        // here could let another scheme exceed the aggregate ceiling while
        // the old socket/helper is still live. Normal disposal takes the
        // reservation in poll_dispose after actual completion.
        if let Some(reservation) = self.reservation.take() {
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
            })),
            total,
            per_host,
        }
    }

    pub(crate) fn try_reserve(&self, host: impl Into<String>) -> Option<Reservation> {
        let host = host.into();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let host_count = state.hosts.get(&host).copied().unwrap_or(0);
        if state.total >= self.total || host_count >= self.per_host {
            return None;
        }
        state.total += 1;
        state.hosts.insert(host.clone(), host_count + 1);
        Some(Reservation {
            authority: self.clone(),
            host,
        })
    }

    pub(crate) fn counts(&self, host: &str) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.total, state.hosts.get(host).copied().unwrap_or(0))
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
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
    }
}

cfg_if::cfg_if! { if #[cfg(test)] {
mod tests {
    use super::super::ssh_pool::{Connector, Resource};
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
        assert!(
            https
                .start(&Key::https("github.example", 443), &Identity::Https, None)
                .is_err()
        );
        assert!(matches!(
            resource.poll_dispose(&mut cx, false),
            Poll::Ready(Ok(()))
        ));
        assert!(
            https
                .start(&Key::https("github.example", 443), &Identity::Https, None)
                .is_ok()
        );
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
