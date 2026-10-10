//! The governor: every operation's limit machines for one pool, behind one
//! lock, driving the pool's numbers (adaptive concurrency design §4.1 and
//! §4.9).
//!
//! **One lock, two domains (F10).** The pool host reports each connection's
//! state change, in the order the host acted, through [`Observer`]; the
//! endpoints report the results of attempts (`answered`, `refused`) and ask
//! for admission. All of it passes through the one mutex here, so a machine
//! sees a single order, and every number the machines set on the pool
//! (`set_limit`, `set_settle`, `discard_idle`) is set under that same lock,
//! before the call returns to the endpoint that asked. Lock order is host,
//! then governor, then pool; nothing here calls back into a host.
//!
//! **Machines are per operation (§4.1).** An operation's machine for a site
//! is created with the operation and dropped with it, with its own ceiling
//! `C`, its own `--max-retries` flag, its own `N` and timer. What the server
//! holds is not an operation's: every connection event reaches every live
//! operation's table, so each sees the same `Connected` and `Possible`; what
//! an operation learns from is the results its own members report. The
//! server's word is shared: a `Retry-After` heard by one operation holds the
//! site for all live ones, and for the next operation if it outlasts them.
//! The pool has one number per site, which is the most permissive of the
//! operations' (their own endpoints enforce their own targets, ahead of the
//! pool), and a settle time that is the longest. Overlapping operations share
//! one capacity (a different one is refused), so their ceilings agree; their
//! flags and budgets need not.
//!
//! **An attempt is its connection.** A new connection's attempt begins at the
//! socket connect the host reports (§4.3: the window runs from the connect)
//! and a leased exchange's at `exchange_begins`; both are keyed by the pool's
//! connection id, which is also the machine's. The pool enforces the number
//! of connections; the endpoints consult only the gate (a hold, a
//! confirmation, a test in flight) and the number, so an open that can lease
//! an idle connection is never held by a count.

use super::{
    control::{Action, Limit, Outcome},
    fsm::State,
    hold::Hold,
    notes::Note,
    states::{ConnEvent, ConnId},
    timer::Spread,
    windows::{AttemptId, AttemptKind, Own},
};
use crate::git::endpoint::ssh_pool::{Observer, Seen};
use gwz_transport::{
    pool::{ConnectionId, Key, PoolControl, Site},
    protocol::Scheme,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

/// Notes kept for the reader that drains them (§9); the oldest are dropped.
const NOTES_KEPT: usize = 64;

/// What an endpoint needs to start something on a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Admission {
    /// New connections may start: no hold, confirmation, test in flight or
    /// inconclusive refusal's overlap.
    pub(crate) gate_open: bool,
    /// How many connections the operation may have on the site: the believed
    /// limit `N`, or the ceiling in SATURATED.
    pub(crate) target: usize,
    /// Below the ceiling, whether the site has room (`Possible < N`) for a
    /// member that cannot lease an idle connection (§4.5, §5.2). Always true
    /// at the ceiling, where the pool's own limit holds a request.
    pub(crate) room: bool,
    /// The connections of the site that are set up: some may be idle, which a
    /// member leases without a start.
    pub(crate) connected: usize,
}

/// A site's machine as the endpoints and the member scheduler may read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct View {
    pub(crate) state: State,
    pub(crate) n: usize,
    pub(crate) ceiling: usize,
    pub(crate) connected: usize,
    pub(crate) possible: usize,
    pub(crate) pool_limit: usize,
    pub(crate) settle_ms: u64,
    pub(crate) confirmation: bool,
}

pub(super) struct Slot {
    pub(super) site: Site,
    pub(super) limit: Limit,
    /// The attempts that are exchanges on a leased connection.
    pub(super) leased: BTreeSet<u64>,
    /// Setups the host saw end, oldest first, with when: each waits for its
    /// endpoint to say why it failed (`Scoped::setup_failed`), since only the
    /// endpoint holds the failure. The window stopped where the setup ended.
    pub(super) ended: VecDeque<(u64, u64)>,
}

