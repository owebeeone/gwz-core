//! One key's limit discovery: the state table, the windows, the evidence
//! filter, the machine, the probe timer, the hold, the confirmation and the
//! test slot, behind events and queries (§4.5 to §4.7). Time is a number
//! the caller supplies; the clock and the jitter are injected. The parts of a
//! key are in the child modules: RESTORING's steps (`restore`), SSH's setup
//! limit `Ns` (`setup_limit`), the test slot (`test_slot`) and an attempt's
//! result (`verdict`).
use super::{
    filter::Signal,
    fsm::{Fsm, State},
    hold::{Hold, Origin},
    notes::{Note, Notes},
    states::{ConnEvent, ConnId, Table},
    timer::{Spread, Timer},
    windows::{AttemptId, AttemptKind, Own, Windows},
};
use std::collections::{BTreeMap, BTreeSet};

mod restore;
mod setup_limit;
mod test_slot;
mod verdict;

use restore::Restore;
use setup_limit::SetupLimit;

/// What an admitted attempt is. A restore step carries its target `S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    Ordinary,
    Probe,
    Confirming,
    Restore {
        target: usize,
    },
    /// A test of the setup limit `Ns` (§4.10).
    SetupProbe,
}

/// How an attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// `fresh` is false when the pool leased an idle connection: no setup.
    Succeeded { fresh: bool },
    /// A Throttle or Suspect refusal. `retry_after_ms` sets the hold, `post`
    /// says it came on a POST.
    Refused {
        signal: Signal,
        retry_after_ms: Option<u64>,
        post: bool,
    },
    /// Ended with no verdict for this machine: Queue, Local, Transient,
    /// Permanent, cancelled, timed out.
    Ended,
}

/// What a result was to the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ruling {
    Succeeded,
    /// A confirming test succeeded: the Suspect is refuted.
    Refuted,
    /// An unfair test, or a test served by reuse: re-armed.
    Unfair,
    Ended,
    RetryMachine,
    Inconclusive,
    HoldOnly,
    Overload {
        n: usize,
    },
    Confirmation,
    RefusedTest,
    /// A drop before authentication opened a setup window: new setups are
    /// held until the co-setups it overlapped have results (§4.10).
    SetupLimit,
}

/// A test that is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TestPlan {
    pub(crate) kind: AttemptKind,
    /// The admission target: `N + 1`, or `k = Connected + 1`.
    pub(crate) target: usize,
    /// The key is quiet and the test's base holds: it can start now.
    pub(crate) ready: bool,
}

/// What the endpoint must do for the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// A long hold has ended: discard the key's idle connections, then call
    /// [`Limit::idle_discarded`]. The hold lifts after that.
    DiscardIdle,
}

pub(crate) struct Limit {
    ceiling: usize,
    adaptive: bool,
    table: Table,
    windows: Windows,
    fsm: Fsm,
    timer: Timer,
    hold: Hold,
    notes: Notes,
    targets: BTreeMap<AttemptId, usize>,
    test_slot: Option<AttemptId>,
    /// A test an endpoint has started for a carrier whose connection the host
    /// has not yet begun: the next connection on the site is the test (§4.9:
    /// the pool's limit is raised under the same lock as the admission).
    armed: Option<(AttemptKind, usize, u64)>,
    /// Which arming a carrier's token may give back.
    arms: u64,
    confirmation: bool,
    barrier: BTreeSet<ConnId>,
    needing_new: usize,
    non_final: usize,
    /// The `N` in force at the last success (§5.5's `N_good`), or the ceiling
    /// before any.
    last_good: usize,
    /// `N_good` as frozen when the retry machine left Cold or Healthy: set
    /// for the length of an outage.
    outage: Option<usize>,
    restore: Option<Restore>,
    /// SSH's setup limit `Ns` (§4.10).
    setups: SetupLimit,
}

