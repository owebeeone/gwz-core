//! RESTORING's judged doubling steps (§5.5), one part of a key's [`Limit`].
use super::{Limit, State};
use crate::git::endpoint::setup_retry::limit::windows::{AttemptId, AttemptKind};
use std::collections::BTreeSet;

/// RESTORING's step: a judged doubling toward `N_good` (§5.5).
pub(super) struct Restore {
    pub(super) good: usize,
    /// The step's target `S`: starts are admitted up to it.
    pub(super) target: usize,
    /// The step's starts that have no result yet.
    pub(super) step: BTreeSet<AttemptId>,
    pub(super) started: bool,
    pub(super) refused: bool,
}

impl Limit {
    /// The retry machine left Cold or Healthy (§5.5): `N_good` is the `N` in
    /// force at the last success, kept for the length of the outage.
    pub(crate) fn outage(&mut self) {
        self.outage.get_or_insert(self.last_good);
    }
    /// The retry machine is Healthy again: when the outage left `N` below
    /// `N_good`, RESTORING begins (§5.5). Nothing is restored without a
    /// budget to test with.
    pub(crate) fn recovered(&mut self, now: u64) {
        let Some(good) = self.outage.take() else {
            return;
        };
        if !self.adaptive || self.fsm.n() >= good || self.restore.is_some() {
            return;
        }
        self.table.advance(now);
        // The probe timer needs no stopping: no probe is due while RESTORING,
        // and its exit arms the timer afresh.
        self.fsm.begin_restore();
        self.restore = Some(Restore {
            good,
            target: self.step_target(good),
            step: BTreeSet::new(),
            started: false,
            refused: false,
        });
    }
    /// `S := max(N, min(N_good, 2 x max(1, Connected)))`.
    fn step_target(&self, good: usize) -> usize {
        let doubled = 2 * self.table.connected().max(1);
        self.fsm.n().max(good.min(doubled))
    }
    /// Whether the key's retry machine has left Cold or Healthy and not come
    /// back (§5.5): `N_good` is frozen.
    pub(crate) fn in_outage(&self) -> bool {
        self.outage.is_some()
    }
    /// Whether a restore is under way: the endpoints let only members not on
    /// their final attempt start new connections then.
    pub(crate) fn restoring(&self) -> bool {
        self.restore.is_some()
    }
    /// What a new connection that is not a test starts as: a restore step's
    /// start while RESTORING (§4.4), else an ordinary start at the pool's
    /// limit.
    pub(crate) fn start_kind(&self) -> (AttemptKind, usize) {
        match &self.restore {
            Some(restore) => (AttemptKind::Restore, restore.target),
            None => (AttemptKind::Ordinary, self.pool_limit()),
        }
    }
    /// The restore is over: STABLE at the current `N`, `T := T0`.
    pub(super) fn end_restore(&mut self, now: u64) {
        self.restore = None;
        self.fsm.end_restore();
        self.timer.reset();
        self.timer.arm(now);
    }
    /// A step's result: when every start of the step has one, a refusal ends
    /// the restore, reaching `N_good` ends it, and otherwise the next step
    /// doubles. An Overload or the ceiling ended it already.
    pub(super) fn restore_progress(
        &mut self,
        attempt: AttemptId,
        kind: AttemptKind,
        refused: bool,
        now: u64,
    ) {
        if self.fsm.state() != State::Restoring {
            self.restore = None;
            return;
        }
        let Some(restore) = &mut self.restore else {
            return;
        };
        if kind == AttemptKind::Restore {
            restore.step.remove(&attempt);
            restore.refused |= refused;
        }
        if !restore.started || !restore.step.is_empty() {
            return;
        }
        if restore.refused || self.fsm.n() >= restore.good {
            self.end_restore(now);
            return;
        }
        let good = restore.good;
        let target = self.step_target(good);
        if let Some(restore) = &mut self.restore {
            restore.target = target;
            restore.started = false;
        }
    }
}