/// How long an ended setup waits for its endpoint's word before it counts as
/// an attempt that ended with no verdict.
const ENDED_TTL_MS: u64 = 1_000;

/// One operation's machines.
pub(super) struct Scope {
    pub(super) ceiling: usize,
    pub(super) adaptive: bool,
    pub(super) slots: Vec<Slot>,
    pub(super) notes: Vec<(Site, Note)>,
}

pub(super) struct Book {
    /// What an operation not yet begun is given (the pool's own cap).
    default_ceiling: usize,
    default_adaptive: bool,
    spread: Box<dyn Fn() -> Spread + Send + Sync>,
    pub(super) scopes: BTreeMap<String, Scope>,
    /// What the pool was last told for each site: `(limit, settle ms)`.
    applied: Vec<(Site, (usize, u64))>,
    /// Holds still in force when their operation ended: the server's word
    /// outlasts the operation that heard it.
    held_over: Vec<(Site, Hold)>,
}

/// A cloneable handle on one pool's limit machines.
#[derive(Clone)]
pub(crate) struct Governor {
    pub(super) control: PoolControl,
    pub(super) book: Arc<Mutex<Book>>,
}

/// One operation's view of a governor: what its endpoint asks and reports.
#[derive(Clone)]
pub(crate) struct Scoped {
    pub(super) governor: Governor,
    pub(super) operation: String,
}

/// A pool connection as the machine names it: the pool's sequence number,
/// which is unique within one pool, and a governor serves one pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Conn(pub(crate) u64);
impl Conn {
    pub(crate) fn of(connection: ConnectionId) -> Self {
        Self(connection.sequence())
    }
    pub(super) fn attempt(self) -> AttemptId {
        AttemptId(self.0)
    }
    pub(super) fn id(self) -> ConnId {
        ConnId(self.0)
    }
}

impl Book {
    /// The scope of `operation`, made with the defaults on first use.
    pub(super) fn scope(&mut self, operation: &str) -> &mut Scope {
        let (ceiling, adaptive) = (self.default_ceiling, self.default_adaptive);
        self.scopes
            .entry(operation.to_owned())
            .or_insert_with(|| Scope::new(ceiling, adaptive))
    }

    /// The index of `site`'s slot in `operation`'s scope, made on first use.
    /// A new machine inherits any hold in force on the site: from the other
    /// live operations, or one an ended operation left.
    pub(super) fn slot(&mut self, operation: &str, site: &Site, now: u64) -> usize {
        let scope = self.scope(operation);
        if let Some(index) = scope.slots.iter().position(|slot| &slot.site == site) {
            return index;
        }
        let (ceiling, adaptive) = (scope.ceiling, scope.adaptive);
        let mut limit = Limit::new(ceiling, adaptive, (self.spread)());
        let live = self
            .scopes
            .values()
            .flat_map(|scope| &scope.slots)
            .filter(|slot| &slot.site == site)
            .find_map(|slot| slot.limit.hold_in_force(now));
        let left = self
            .held_over
            .iter()
            .position(|(held, hold)| held == site && hold.in_force(now))
            .map(|index| self.held_over.swap_remove(index).1);
        if let Some(hold) = live.or(left) {
            limit.restore_hold(hold);
        }
        let scope = self.scope(operation);
        scope.slots.push(Slot {
            site: site.clone(),
            limit,
            leased: BTreeSet::new(),
            ended: VecDeque::new(),
        });
        scope.slots.len() - 1
    }
}

