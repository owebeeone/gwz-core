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
//! **The site's connections are not an operation's.** The governor keeps one
//! table of the connections on each site, fed by every event whether or not
//! any operation is live; a machine starts from a copy of it, so an operation
//! that begins while connections exist (another operation's, or the last
//! one's, left idle) sees what the server holds.
//!
//! **An attempt is its connection, and a connection is tagged with its member.**
//! A new connection's attempt begins at the socket connect the host reports
//! (§4.3: the window runs from the connect) and a leased exchange's at
//! `exchange_begins`; both are keyed by the pool's connection id, which is also
//! the machine's. The request that opened a connection carries a tag naming
//! its operation and member ([`label`]), which the pool hands back on the
//! connect: that is how an armed test is taken only by its own operation's
//! connection, and how a failed setup is judged on its own window, by the
//! member whose endpoint reports it, whether the host has seen the connect end
//! yet or not. The pool enforces the number of connections; the endpoints
//! consult only the gate (a hold, a confirmation, a test in flight) and the
//! number, so an open that can lease an idle connection is never held by a
//! count.

use super::{
    control::{Action, Limit, Outcome},
    fsm::State,
    hold::Hold,
    notes::Note,
    states::{ConnEvent, ConnId, Table},
    timer::Spread,
    windows::{AttemptId, Own},
};
use crate::git::endpoint::ssh_pool::{Observer, Seen};
use gwz_transport::{
    pool::{ConnectionId, Key, PoolControl, Site},
    protocol::Scheme,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
};

/// Notes kept for the reader that drains them (§9); the oldest are dropped.
const NOTES_KEPT: usize = 64;

/// How long a setup the host saw end waits for its endpoint's word before it
/// counts as an attempt that ended with no verdict.
const ENDED_TTL_MS: u64 = 1_000;

/// The tag a request carries to its connection: the operation it belongs to
/// and the member it serves.
pub(crate) fn label(operation: &str, member: &str) -> String {
    format!("{operation}\u{1f}{member}")
}
fn operation_of(tag: &str) -> &str {
    tag.split_once('\u{1f}')
        .map_or(tag, |(operation, _)| operation)
}

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
    /// A restore after an outage is under way (§5.5): a new connection is
    /// started only by a member not on its final attempt.
    pub(crate) restoring: bool,
}

impl Admission {
    /// Whether a member waits for a new connection although the gate is open:
    /// a restore step's start is carried by a member not on its final attempt,
    /// so a restore never fails a member (§5.5). A member that can lease an
    /// idle connection starts nothing, and a test's carrier is the test's.
    pub(crate) fn holds_final_attempt(
        &self,
        final_attempt: bool,
        may_lease_idle: bool,
        carries_test: bool,
    ) -> bool {
        self.restoring && final_attempt && !may_lease_idle && !carries_test
    }
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
    /// The key's retry machine is down or recovering, and `N_good` is frozen
    /// (§5.5).
    pub(crate) outage: bool,
}

pub(super) struct Slot {
    pub(super) site: Site,
    pub(super) limit: Limit,
    /// The attempts that are exchanges on a leased connection.
    pub(super) leased: BTreeSet<u64>,
    /// Whether the machine wants the pool to evict nothing on the site.
    pub(super) suppress: bool,
}

/// One operation's machines.
pub(super) struct Scope {
    pub(super) ceiling: usize,
    pub(super) adaptive: bool,
    pub(super) slots: Vec<Slot>,
    pub(super) notes: Vec<(Site, Note)>,
}

/// Where a connection stands for the one who judges its setup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    /// Setting up; its member's endpoint may report a failure at any time.
    Live,
    /// The host saw the setup end at this time; the report is still to come.
    Ended(u64),
    /// Its member's endpoint reported first and the machine judged it then.
    Judged,
    /// Set up.
    Up,
}
pub(super) struct Info {
    pub(super) tag: Option<String>,
    pub(super) stage: Stage,
}

/// What the server holds on one site, whoever's it is.
pub(super) struct SiteState {
    pub(super) site: Site,
    pub(super) table: Table,
    pub(super) conns: BTreeMap<u64, Info>,
}

