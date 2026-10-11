//! SSH's setup limit `Ns` (§4.10), one part of a key's [`Limit`]: a second
//! machine and timer over the setups in flight, fed only by drops before
//! authentication, never by `N`'s evidence. A drop whose window is conclusive
//! holds new setups until the co-setups it overlapped have results, then sets
//! `Ns` to the number that got through.
use super::{Limit, Ruling, TestPlan};
use crate::git::endpoint::setup_retry::limit::{
    filter::fair,
    fsm::{Backoff, Fsm, State},
    states::{Change, ConnId, Phase},
    timer::{Spread, Timer},
    windows::{AttemptId, AttemptKind, Closed},
};
use std::collections::BTreeSet;

/// The co-setups of a conclusive drop, awaited (§4.10).
struct Window {
    /// Clocked setups of `S_lo_s(a)` without a result. A setup with no clock
    /// (`--ssh-timeout 0`) is not waited for and counts as not succeeded.
    pending: BTreeSet<ConnId>,
    succeeded: usize,
}

/// `Ns`, its machine and its timer, and the setup test in flight.
pub(super) struct SetupLimit {
    fsm: Fsm,
    timer: Timer,
    window: Option<Window>,
    test: Option<AttemptId>,
}

impl SetupLimit {
    pub(super) fn new(ceiling: usize, spread: Spread) -> Self {
        Self {
            fsm: Fsm::new(ceiling),
            timer: Timer::new(spread),
            window: None,
            test: None,
        }
    }
    /// A test that said nothing is due again.
    fn rearm(&mut self, now: u64) {
        self.fsm.test_inconclusive();
        self.timer.make_due(now);
    }
    /// A window is open: new setups are held until it resolves.
    pub(super) fn holds(&self) -> bool {
        self.window.is_some()
    }
    /// The setup test in flight is `attempt`'s.
    pub(super) fn is_test(&self, attempt: AttemptId) -> bool {
        self.test == Some(attempt)
    }
    pub(super) fn n(&self) -> usize {
        self.fsm.n()
    }
    pub(super) fn state(&self) -> State {
        self.fsm.state()
    }
    /// The next probe-timer expiry, while the machine waits on it.
    pub(super) fn deadline(&self) -> Option<u64> {
        (self.fsm.state() == State::Stable && self.test.is_none())
            .then(|| self.timer.deadline())
            .flatten()
    }
}