impl Scope {
    fn new(ceiling: usize, adaptive: bool) -> Self {
        Self {
            ceiling,
            adaptive,
            slots: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl Governor {
    /// A governor over the pool `control` reaches. An operation that was never
    /// begun gets `ceiling`, the pool's own cap, and `adaptive`, false when no
    /// refusal may lower `N` (`--max-retries 0`, §5.3). `spread` draws each
    /// key's probe-timer jitter.
    pub(crate) fn new(
        control: PoolControl,
        ceiling: usize,
        adaptive: bool,
        spread: impl Fn() -> Spread + Send + Sync + 'static,
    ) -> Self {
        Self {
            control,
            book: Arc::new(Mutex::new(Book {
                default_ceiling: ceiling,
                default_adaptive: adaptive,
                spread: Box::new(spread),
                scopes: BTreeMap::new(),
                applied: Vec::new(),
                held_over: Vec::new(),
            })),
        }
    }
    /// `new`, with the probe timers' jitter drawn from the operating system's
    /// random source.
    pub(crate) fn random(control: PoolControl, ceiling: usize, adaptive: bool) -> Self {
        Self::new(control, ceiling, adaptive, Spread::random)
    }
    pub(super) fn book(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// What `operation`'s endpoint asks and reports.
    pub(crate) fn scoped(&self, operation: &str) -> Scoped {
        Scoped {
            governor: self.clone(),
            operation: operation.to_owned(),
        }
    }

    /// An operation begins with ceiling `C` and `adaptive` false when no
    /// refusal may lower its `N`. Its machines start SATURATED. An operation
    /// of the same name that was live is replaced as if it had ended.
    pub(crate) fn begin_operation(
        &self,
        operation: &str,
        ceiling: usize,
        adaptive: bool,
        now: u64,
    ) {
        let mut book = self.book();
        self.drop_scope(&mut book, operation, now);
        book.scopes
            .insert(operation.to_owned(), Scope::new(ceiling, adaptive));
        // The pool's numbers may have been cleared by the capacity install
        // that admitted this operation: send them again.
        book.applied.clear();
    }

    /// An operation ended: its machines go, and the numbers they set leave the
    /// pool unless another operation still wants them. A hold in force stays
    /// for the next operation.
    pub(crate) fn end_operation(&self, operation: &str, now: u64) {
        let mut book = self.book();
        self.drop_scope(&mut book, operation, now);
    }

    fn drop_scope(&self, book: &mut Book, operation: &str, now: u64) {
        book.held_over.retain(|(_, hold)| hold.in_force(now));
        let Some(scope) = book.scopes.remove(operation) else {
            return;
        };
        for mut slot in scope.slots {
            if slot.limit.holding(now) {
                let hold = slot.limit.take_hold();
                book.held_over.push((slot.site.clone(), hold));
            }
            self.reconcile(book, &slot.site);
        }
    }

    /// Time has passed: holds end, settles lapse, notes flush.
    pub(crate) fn tick(&self, now: u64) {
        let mut book = self.book();
        let at: Vec<(String, usize)> = book
            .scopes
            .iter()
            .flat_map(|(name, scope)| (0..scope.slots.len()).map(move |i| (name.clone(), i)))
            .collect();
        for (operation, index) in at {
            self.sync(&mut book, &operation, index, now);
        }
    }

    /// The next time anything in any key changes with no event.
    pub(crate) fn next_deadline(&self, now: u64) -> Option<u64> {
        self.book()
            .scopes
            .values()
            .flat_map(|scope| &scope.slots)
            .filter_map(|slot| slot.limit.next_deadline(now))
            .min()
    }

    /// Brings time forward on one slot, does what the machine asks of the
    /// pool, and tells the pool its numbers when they changed.
    pub(super) fn sync(&self, book: &mut Book, operation: &str, index: usize, now: u64) {
        let Some(scope) = book.scopes.get_mut(operation) else {
            return;
        };
        let slot = &mut scope.slots[index];
        while let Some(&(conn, at)) = slot.ended.front() {
            if now.saturating_sub(at) < ENDED_TTL_MS {
                break;
            }
            slot.ended.pop_front();
            slot.limit.result(AttemptId(conn), Outcome::Ended, now);
        }
        if slot.limit.tick(now) == Some(Action::DiscardIdle) {
            // One step under the pool's lock, before the hold lifts.
            self.control.discard_idle(&slot.site);
            slot.limit.idle_discarded(now);
        }
        let notes = slot.limit.drain_notes();
        let site = slot.site.clone();
        scope
            .notes
            .extend(notes.into_iter().map(|note| (site.clone(), note)));
        let excess = scope.notes.len().saturating_sub(NOTES_KEPT);
        scope.notes.drain(..excess);
        self.reconcile(book, &site);
    }

    /// Tells the pool `site`'s numbers when they changed: the most
    /// permissive limit and the longest settle of the live machines, or
    /// nothing when none is left.
    fn reconcile(&self, book: &mut Book, site: &Site) {
        let wanted = book
            .scopes
            .values()
            .flat_map(|scope| &scope.slots)
            .filter(|slot| &slot.site == site)
            .map(|slot| (slot.limit.pool_limit(), slot.limit.settle_ms()))
            .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)));
        let known = book.applied.iter().position(|(applied, _)| applied == site);
        match (wanted, known) {
            (Some(wanted), Some(index)) if book.applied[index].1 != wanted => {
                self.tell_pool(site, wanted);
                book.applied[index].1 = wanted;
            }
            (Some(wanted), None) => {
                self.tell_pool(site, wanted);
                book.applied.push((site.clone(), wanted));
            }
            (None, Some(index)) => {
                book.applied.swap_remove(index);
                self.control.clear_limit(site);
                let _ = self.control.set_settle(site, 0);
            }
            _ => {}
        }
    }

    fn tell_pool(&self, site: &Site, (limit, settle_ms): (usize, u64)) {
        let _ = self.control.set_limit(site, limit);
        let _ = self.control.set_settle(site, settle_ms);
    }
}

impl Observer for Governor {
    fn seen(&self, key: &Key, connection: ConnectionId, seen: Seen, now: u64) {
        let connection = Conn::of(connection);
        let site = key.site();
        let mut book = self.book();
        let operations: Vec<String> = book.scopes.keys().cloned().collect();
        for operation in operations {
            let index = book.slot(&operation, &site, now);
            let slot = &mut book.scopes.get_mut(&operation).expect("scope").slots[index];
            let id = connection.id();
            match seen {
                Seen::Started { clocked } => {
                    // A test an endpoint armed for its carrier is this
                    // connection; any other start is an ordinary one.
                    let (kind, target) = slot
                        .limit
                        .take_armed()
                        .unwrap_or_else(|| (AttemptKind::Ordinary, slot.limit.pool_limit()));
                    let began = slot.limit.begin(
                        connection.attempt(),
                        kind,
                        target,
                        Own::New(id),
                        clocked,
                        now,
                    );
                    if !began {
                        slot.limit.conn(id, ConnEvent::Started { clocked }, now);
                    }
                }
                // HTTPS is set up when its first exchange is answered, which
                // the endpoint reports; SSH when it is authenticated, which
                // is this. Authenticated is a fact about the server, so every
                // operation's machine takes it as its success.
                Seen::Connected if key.scheme.wire() == Scheme::Https.wire() => {}
                Seen::Connected => {
                    slot.limit.conn(id, ConnEvent::Connected, now);
                    slot.limit.result(
                        connection.attempt(),
                        Outcome::Succeeded { fresh: true },
                        now,
                    );
                }
                // A setup that ended: the server refused it, or the client gave
                // up on it. The window stops here, and the verdict waits for the
                // endpoint's failure (`Scoped::setup_failed`).
                Seen::SetupEnded | Seen::Retired => {
                    let event = match seen {
                        Seen::SetupEnded => ConnEvent::SetupEnded,
                        _ => ConnEvent::Retired,
                    };
                    slot.limit.conn(id, event, now);
                    slot.leased.remove(&connection.0);
                    slot.limit.freeze(connection.attempt());
                    slot.ended.push_back((connection.0, now));
                }
                Seen::Closing | Seen::ServerClosed => {
                    let event = match seen {
                        Seen::Closing => ConnEvent::Closing,
                        _ => ConnEvent::ServerClosed,
                    };
                    slot.limit.conn(id, event, now);
                    // The connection left with its attempt unanswered: the
                    // attempt ends with no verdict for the machine.
                    slot.leased.remove(&connection.0);
                    slot.limit.result(connection.attempt(), Outcome::Ended, now);
                }
                Seen::Disposed => {
                    slot.limit.conn(id, ConnEvent::Disposed, now);
                }
            }
            self.sync(&mut book, &operation, index, now);
        }
    }
}
