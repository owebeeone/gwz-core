//! The governor: every site's limit machine for one pool, behind one lock,
//! driving the pool's numbers (adaptive concurrency design §4.9).
//!
//! **One lock, two domains (F10).** The pool host reports each connection's
//! state change, in the order the host acted, through [`Observer`]; the
//! endpoints report the results of attempts (`answered`, `refused`) and ask
//! for admission. All of it passes through the one mutex here, so the
//! machine sees a single order, and every number it then sets on the pool
//! (`set_limit`, `set_settle`, `discard_idle`) is set under that same lock,
//! before the call returns to the endpoint that asked. Lock order is host,
//! then governor, then pool; nothing here calls back into a host.
//!
//! **An attempt is its connection.** A new connection's attempt begins at the
//! socket connect the host reports (§4.3: the window runs from the connect)
//! and a leased exchange's at `exchange_begins`; both are keyed by the pool's
//! connection id, which is also the machine's. The pool enforces the number
//! of connections; the endpoints consult only the gate (a hold, a
//! confirmation, a test in flight) and the number, so an open that can lease
//! an idle connection is never held by a count.
use super::{
    control::{Action, Limit, Outcome, Ruling},
    filter::Signal,
    fsm::State,
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
    collections::BTreeSet,
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
    /// How many connections the pool is allowed on the site: the believed
    /// limit `N`, or the ceiling in SATURATED.
    pub(crate) target: usize,
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

struct Slot {
    site: Site,
    limit: Limit,
    /// The attempts that are exchanges on a leased connection.
    leased: BTreeSet<u64>,
    /// What the pool was last told.
    applied: Option<(usize, u64)>,
}

struct Book {
    ceiling: usize,
    adaptive: bool,
    spread: Box<dyn Fn() -> Spread + Send + Sync>,
    slots: Vec<Slot>,
    notes: Vec<(Site, Note)>,
}

/// A cloneable handle on one pool's limit machines.
#[derive(Clone)]
pub(crate) struct Governor {
    control: PoolControl,
    book: Arc<Mutex<Book>>,
}

/// A pool connection as the machine names it: the pool's sequence number,
/// which is unique within one pool, and a governor serves one pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Conn(pub(crate) u64);
impl Conn {
    pub(crate) fn of(connection: ConnectionId) -> Self {
        Self(connection.sequence())
    }
    fn attempt(self) -> AttemptId {
        AttemptId(self.0)
    }
    fn id(self) -> ConnId {
        ConnId(self.0)
    }
}

impl Governor {
    /// A governor over the pool `control` reaches, with the ceiling `C` of
    /// the operation in force and `adaptive` false when no refusal may lower
    /// `N` (`--max-retries 0`, §5.3). `spread` draws each key's probe-timer
    /// jitter.
    pub(crate) fn new(
        control: PoolControl,
        ceiling: usize,
        adaptive: bool,
        spread: impl Fn() -> Spread + Send + Sync + 'static,
    ) -> Self {
        Self {
            control,
            book: Arc::new(Mutex::new(Book {
                ceiling,
                adaptive,
                spread: Box::new(spread),
                slots: Vec::new(),
                notes: Vec::new(),
            })),
        }
    }
    /// `new`, with the probe timers' jitter drawn from the operating system's
    /// random source.
    pub(crate) fn random(control: PoolControl, ceiling: usize, adaptive: bool) -> Self {
        Self::new(control, ceiling, adaptive, Spread::random)
    }
    fn book(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new operation: every key starts SATURATED at `ceiling`, and the
    /// numbers the last operation set on the pool are cleared.
    pub(crate) fn begin_operation(&self, ceiling: usize, adaptive: bool) {
        let mut book = self.book();
        for slot in book.slots.drain(..) {
            self.control.clear_limit(&slot.site);
            let _ = self.control.set_settle(&slot.site, 0);
        }
        book.ceiling = ceiling;
        book.adaptive = adaptive;
        book.notes.clear();
    }

    /// Whether new connections may start on `key`'s site, and how many the
    /// pool may hold there.
    pub(crate) fn admission(&self, key: &Key, now: u64) -> Admission {
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        self.sync(&mut book, index, now);
        let limit = &mut book.slots[index].limit;
        Admission {
            gate_open: limit.gate_open(now),
            target: limit.pool_limit(),
        }
    }

    /// Whether an open may begin its first exchange (a discovery) on a leased
    /// connection: not during a hold (§4.5).
    pub(crate) fn exchange_may_begin(&self, key: &Key, now: u64) -> bool {
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        self.sync(&mut book, index, now);
        book.slots[index].limit.admits_first_exchange(now)
    }

    /// An exchange begins on the leased connection `connection`: its window
    /// runs from here. False, and nothing begun, during a hold.
    pub(crate) fn exchange_begins(&self, key: &Key, connection: Conn, now: u64) -> bool {
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        self.sync(&mut book, index, now);
        let slot = &mut book.slots[index];
        if !slot.limit.admits_first_exchange(now) {
            return false;
        }
        let target = slot.limit.pool_limit();
        let began = slot.limit.begin(
            connection.attempt(),
            AttemptKind::Ordinary,
            target,
            Own::Leased(connection.id()),
            true,
            now,
        );
        if began {
            slot.leased.insert(connection.0);
        }
        began
    }

    /// The attempt on `connection` succeeded: a new connection's first
    /// exchange was answered (for HTTPS, that is when it is Connected), or a
    /// leased exchange was.
    pub(crate) fn answered(&self, key: &Key, connection: Conn, now: u64) {
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        let slot = &mut book.slots[index];
        let fresh = !slot.leased.remove(&connection.0);
        if fresh {
            slot.limit.conn(connection.id(), ConnEvent::Connected, now);
        }
        slot.limit
            .result(connection.attempt(), Outcome::Succeeded { fresh }, now);
        self.sync(&mut book, index, now);
    }

    /// The attempt on `connection` was refused with `signal`, with a
    /// `Retry-After` of `retry_after_ms` if the server gave one, on a POST
    /// when `post`. `None` when no attempt is in flight on it.
    pub(crate) fn refused(
        &self,
        key: &Key,
        connection: Conn,
        signal: Signal,
        retry_after_ms: Option<u64>,
        post: bool,
        now: u64,
    ) -> Option<Ruling> {
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        let slot = &mut book.slots[index];
        slot.leased.remove(&connection.0);
        let ruling = slot.limit.result(
            connection.attempt(),
            Outcome::Refused {
                signal,
                retry_after_ms,
                post,
            },
            now,
        );
        if ruling.is_none()
            && let Some(wait) = retry_after_ms
        {
            // No attempt was in flight to judge, but the server's word holds.
            slot.limit.set_hold(wait, post, now);
        }
        self.sync(&mut book, index, now);
        ruling
    }

    /// Time has passed: holds end, settles lapse, notes flush.
    pub(crate) fn tick(&self, now: u64) {
        let mut book = self.book();
        for index in 0..book.slots.len() {
            self.sync(&mut book, index, now);
        }
    }

    /// The next time anything in any key changes with no event.
    pub(crate) fn next_deadline(&self, now: u64) -> Option<u64> {
        self.book()
            .slots
            .iter()
            .filter_map(|slot| slot.limit.next_deadline(now))
            .min()
    }

    /// `key`'s site as it stands, if the operation has touched it.
    pub(crate) fn view(&self, key: &Key, now: u64) -> Option<View> {
        let mut book = self.book();
        let index = book.slots.iter().position(|slot| key.site() == slot.site)?;
        self.sync(&mut book, index, now);
        let ceiling = book.ceiling;
        let slot = &mut book.slots[index];
        let table = slot.limit.table();
        Some(View {
            state: slot.limit.state(),
            n: slot.limit.n(),
            ceiling,
            connected: table.connected(),
            possible: table.possible(),
            pool_limit: slot.limit.pool_limit(),
            settle_ms: slot.limit.settle_ms(),
            confirmation: slot.limit.confirmation_open(),
        })
    }

    /// The notes the machines have made since the last call (§9).
    pub(crate) fn drain_notes(&self) -> Vec<(Site, Note)> {
        std::mem::take(&mut self.book().notes)
    }

    /// The index of `key`'s site, made on its first use.
    fn slot(&self, book: &mut Book, key: &Key) -> usize {
        let site = key.site();
        if let Some(index) = book.slots.iter().position(|slot| slot.site == site) {
            return index;
        }
        let limit = Limit::new(book.ceiling, book.adaptive, (book.spread)());
        book.slots.push(Slot {
            site,
            limit,
            leased: BTreeSet::new(),
            applied: None,
        });
        book.slots.len() - 1
    }

    /// Brings time forward on one site, does what the machine asks of the
    /// pool, and tells the pool its numbers when they changed.
    fn sync(&self, book: &mut Book, index: usize, now: u64) {
        let slot = &mut book.slots[index];
        if slot.limit.tick(now) == Some(Action::DiscardIdle) {
            // One step under the pool's lock, before the hold lifts.
            self.control.discard_idle(&slot.site);
            slot.limit.idle_discarded(now);
        }
        let wanted = (slot.limit.pool_limit(), slot.limit.settle_ms());
        if slot.applied != Some(wanted) {
            let _ = self.control.set_limit(&slot.site, wanted.0);
            let _ = self.control.set_settle(&slot.site, wanted.1);
            slot.applied = Some(wanted);
        }
        let notes = slot.limit.drain_notes();
        let site = slot.site.clone();
        book.notes
            .extend(notes.into_iter().map(|note| (site.clone(), note)));
        let excess = book.notes.len().saturating_sub(NOTES_KEPT);
        book.notes.drain(..excess);
    }
}

impl Observer for Governor {
    fn seen(&self, key: &Key, connection: ConnectionId, seen: Seen, now: u64) {
        let connection = Conn::of(connection);
        let mut book = self.book();
        let index = self.slot(&mut book, key);
        let slot = &mut book.slots[index];
        let id = connection.id();
        match seen {
            Seen::Started { clocked } => {
                let target = slot.limit.pool_limit();
                let began = slot.limit.begin(
                    connection.attempt(),
                    AttemptKind::Ordinary,
                    target,
                    Own::New(id),
                    clocked,
                    now,
                );
                if !began {
                    slot.limit.conn(id, ConnEvent::Started { clocked }, now);
                }
            }
            // HTTPS is set up when its first exchange is answered, which the
            // endpoint reports; SSH when it is authenticated, which is this.
            Seen::Connected if key.scheme.wire() == Scheme::Https.wire() => {}
            Seen::Connected => {
                slot.limit.conn(id, ConnEvent::Connected, now);
                slot.limit.result(
                    connection.attempt(),
                    Outcome::Succeeded { fresh: true },
                    now,
                );
            }
            Seen::SetupEnded | Seen::Retired | Seen::Closing | Seen::ServerClosed => {
                let event = match seen {
                    Seen::SetupEnded => ConnEvent::SetupEnded,
                    Seen::Retired => ConnEvent::Retired,
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
        self.sync(&mut book, index, now);
    }
}
