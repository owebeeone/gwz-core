//! The key's one test slot: when a test is due, who carries it, and when the
//! client must close nothing (§4.5, §4.7), one part of a key's [`Limit`].
use super::{Limit, TestPlan};
use crate::git::endpoint::setup_retry::limit::{fsm::State, windows::AttemptKind};

impl Limit {
    /// The test that is due, if one is (§4.5 "test pending"), and whether it
    /// can start. A probe needs a carrier not on its final attempt, a
    /// confirming test any member that needs a new connection.
    pub(crate) fn test_due(&mut self, now: u64) -> Option<TestPlan> {
        self.table.advance(now);
        if self.test_slot.is_some() || self.armed.is_some() || self.hold.in_force(now) {
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
            // The probe timer is suspended during a restore (§4.6).
            State::Probing | State::Saturated | State::Restoring => false,
        };
        (due && self.non_final > 0).then_some(TestPlan {
            kind: AttemptKind::Probe,
            target: n + 1,
            ready: quiet && connected == n,
        })
    }
    /// Starts the due test for a carrier, when it is ready: the next
    /// connection on the site is the test. A probe is never carried by a
    /// member on its final attempt (`carrier_final`), a confirming test is
    /// (§4.7). Returns the admission target, which the pool's limit follows.
    pub(crate) fn arm_test(&mut self, carrier_final: bool, now: u64) -> Option<(usize, u64)> {
        // `N`'s test first. The setup test is startable only with `Ns` setups
        // in flight, which a probe's base (a quiet key) rules out, so the two
        // are never startable together and neither can starve the other.
        let plan = match self.test_due(now) {
            Some(plan) if plan.ready && !(carrier_final && plan.kind == AttemptKind::Probe) => plan,
            _ => self
                .setup_test(now)
                .filter(|plan| plan.ready && !carrier_final)?,
        };
        self.arms += 1;
        self.armed = Some((plan.kind, plan.target, self.arms));
        Some((self.pool_limit(), self.arms))
    }
    /// The connection the host has begun is the armed test.
    pub(crate) fn take_armed(&mut self) -> Option<(AttemptKind, usize)> {
        self.armed.take().map(|(kind, target, _)| (kind, target))
    }
    /// The carrier of arming `arm` ended with no connection of its own (it
    /// leased an idle one, failed or was cancelled): a probe is due again as
    /// soon as the key is quiet. Does nothing if that test was taken, or
    /// another has been armed since.
    pub(crate) fn disarm(&mut self, arm: u64) {
        if let Some((kind, _, armed)) = self.armed
            && armed == arm
        {
            self.armed = None;
            if kind == AttemptKind::SetupProbe {
                self.setup_test_rearm(0);
            }
            if kind == AttemptKind::Probe {
                // Expired already, whatever the time.
                self.rearm(0);
            }
        }
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
    /// A test that said nothing is due again when the key is next quiet.
    pub(super) fn rearm(&mut self, now: u64) {
        self.fsm.test_inconclusive();
        self.timer.make_due(now);
    }
}
