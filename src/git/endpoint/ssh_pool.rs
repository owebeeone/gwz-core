//! Physical ownership behind the transport pool's exclusive lease ledger.
//! One endpoint worker drives this host and its timer independently of Git calls.
use gwz_transport::{
    pool::{Action, Config, ConnectionId, Error, Identity, Key, Lease, Pool, PoolDriver},
    protocol::{Disposition, Effect, ErrorCode, Failure},
};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    pin::pin,
    task::{Context, Poll},
};

/// Connection setup is nonblocking. An error returned by start owns no remaining
/// physical resources. Trust must be checked before authentication is offered.
pub(crate) trait Connector {
    type Resource: Resource;
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure>;
}

/// Single-owner connecting, idle or active SSH resource. Drop MUST terminate its
/// socket before native destructors; forced disposal must never wait on a peer.
pub(crate) trait Resource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>>;
    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>>;
    /// True only for an idle authenticated session after complete channel cleanup.
    fn reusable(&self) -> bool;
}

enum Phase {
    Connecting,
    Ready,
    Disposing {
        connect_failure: Option<Failure>,
        force: bool,
    },
}
struct Entry<R> {
    resource: R,
    phase: Phase,
}
pub(crate) struct PoolHost<C: Connector> {
    // Physical owners are destroyed before driver loss invalidates clients.
    entries: BTreeMap<ConnectionId, Entry<C::Resource>>,
    driver: PoolDriver,
    connector: C,
    action_budget: usize,
    disposal_error: Option<io::Error>,
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
            },
        ))
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

    pub(crate) fn release(&mut self, lease: Lease, disposition: Disposition) -> Result<(), Error> {
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

    pub(crate) fn shutdown_complete(&self) -> bool {
        self.entries.is_empty() && self.driver.shutdown_complete()
    }

    pub(crate) fn take_disposal_error(&mut self) -> Option<io::Error> {
        self.disposal_error.take()
    }

    /// Bounded turn. Advance BEFORE processing completions: exact deadline wins.
    /// Host must call periodically even when no checkout or action wakes it.
    pub(crate) fn step(&mut self, cx: &mut Context<'_>, now: u64) -> Result<(), Error> {
        self.driver.advance(now);
        self.actions(cx)?;
        let ids: Vec<_> = self.entries.keys().copied().collect();
        for id in ids {
            let entry = self.entries.get_mut(&id).expect("worker-owned entry");
            match &mut entry.phase {
                Phase::Connecting => match entry.resource.poll_connected(cx) {
                    Poll::Ready(Ok(identity)) => {
                        entry.phase = Phase::Ready;
                        self.driver.connected(id, Ok(identity))?;
                    }
                    Poll::Ready(Err(failure)) => {
                        entry.phase = Phase::Disposing {
                            connect_failure: Some(failure),
                            force: false,
                        };
                    }
                    Poll::Pending => {}
                },
                Phase::Disposing {
                    connect_failure,
                    force,
                } => {
                    match entry.resource.poll_dispose(cx, *force) {
                        Poll::Ready(Ok(())) => {
                            let failure = connect_failure.clone();
                            // Destruction precedes the capacity acknowledgement.
                            self.entries.remove(&id);
                            if let Some(failure) = failure {
                                self.driver.connected(id, Err(failure))?;
                            } else {
                                self.driver.closed(id)?;
                            }
                        }
                        Poll::Ready(Err(error)) => {
                            *force = true;
                            if self.disposal_error.is_none() {
                                self.disposal_error = Some(error);
                            }
                        }
                        Poll::Pending => {}
                    }
                }
                Phase::Ready => {}
            }
        }
        self.actions(cx)
    }

    fn actions(&mut self, cx: &mut Context<'_>) -> Result<(), Error> {
        for _ in 0..self.action_budget {
            let action = match pin!(self.driver.next_action()).poll(cx) {
                Poll::Ready(Some(action)) => action,
                _ => return Ok(()),
            };
            match action {
                Action::Connect {
                    connection,
                    key,
                    identity,
                    network_deadline,
                } => match self.connector.start(&key, &identity, network_deadline) {
                    Ok(resource) => {
                        self.entries.insert(
                            connection,
                            Entry {
                                resource,
                                phase: Phase::Connecting,
                            },
                        );
                    }
                    Err(failure) => self.driver.connected(connection, Err(failure))?,
                },
                Action::CancelConnect { connection, .. } | Action::AbortConnect { connection } => {
                    let force = matches!(action, Action::AbortConnect { .. });
                    let entry = self.entries.get_mut(&connection).ok_or(Error::Stale)?;
                    match &mut entry.phase {
                        Phase::Disposing { force: prior, .. } => *prior |= force,
                        _ => {
                            entry.phase = Phase::Disposing {
                                connect_failure: Some(Failure {
                                    code: ErrorCode::Cancelled,
                                    effect: Effect::None,
                                }),
                                force,
                            };
                        }
                    }
                }
                Action::Close { connection, .. } | Action::Abort { connection } => {
                    let force = matches!(action, Action::Abort { .. });
                    let entry = self.entries.get_mut(&connection).ok_or(Error::Stale)?;
                    entry.phase = Phase::Disposing {
                        connect_failure: None,
                        force,
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
        // Resource::drop is the final socket-termination fallback, including
        // when forced cleanup reports an OS failure. Never acknowledge reuse.
        self.entries.clear();
    }
}