pub(super) struct Book {
    /// What an operation not yet begun is given (the pool's own cap).
    default_ceiling: usize,
    spread: Box<dyn Fn() -> Spread + Send + Sync>,
    pub(super) scopes: BTreeMap<String, Scope>,
    pub(super) sites: Vec<SiteState>,
    /// What the pool was last told for each site: limit, settle ms, no-evict.
    applied: Vec<(Site, (usize, u64, bool))>,
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

/// A started test, which the carrier holds until it has a connection of its
/// own or ends: dropping it gives the test back if it was never taken, on
/// every exit, so a carrier that fails before reaching the pool cannot leave
/// the site's gate shut (§5.2).
#[must_use = "dropping the token gives the test back"]
pub(crate) struct TestToken {
    governor: Governor,
    operation: String,
    site: Site,
    arm: u64,
}
impl TestToken {
    pub(super) fn new(governor: &Governor, operation: &str, site: Site, arm: u64) -> Self {
        Self {
            governor: governor.clone(),
            operation: operation.to_owned(),
            site,
            arm,
        }
    }
}
impl Drop for TestToken {
    fn drop(&mut self) {
        let mut book = self.governor.book();
        if let Some(scope) = book.scopes.get_mut(&self.operation)
            && let Some(slot) = scope.slots.iter_mut().find(|slot| slot.site == self.site)
        {
            slot.limit.disarm(self.arm);
        }
        self.governor.reconcile(&mut book, &self.site);
    }
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
    /// The index of `site`'s state, made on its first event.
    pub(super) fn default_ceiling(&self) -> usize {
        self.default_ceiling
    }
    pub(super) fn site_index(&mut self, site: &Site) -> usize {
        if let Some(index) = self.sites.iter().position(|state| &state.site == site) {
            return index;
        }
        self.sites.push(SiteState {
            site: site.clone(),
            table: Table::new(),
            conns: BTreeMap::new(),
        });
        self.sites.len() - 1
    }

    /// The index of `site`'s slot in `operation`'s scope, made on first use.
    /// `None` when the operation is not live: a scope is made only by
    /// `begin_operation`, so a late report for an ended one revives nothing.
    /// A new machine starts from the site's table, and inherits any hold in
    /// force on the site: from the other live operations, or one an ended
    /// operation left.
    pub(super) fn slot(&mut self, operation: &str, site: &Site, now: u64) -> Option<usize> {
        let scope = self.scopes.get(operation)?;
        if let Some(index) = scope.slots.iter().position(|slot| &slot.site == site) {
            return Some(index);
        }
        let (ceiling, adaptive) = (scope.ceiling, scope.adaptive);
        let mut limit = Limit::new(ceiling, adaptive, (self.spread)());
        let seeded = self.site_index(site);
        limit.seed(self.sites[seeded].table.clone());
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
        let scope = self.scopes.get_mut(operation)?;
        scope.slots.push(Slot {
            site: site.clone(),
            limit,
            leased: BTreeSet::new(),
            suppress: false,
        });
        Some(scope.slots.len() - 1)
    }

