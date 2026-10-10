//! One key's limit discovery: the state table, the windows, the evidence
//! filter, the machine, the probe timer, the hold, the confirmation and the
//! test slot, behind events and queries (§4.5 to §4.7). Time is a number
//! the caller supplies; the clock and the jitter are injected.
use super::{
    filter::{Evidence, Refusal, Signal, evidence, fair},
    fsm::{Backoff, Fsm, State},
    hold::{Hold, Origin},
    notes::{Note, Notes},
    states::{ConnEvent, ConnId, Table},
    timer::{Spread, Timer},
    windows::{AttemptId, AttemptKind, Closed, Own, Windows},
};
use std::collections::{BTreeMap, BTreeSet};

/// What an admitted attempt is. A restore step carries its target `S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    Ordinary,
    Probe,
    Confirming,
    Restore { target: usize },
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
    confirmation: bool,
    barrier: BTreeSet<ConnId>,
    needing_new: usize,
    non_final: usize,
}

impl Limit {
    /// A key with ceiling `C`. `adaptive` is false at `--max-retries 0`.
    pub(crate) fn new(ceiling: usize, adaptive: bool, spread: Spread) -> Self {
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
            confirmation: false,
            barrier: BTreeSet::new(),
            needing_new: 0,
            non_final: 0,
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
    pub(crate) fn set_demand(&mut self, needing_new: usize, non_final: usize) {
        self.needing_new = needing_new;
        self.non_final = non_final;
    }
    pub(crate) fn connect_time(&mut self, ms: u64) {
        self.table.settle_mut().observe_connect(ms);
    }
    /// Reports a connection's state change, in the order it happened.
    pub(crate) fn conn(&mut self, conn: ConnId, event: ConnEvent, now: u64) {
        if let Some(change) = self.table.apply(conn, event, now) {
            self.windows.observe(&change);
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
        match self.fsm.state() {
            State::Saturated => self.table.held() < self.ceiling,
            _ => self.table.possible() < self.fsm.n(),
        }
    }
    /// Whether new connections may start at all: no hold, no confirmation,
    /// no test in flight, and no connection of an inconclusive refusal's
    /// window still winding down (§4.4). It does not count connections: the
    /// pool's limit bounds those, and an open that can lease an idle
    /// connection is not a start.
    pub(crate) fn gate_open(&mut self, now: u64) -> bool {
        self.table.advance(now);
        if self.hold.in_force(now) || self.confirmation || self.test_slot.is_some() {
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
    /// The test that is due, if one is (§4.5 "test pending"), and whether it
    /// can start. A probe needs a carrier not on its final attempt, a
    /// confirming test any member that needs a new connection.
    pub(crate) fn test_due(&mut self, now: u64) -> Option<TestPlan> {
        self.table.advance(now);
        if self.test_slot.is_some() || self.hold.in_force(now) {
            return None;
        }
        let (connected, quiet) = (self.table.connected(), self.table.is_quiet());
        if self.confirmation {
            return (self.needing_new > 0).then_some(TestPlan {
                kind: AttemptKind::Confirming,
                target: connected + 1,
                ready: quiet,
            });
        }
        let n = self.fsm.n();
        let due = match self.fsm.state() {
            State::Discovering => true,
            State::Stable => self.timer.expired(now),
            State::Probing | State::Saturated => false,
        };
        (due && self.non_final > 0).then_some(TestPlan {
            kind: AttemptKind::Probe,
            target: n + 1,
            ready: quiet && connected == n,
        })
    }
    /// Whether the client must close and evict nothing on the key: a due
    /// test has reached its base and waits for quiet.
    pub(crate) fn closes_suppressed(&mut self, now: u64) -> bool {
        let Some(plan) = self.test_due(now) else {
            return false;
        };
        let n = self.fsm.n();
        let base_failed = self.table.is_quiet() && self.table.connected() != n;
        !plan.ready
            && (plan.kind == AttemptKind::Confirming
                || (self.table.possible() >= n && !base_failed))
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
        self.test_slot
            .and_then(|attempt| self.targets.get(&attempt).copied())
            .unwrap_or_else(|| self.fsm.n())
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
    /// An attempt's result. `None` if the attempt is not in flight.
    pub(crate) fn result(
        &mut self,
        attempt: AttemptId,
        outcome: Outcome,
        now: u64,
    ) -> Option<Ruling> {
        self.table.advance(now);
        let closed = self.windows.close(attempt)?;
        let stored = self.targets.remove(&attempt)?;
        // An ordinary refusal is judged against the N in force at its result
        // (R3: a wave's later refusals have hi >= the lowered N).
        let target = if closed.kind == AttemptKind::Ordinary {
            self.fsm.n()
        } else {
            stored
        };
        if self.test_slot == Some(attempt) {
            self.test_slot = None;
        }
        let before = (self.fsm.n(), self.fsm.state());
        let ruling = match outcome {
            Outcome::Ended => self.ended(closed.kind, now),
            Outcome::Succeeded { fresh } => self.succeeded(&closed, target, fresh, now),
            Outcome::Refused {
                signal,
                retry_after_ms,
                post,
            } => {
                if let Some(wait) = retry_after_ms.and_then(|ms| {
                    let origin = if post {
                        Origin::Post
                    } else {
                        Origin::Discovery
                    };
                    self.hold.set(ms, origin, now)
                }) {
                    self.notes.hold(wait);
                }
                let refusal = Refusal {
                    kind: closed.kind,
                    signal,
                    target,
                    lo: closed.lo,
                    hi: closed.hi,
                    adaptive: self.adaptive,
                };
                self.refused(evidence(&refusal), &closed, now)
            }
        };
        let (n, state) = (self.fsm.n(), self.fsm.state());
        if n != before.0 {
            self.notes.n_changed(n);
        }
        if state == State::Saturated && before.1 != State::Saturated {
            self.timer.disarm();
            self.notes.saturated(self.ceiling);
        }
        self.flush_notes(now);
        Some(ruling)
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
    /// A test that said nothing is due again when the key is next quiet.
    fn rearm(&mut self, now: u64) {
        self.fsm.test_inconclusive();
        self.timer.make_due(now);
    }
    fn ended(&mut self, kind: AttemptKind, now: u64) -> Ruling {
        if kind.is_test() {
            self.rearm(now);
        }
        Ruling::Ended
    }
    fn succeeded(&mut self, closed: &Closed, target: usize, fresh: bool, now: u64) -> Ruling {
        let connected = self.table.connected();
        match closed.kind {
            AttemptKind::Probe if !fresh => {
                self.rearm(now);
                Ruling::Unfair
            }
            AttemptKind::Probe => {
                let fair = fair(target, closed.lo);
                self.fsm.test_succeeded(connected, fair);
                if fair {
                    self.timer.reset();
                    Ruling::Succeeded
                } else {
                    self.timer.make_due(now);
                    Ruling::Unfair
                }
            }
            AttemptKind::Confirming if !fresh => Ruling::Unfair,
            AttemptKind::Confirming => {
                // Refuted, fair or not (§4.5 rule 3).
                self.fsm.success(connected);
                self.confirmation = false;
                Ruling::Refuted
            }
            AttemptKind::Ordinary | AttemptKind::Restore => {
                self.fsm.success(connected);
                Ruling::Succeeded
            }
        }
    }
    fn refused(&mut self, evidence: Evidence, closed: &Closed, now: u64) -> Ruling {
        let confirming = closed.kind == AttemptKind::Confirming;
        match evidence {
            Evidence::RetryMachine => {
                self.confirmation &= !confirming;
                Ruling::RetryMachine
            }
            Evidence::Inconclusive => {
                // The barrier belongs to the adaptive machine: with N pinned
                // at the ceiling nothing is learned from the overlap, and a
                // bare refusal must not delay the next start.
                if self.adaptive {
                    self.barrier.extend(closed.winding_down.iter().copied());
                }
                Ruling::Inconclusive
            }
            Evidence::HoldOnly => Ruling::HoldOnly,
            Evidence::Overload => {
                self.fsm.overload(self.table.connected(), closed.hi);
                self.confirmation = false;
                self.timer.reset();
                self.timer.arm(now);
                self.notes.overload();
                Ruling::Overload { n: self.fsm.n() }
            }
            Evidence::Confirm if self.confirmation => Ruling::Inconclusive,
            Evidence::Confirm => {
                self.confirmation = true;
                Ruling::Confirmation
            }
            Evidence::UnfairTest => {
                self.rearm(now);
                Ruling::Unfair
            }
            Evidence::RefusedTest => {
                match self.fsm.probe_refused() {
                    Backoff::Double => self.timer.back_off(),
                    Backoff::Reset => self.timer.reset(),
                }
                self.timer.arm(now);
                Ruling::RefusedTest
            }
        }
    }
    fn flush_notes(&mut self, now: u64) {
        let resolved = self.table.setups_in_flight() == 0;
        self.notes.flush(now, self.fsm.n(), self.ceiling, resolved);
    }
    /// Whether `attempt` is an admitted test carrier: never deferred.
    pub(crate) fn is_test_carrier(&self, attempt: AttemptId) -> bool {
        self.windows.is_test_carrier(attempt)
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
            self.notes.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
    pub(crate) fn drain_notes(&mut self) -> Vec<Note> {
        self.notes.drain()
    }
}