impl Limit {
    /// The setup limit `Ns`.
    pub(crate) fn setup_limit(&self) -> usize {
        self.setups.n()
    }
    pub(crate) fn setup_state(&self) -> State {
        self.setups.state()
    }
    /// How many setups may be in flight: `Ns`, and one more for a setup test
    /// that is armed or running.
    ///
    /// At the ceiling (`Ns = C`, nothing found) it limits nothing: `N`'s rules
    /// and the pool's own limit hold the connections, as they did.
    pub(crate) fn setup_allowance(&self) -> usize {
        if self.setups.state() == State::Saturated {
            return usize::MAX;
        }
        let testing = self.setups.test.is_some()
            || self
                .armed
                .is_some_and(|(kind, ..)| kind == AttemptKind::SetupProbe);
        self.setups.n() + usize::from(testing)
    }
    /// Whether another setup may be in flight (§4.10: admission needs
    /// setups in flight `< Ns` as well as `Possible < N`).
    pub(super) fn setup_room(&self) -> bool {
        self.setups.state() == State::Saturated || self.table.setting_up() < self.setups.n()
    }
    /// The setup test, if one is due (§4.10): one more setup in flight than
    /// `Ns`, within the room `N` leaves. It needs `Ns` setups in flight
    /// already, and a carrier not on its final attempt.
    pub(super) fn setup_test(&mut self, now: u64) -> Option<TestPlan> {
        self.table.advance(now);
        if self.test_slot.is_some()
            || self.armed.is_some()
            || self.confirmation
            || self.setups.holds()
            || self.hold.in_force(now)
        {
            return None;
        }
        let due = match self.setups.fsm.state() {
            State::Discovering => true,
            State::Stable => self.setups.timer.expired(now),
            State::Probing | State::Saturated | State::Restoring => false,
        };
        let ns = self.setups.n();
        (due && self.non_final > 0).then(|| TestPlan {
            kind: AttemptKind::SetupProbe,
            target: ns + 1,
            ready: self.table.setting_up() == ns && self.n_room(),
        })
    }
    /// A setup test has begun: it holds the slot.
    pub(super) fn setup_test_began(&mut self, attempt: AttemptId) {
        self.setups.test = Some(attempt);
        self.setups.fsm.test_started();
    }
    /// A result for the setup test: it no longer holds the slot.
    pub(super) fn setup_test_over(&mut self, attempt: AttemptId) {
        if self.setups.test == Some(attempt) {
            self.setups.test = None;
        }
    }
    /// The setup test, armed for a carrier that ended with no connection, is
    /// due again.
    pub(super) fn setup_test_rearm(&mut self, now: u64) {
        self.setups.rearm(now);
    }
    /// A fresh success beside `lo_s` setups that stayed in progress: the
    /// server held at least that many (§4.10).
    pub(super) fn setup_succeeded(&mut self, closed: &Closed, target: usize, now: u64) -> Ruling {
        let others = closed.lo_s.len();
        if closed.kind != AttemptKind::SetupProbe {
            self.setups.fsm.success(others + 1);
            return Ruling::Succeeded;
        }
        let fair = fair(target, others);
        self.setups.fsm.test_succeeded(others + 1, fair);
        if fair {
            self.setups.timer.reset();
            Ruling::Succeeded
        } else {
            self.setups.timer.make_due(now);
            Ruling::Unfair
        }
    }
    /// A drop before authentication (§4.10). `target` is the test's `Ns + 1`.
    pub(super) fn setup_dropped(&mut self, closed: &Closed, target: usize, now: u64) -> Ruling {
        // Without a budget no test could ever check the decrease (§5.3).
        if !self.adaptive {
            return Ruling::RetryMachine;
        }
        if closed.kind == AttemptKind::SetupProbe {
            if !fair(target, closed.lo_s.len()) {
                self.setups.rearm(now);
                return Ruling::Unfair;
            }
            match self.setups.fsm.probe_refused() {
                Backoff::Double => self.setups.timer.back_off(),
                Backoff::Reset => self.setups.timer.reset(),
            }
            self.setups.timer.arm(now);
            return Ruling::RefusedTest;
        }
        // Nothing else in setup: not a concurrency limit (§4.8).
        if closed.hi_s == 0 {
            return Ruling::RetryMachine;
        }
        // Conclusive iff the server dropped it while holding fewer than `Ns`;
        // later drops of the wave are no input while its window is open.
        if closed.hi_s >= self.setups.n() || self.setups.holds() {
            return Ruling::Inconclusive;
        }
        let (mut pending, mut succeeded) = (BTreeSet::new(), 0);
        for conn in &closed.lo_s {
            match self.table.phase(*conn) {
                Some(Phase::Connected) => succeeded += 1,
                Some(Phase::SettingUp) if self.table.is_clocked_setup(*conn) => {
                    pending.insert(*conn);
                }
                _ => {}
            }
        }
        self.setups.window = Some(Window { pending, succeeded });
        self.setup_window_resolved(now);
        Ruling::SetupLimit
    }
    /// A setup left its state: the window counts it, and resolves when every
    /// clocked co-setup has a result.
    pub(super) fn setup_changed(&mut self, change: &Change, now: u64) {
        if let Some(window) = &mut self.setups.window
            && change.from == Some(Phase::SettingUp)
            && window.pending.remove(&change.conn)
            && change.to == Phase::Connected
        {
            window.succeeded += 1;
        }
        self.setup_window_resolved(now);
    }
    /// `Ns := max(1, the co-setups that succeeded)`, then the timer runs from
    /// `T0`.
    fn setup_window_resolved(&mut self, now: u64) {
        let Some(window) = self.setups.window.take_if(|w| w.pending.is_empty()) else {
            return;
        };
        self.setups.fsm.overload(window.succeeded, usize::MAX);
        self.setups.timer.reset();
        self.setups.timer.arm(now);
    }
}