    /// Every live operation's slot on `site`, as `(operation, index)`.
    pub(super) fn slots_on(&self, site: &Site) -> Vec<(String, usize)> {
        self.scopes
            .iter()
            .flat_map(|(name, scope)| {
                scope
                    .slots
                    .iter()
                    .position(|slot| &slot.site == site)
                    .map(|index| (name.clone(), index))
            })
            .collect()
    }
}

impl Scope {
    fn new(ceiling: usize) -> Self {
        Self {
            ceiling,
            adaptive: false,
            slots: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl Governor {
    /// A governor over the pool `control` reaches. `ceiling` is the pool's own
    /// cap, which an unscoped report is answered with. `spread` draws each
    /// key's probe-timer jitter.
    pub(crate) fn new(
        control: PoolControl,
        ceiling: usize,
        spread: impl Fn() -> Spread + Send + Sync + 'static,
    ) -> Self {
        Self {
            control,
            book: Arc::new(Mutex::new(Book {
                default_ceiling: ceiling,
                spread: Box::new(spread),
                scopes: BTreeMap::new(),
                sites: Vec::new(),
                applied: Vec::new(),
                held_over: Vec::new(),
            })),
        }
    }
    /// `new`, with the probe timers' jitter drawn from the operating system's
    /// random source.
    pub(crate) fn random(control: PoolControl, ceiling: usize) -> Self {
        Self::new(control, ceiling, Spread::random)
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
        let mut scope = Scope::new(ceiling);
        scope.adaptive = adaptive;
        book.scopes.insert(operation.to_owned(), scope);
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
        if slot.limit.tick(now) == Some(Action::DiscardIdle) {
            // One step under the pool's lock, before the hold lifts.
            self.control.discard_idle(&slot.site);
            slot.limit.idle_discarded(now);
        }
        slot.suppress = slot.limit.closes_suppressed(now);
        let notes = slot.limit.drain_notes();
        let site = slot.site.clone();
        scope
            .notes
            .extend(notes.into_iter().map(|note| (site.clone(), note)));
        let excess = scope.notes.len().saturating_sub(NOTES_KEPT);
        scope.notes.drain(..excess);
        self.sweep(book, &site, now);
        self.reconcile(book, &site);
    }

    /// Syncs every live operation's slot on `site`.
    pub(super) fn sync_site(&self, book: &mut Book, site: &Site, now: u64) {
        for (operation, index) in book.slots_on(site) {
            self.sync(book, &operation, index, now);
        }
    }

    /// A setup the host saw end, whose endpoint never said why, ends its
    /// attempt with no verdict once its time is up.
    fn sweep(&self, book: &mut Book, site: &Site, now: u64) {
        let index = book.site_index(site);
        let expired: Vec<u64> = book.sites[index]
            .conns
            .iter()
            .filter(|(_, info)| {
                matches!(info.stage, Stage::Ended(at) if now.saturating_sub(at) >= ENDED_TTL_MS)
            })
            .map(|(conn, _)| *conn)
            .collect();
        for conn in expired {
            let info = book.sites[index].conns.remove(&conn);
            let owner = info.and_then(|info| info.tag);
            let Some(owner) = owner.as_deref().map(operation_of) else {
                continue;
            };
            if let Some(i) = book.slot(owner, site, now)
                && let Some(scope) = book.scopes.get_mut(owner)
            {
                scope.slots[i]
                    .limit
                    .result(AttemptId(conn), Outcome::Ended, now);
            }
        }
    }

    /// Tells the pool `site`'s numbers when they changed: the most
    /// permissive limit and the longest settle of the live machines, evictions
    /// stopped if any machine wants them stopped, or nothing when none is left.
    pub(super) fn reconcile(&self, book: &mut Book, site: &Site) {
        let wanted = book
            .scopes
            .values()
            .flat_map(|scope| &scope.slots)
            .filter(|slot| &slot.site == site)
            .map(|slot| {
                (
                    slot.limit.pool_limit(),
                    slot.limit.settle_ms(),
                    slot.suppress,
                )
            })
            .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1), a.2 || b.2));
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
                let _ = self.control.set_no_evict(site, false);
            }
            _ => {}
        }
    }

    fn tell_pool(&self, site: &Site, (limit, settle_ms, no_evict): (usize, u64, bool)) {
        let _ = self.control.set_limit(site, limit);
        let _ = self.control.set_settle(site, settle_ms);
        let _ = self.control.set_no_evict(site, no_evict);
    }
}

impl Observer for Governor {
    fn started(
        &self,
        key: &Key,
        connection: ConnectionId,
        tag: Option<&str>,
        clocked: bool,
        now: u64,
    ) {
        let connection = Conn::of(connection);
        let site = key.site();
        let mut book = self.book();
        let state = book.site_index(&site);
        let table = &mut book.sites[state];
        table
            .table
            .apply(connection.id(), ConnEvent::Started { clocked }, now);
        table.conns.insert(
            connection.0,
            Info {
                tag: tag.map(str::to_owned),
                stage: Stage::Live,
            },
        );
        let owner = tag.map(operation_of);
        let operations: Vec<String> = book.scopes.keys().cloned().collect();
        for operation in operations {
            let Some(index) = book.slot(&operation, &site, now) else {
                continue;
            };
            let slot = &mut book.scopes.get_mut(&operation).expect("scope").slots[index];
            // A test an endpoint armed for its carrier is the next connection
            // that carrier's operation starts; any other start is an ordinary
            // one, whoever's.
            let (kind, target) = if owner == Some(operation.as_str()) {
                slot.limit.take_armed()
            } else {
                None
            }
            .unwrap_or_else(|| slot.limit.start_kind());
            let began = slot.limit.begin(
                connection.attempt(),
                kind,
                target,
                Own::New(connection.id()),
                clocked,
                now,
            );
            if !began {
                slot.limit
                    .conn(connection.id(), ConnEvent::Started { clocked }, now);
            }
            self.sync(&mut book, &operation, index, now);
        }
    }