impl Limit {
    /// A key with ceiling `C`. `adaptive` is false at `--max-retries 0`.
    pub(crate) fn new(ceiling: usize, adaptive: bool, spread: Spread) -> Self {
        let setups = SetupLimit::new(ceiling, spread.clone());
        Self {
            ceiling,
            adaptive,
            table: Table::new(),
            windows: Windows::new(),
            fsm: Fsm::new(ceiling),
            timer: Timer::new(spread),
            hold: Hold::default(),
            notes: Notes::default(),
            targets: BTreeMap::new(),
            test_slot: None,
            armed: None,
            arms: 0,
            confirmation: false,
            barrier: BTreeSet::new(),
            needing_new: 0,
            non_final: 0,
            last_good: ceiling,
            outage: None,
            restore: None,
            setups,
        }
    }
    pub(crate) fn n(&self) -> usize {
        self.fsm.n()
    }
    pub(crate) fn state(&self) -> State {
        self.fsm.state()
    }
    pub(crate) fn table(&self) -> &Table {
        &self.table
    }
    pub(crate) fn confirmation_open(&self) -> bool {
        self.confirmation
    }
    /// The queued members that need a new connection, and how many of them
    /// are not on their final attempt (the probe's carriers).
    pub(crate) fn set_demand(&mut self, needing_new: usize, non_final: usize, now: u64) {
        self.needing_new = needing_new;
        self.non_final = non_final;
        // A restore step's starts are carried by members not on their final
        // attempt. With none to carry it and nothing in flight, the restore
        // gives the rest of the distance to the probe timer.
        if needing_new > 0
            && non_final == 0
            && self.restore.as_ref().is_some_and(|r| r.step.is_empty())
        {
            self.end_restore(now);
        }
    }
    /// Starts from the table of connections the site already has, which no
    /// event of this machine's own life told it about.
    pub(crate) fn seed(&mut self, table: Table) {
        self.table = table;
    }
    /// The server held every connection now Connected at once (§4.2): another
    /// operation's connection was answered, which is this machine's evidence
    /// as much as its own. `N := min(C, max(N, Connected))`.
    pub(crate) fn observed_success(&mut self, now: u64) {
        let before = (self.fsm.n(), self.fsm.state());
        self.table.advance(now);
        self.fsm.success(self.table.connected());
        self.last_good = self.fsm.n();
        self.settled(before, now);
    }
    pub(crate) fn connect_time(&mut self, ms: u64) {
        self.table.settle_mut().observe_connect(ms);
    }
    /// Reports a connection's state change, in the order it happened.
    pub(crate) fn conn(&mut self, conn: ConnId, event: ConnEvent, now: u64) {
        if let Some(change) = self.table.apply(conn, event, now) {
            self.windows.observe(&change);
            self.setup_changed(&change, now);
        }
        self.flush_notes(now);
    }
    /// Whether an ordinary start on a new connection may begin now (§4.5).
    /// Outside SATURATED a due test fills only to its base, `Possible < N`,
    /// which is the same rule; a confirmation or a test in flight allows no
    /// start at all.
    pub(crate) fn admits_ordinary(&mut self, now: u64) -> bool {
        if !self.gate_open(now) {
            return false;
        }
        self.n_room() && self.setup_room()
    }
    /// The room `N` leaves: SATURATED's ceiling rule, else `Possible < N`.
    pub(super) fn n_room(&self) -> bool {
        match self.fsm.state() {
            State::Saturated => self.table.held() < self.ceiling,
            _ => self.table.possible() < self.room_target() && self.step_has_starts(),
        }
    }
    /// Whether the RESTORING step in progress may still start a connection
    /// (State P3-2); always true outside RESTORING.
    fn step_has_starts(&self) -> bool {
        self.restore
            .as_ref()
            .is_none_or(|restore| restore.quota > 0)
    }
    /// How many connections may be Possible before ordinary starts stop: `N`,
    /// or the restore step's target.
    fn room_target(&self) -> usize {
        self.restore.as_ref().map_or(self.fsm.n(), |r| r.target)
    }
    /// Whether the site has room for one more connection of this operation
    /// below `N` (§4.5). At the ceiling the pool's own limit says so.
    pub(crate) fn has_room(&self) -> bool {
        (self.fsm.state() == State::Saturated
            || (self.table.possible() < self.room_target() && self.step_has_starts()))
            && self.setup_room()
    }
    /// Whether new connections may start at all: no hold, no confirmation,
    /// no test in flight, and no connection of an inconclusive refusal's
    /// window still winding down (§4.4). It does not count connections: the
    /// pool's limit bounds those, and an open that can lease an idle
    /// connection is not a start.
    pub(crate) fn gate_open(&mut self, now: u64) -> bool {
        self.table.advance(now);
        if self.hold.in_force(now)
            || self.confirmation
            || self.setups.holds()
            || self.test_slot.is_some()
            || self.armed.is_some()
        {
            return false;
        }
        if !self.barrier.is_empty() {
            if self.table.any_winding_down(&self.barrier) {
                return false;
            }
            self.barrier.clear();
        }
        true
    }
    /// Whether an open may begin its first exchange (a discovery) on a
    /// leased connection: not during a hold.
    pub(crate) fn admits_first_exchange(&mut self, now: u64) -> bool {
        self.table.advance(now);
        !self.hold.in_force(now)
    }
    /// Whether a member past its discovery may send its next exchange: not
    /// while a POST-set hold is in force.
    pub(crate) fn admits_continuing(&mut self, now: u64) -> bool {
        self.table.advance(now);
        !self.hold.blocks_continuing(now)
    }
    /// Admits an attempt, opens its window, and starts its connection in the
    /// table when it is new. Returns the admission target (the pool's
    /// limit), or `None` when the rules do not admit it.
    pub(crate) fn admit(
        &mut self,
        attempt: AttemptId,
        start: Start,
        own: Own,
        clocked: bool,
        now: u64,
    ) -> Option<usize> {
        if self.targets.contains_key(&attempt) {
            return None;
        }
        let (kind, target) = match start {
            Start::Ordinary => {
                let admitted = match own {
                    Own::New(_) => self.admits_ordinary(now),
                    Own::Leased(_) => self.admits_first_exchange(now),
                };
                let target = if self.fsm.state() == State::Saturated {
                    self.ceiling
                } else {
                    self.fsm.n()
                };
                (admitted.then_some(AttemptKind::Ordinary)?, target)
            }
            Start::Probe | Start::Confirming => {
                let wanted = if start == Start::Probe {
                    AttemptKind::Probe
                } else {
                    AttemptKind::Confirming
                };
                let plan = self
                    .test_due(now)
                    .filter(|plan| plan.kind == wanted && plan.ready)?;
                matches!(own, Own::New(_)).then_some((wanted, plan.target))?
            }
            Start::SetupProbe => {
                let plan = self
                    .setup_test(now)
                    .filter(|plan| plan.ready && matches!(own, Own::New(_)))?;
                (AttemptKind::SetupProbe, plan.target)
            }
            Start::Restore { target } => (
                self.admits_first_exchange(now)
                    .then_some(AttemptKind::Restore)?,
                target,
            ),
        };
        self.begin(attempt, kind, target, own, clocked, now)
            .then_some(target)
    }
    /// Opens `attempt`'s window with `kind` and `target`, and starts its
    /// connection in the table when it is new, without asking admission
    /// again. The pool has made the connection under its own limit, and the
    /// window runs from its socket connect (§4.3), so the endpoint that sees
    /// the connect calls this. False, changing nothing, if the attempt is
    /// already in flight.
    pub(crate) fn begin(
        &mut self,
        attempt: AttemptId,
        kind: AttemptKind,
        target: usize,
        own: Own,
        clocked: bool,
        now: u64,
    ) -> bool {
        if self.targets.contains_key(&attempt)
            || !self.windows.open(attempt, kind, own, &self.table)
        {
            return false;
        }
        self.targets.insert(attempt, target);
        if kind.is_test() {
            self.test_slot = Some(attempt);
        }
        if kind == AttemptKind::Probe {
            self.fsm.test_started();
        }
        if kind == AttemptKind::SetupProbe {
            self.setup_test_began(attempt);
        }
        if kind == AttemptKind::Restore
            && let Some(restore) = &mut self.restore
        {
            restore.step.insert(attempt);
            restore.started = true;
            if matches!(own, Own::New(_)) {
                restore.quota = restore.quota.saturating_sub(1);
            }
        }
        if let Own::New(conn) = own {
            self.conn(conn, ConnEvent::Started { clocked }, now);
        }
        true
    }
    /// Whether a hold is in force at `now`: the server's word, which no
    /// other operation's start may erase.
    pub(crate) fn holding(&self, now: u64) -> bool {
        self.hold.in_force(now)
    }
    /// The hold, if one is in force at `now`: what a machine of the same key
    /// in another operation inherits.
    pub(crate) fn hold_in_force(&self, now: u64) -> Option<Hold> {
        self.hold.in_force(now).then(|| self.hold.clone())
    }
    /// The attempt's connection ended: its window stops observing, and its
    /// verdict, if there is one, is asked for later.
    pub(crate) fn freeze(&mut self, attempt: AttemptId) {
        self.windows.freeze(attempt);
    }
    /// Takes the hold out, leaving none: a key kept across an operation's end
    /// for its hold alone carries it into a fresh `Limit`.
    pub(crate) fn take_hold(&mut self) -> Hold {
        std::mem::take(&mut self.hold)
    }
    /// Puts back a hold taken from another `Limit` of the same key.
    pub(crate) fn restore_hold(&mut self, hold: Hold) {
        self.hold = hold;
    }
    /// The number of connections the pool may hold on the key (§4.9): the
    /// machine's `N`, or the target of the test in flight. In SATURATED it is
    /// the ceiling.
    pub(crate) fn pool_limit(&self) -> usize {
        // A setup test counts setups, not connections: the pool's limit is
        // what `N` says.
        self.test_slot
            .filter(|attempt| !self.setups.is_test(*attempt))
            .and_then(|attempt| self.targets.get(&attempt).copied())
            .or(self
                .armed
                .filter(|(kind, ..)| *kind != AttemptKind::SetupProbe)
                .map(|(_, target, _)| target))
            .unwrap_or_else(|| self.room_target())
    }
    /// How long the pool holds a freed slot of the key (§4.5): `Ts` outside
    /// SATURATED, none at the ceiling.
    pub(crate) fn settle_ms(&self) -> u64 {
        if self.fsm.state() == State::Saturated {
            0
        } else {
            self.table.settle().ts()
        }
    }
    /// A `Retry-After` that came on a response with no attempt in flight (a
    /// second request on a carried connection): the hold, and nothing else
    /// (§4.5: any 429 or 503 carrying `Retry-After` sets the key's hold,
    /// whatever its judgement).
    pub(crate) fn set_hold(&mut self, retry_after_ms: u64, post: bool, now: u64) {
        self.table.advance(now);
        let origin = if post {
            Origin::Post
        } else {
            Origin::Discovery
        };
        if let Some(wait) = self.hold.set(retry_after_ms, origin, now) {
            self.notes.hold(wait);
        }
        self.flush_notes(now);
    }
    fn flush_notes(&mut self, now: u64) {
        let resolved = self.table.setups_in_flight() == 0;
        self.notes.flush(now, self.fsm.n(), self.ceiling, resolved);
    }
    /// The endpoint discarded the key's idle connections after a long hold.
    pub(crate) fn idle_discarded(&mut self, now: u64) {
        self.hold.idle_discarded();
        self.flush_notes(now);
    }
    /// Advances time: settles, ends holds, flushes notes.
    pub(crate) fn tick(&mut self, now: u64) -> Option<Action> {
        self.table.advance(now);
        let action = self.hold.discard_due(now).then_some(Action::DiscardIdle);
        self.flush_notes(now);
        action
    }
    /// The next time anything here changes with no event: a hold, a settle,
    /// the probe timer, a coalesced note.
    pub(crate) fn next_deadline(&self, now: u64) -> Option<u64> {
        let probe = (self.fsm.state() == State::Stable && self.test_slot.is_none())
            .then(|| self.timer.deadline())
            .flatten()
            .filter(|deadline| *deadline > now);
        [
            self.hold.deadline(now),
            self.table.next_settle_deadline(),
            probe,
            self.setups.deadline().filter(|deadline| *deadline > now),
            self.notes.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
    /// The key is Down (§5.5): the note of this episode, once.
    pub(crate) fn down(&mut self, now: u64) {
        self.notes.down();
        self.flush_notes(now);
    }
    pub(crate) fn drain_notes(&mut self) -> Vec<Note> {
        self.notes.drain()
    }
}