    fn seen(&self, key: &Key, connection: ConnectionId, seen: Seen, now: u64) {
        let connection = Conn::of(connection);
        let site = key.site();
        let id = connection.id();
        let mut book = self.book();
        let state = book.site_index(&site);
        if let Seen::TcpConnected { ms } = seen {
            // The settle time follows the TCP connect (§4.1), reported by the
            // host when the socket connect completed: not the setup after it.
            book.sites[state].table.settle_mut().observe_connect(ms);
            for operation in book.scopes.keys().cloned().collect::<Vec<_>>() {
                if let Some(index) = book.slot(&operation, &site, now) {
                    let slot = &mut book.scopes.get_mut(&operation).expect("scope").slots[index];
                    slot.limit.connect_time(ms);
                }
            }
            return;
        }
        let event = match seen {
            // HTTPS is set up when its first exchange is answered, which the
            // endpoint reports; SSH when it is authenticated, which is this.
            Seen::Started { .. } | Seen::TcpConnected { .. } => return,
            Seen::Connected if key.scheme.wire() == Scheme::Https.wire() => return,
            Seen::Connected => ConnEvent::Connected,
            Seen::SetupEnded => ConnEvent::SetupEnded,
            Seen::Retired => ConnEvent::Retired,
            Seen::Closing => ConnEvent::Closing,
            Seen::ServerClosed => ConnEvent::ServerClosed,
            Seen::Disposed => ConnEvent::Disposed,
        };
        let sites = &mut book.sites[state];
        sites.table.apply(id, event, now);
        let mut owner = None;
        let mut judged = false;
        match (seen, sites.conns.get_mut(&connection.0)) {
            (Seen::Connected, Some(info)) => {
                info.stage = Stage::Up;
            }
            (Seen::SetupEnded | Seen::Retired, Some(info)) => {
                owner = info.tag.as_deref().map(|tag| operation_of(tag).to_owned());
                judged = info.stage == Stage::Judged;
                if judged {
                    sites.conns.remove(&connection.0);
                } else {
                    info.stage = Stage::Ended(now);
                }
            }
            (Seen::Closing | Seen::ServerClosed, Some(_)) => {
                sites.conns.remove(&connection.0);
            }
            _ => {}
        }
        let operations: Vec<String> = book.scopes.keys().cloned().collect();
        for operation in operations {
            let Some(index) = book.slot(&operation, &site, now) else {
                continue;
            };
            let slot = &mut book.scopes.get_mut(&operation).expect("scope").slots[index];
            match seen {
                Seen::Started { .. } | Seen::TcpConnected { .. } => {}
                // Authenticated is a fact about the server, so every
                // operation's machine takes it as its success.
                Seen::Connected => {
                    slot.limit.conn(id, ConnEvent::Connected, now);
                    slot.limit.result(
                        connection.attempt(),
                        Outcome::Succeeded { fresh: true },
                        now,
                    );
                }
                // A setup that ended: the server refused it, or the client
                // gave up on it. The window stops here, and its member's
                // endpoint says why (`Scoped::setup_failed`); the others have
                // no one to say.
                Seen::SetupEnded | Seen::Retired => {
                    slot.limit.conn(id, event, now);
                    slot.leased.remove(&connection.0);
                    if owner.as_deref() == Some(operation.as_str()) {
                        slot.limit.freeze(connection.attempt());
                    } else {
                        slot.limit.result(connection.attempt(), Outcome::Ended, now);
                    }
                }
                Seen::Closing | Seen::ServerClosed => {
                    slot.limit.conn(id, event, now);
                    // The connection left with its attempt unanswered: the
                    // attempt ends with no verdict for the machine.
                    slot.leased.remove(&connection.0);
                    slot.limit.result(connection.attempt(), Outcome::Ended, now);
                }
                Seen::Disposed => {
                    slot.limit.conn(id, event, now);
                }
            }
            self.sync(&mut book, &operation, index, now);
        }
        self.reconcile(&mut book, &site);
    }
}
