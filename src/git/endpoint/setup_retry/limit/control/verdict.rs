//! An attempt's result and what it does to the machine (§4.2, §4.4, §4.5),
//! one part of a key's [`Limit`].
use super::{Limit, Outcome, Ruling};
use crate::git::endpoint::setup_retry::limit::{
    filter::{Evidence, Refusal, Signal, evidence, fair},
    fsm::{Backoff, State},
    hold::Origin,
    windows::{AttemptId, AttemptKind, Closed},
};

impl Limit {
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
        let target = if matches!(closed.kind, AttemptKind::Ordinary | AttemptKind::SetupProbe) {
            self.fsm.n()
        } else {
            stored
        };
        if self.test_slot == Some(attempt) {
            self.test_slot = None;
        }
        self.setup_test_over(attempt);
        let before = (self.fsm.n(), self.fsm.state());
        let ruling = match outcome {
            Outcome::Ended => self.ended(closed.kind, now),
            Outcome::Succeeded { fresh } => self.succeeded(&closed, target, stored, fresh, now),
            // A drop before authentication is the setup limit's evidence and
            // none for `N` (§4.10): for `N` the attempt ended with no verdict.
            Outcome::Refused {
                signal: Signal::SetupDrop,
                ..
            } => {
                if closed.kind != AttemptKind::SetupProbe {
                    self.ended(closed.kind, now);
                }
                self.setup_dropped(&closed, stored, now)
            }
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
                if closed.kind == AttemptKind::SetupProbe {
                    self.setup_test_rearm(now);
                }
                self.refused(evidence(&refusal), &closed, now)
            }
        };
        let refused = matches!(outcome, Outcome::Refused { .. });
        self.restore_progress(attempt, closed.kind, refused, now);
        self.settled(before, now);
        Some(ruling)
    }
    /// Notes what a result did to `N` and the state, and flushes the notes.
    pub(super) fn settled(&mut self, before: (usize, State), now: u64) {
        let (n, state) = (self.fsm.n(), self.fsm.state());
        if n != before.0 {
            self.notes.n_changed(n);
        }
        if state == State::Saturated && before.1 != State::Saturated {
            self.timer.disarm();
            self.notes.saturated(self.ceiling);
        }
        self.flush_notes(now);
    }
    fn ended(&mut self, kind: AttemptKind, now: u64) -> Ruling {
        if kind.is_n_test() {
            self.rearm(now);
        }
        if kind == AttemptKind::SetupProbe {
            self.setup_test_rearm(now);
        }
        Ruling::Ended
    }
    fn succeeded(
        &mut self,
        closed: &Closed,
        target: usize,
        stored: usize,
        fresh: bool,
        now: u64,
    ) -> Ruling {
        let ruling = self.succeeded_as(closed, target, fresh, now);
        if fresh {
            self.last_good = self.fsm.n();
            // A setup test is judged against `Ns + 1`, its stored target.
            let setup = self.setup_succeeded(closed, stored, now);
            return if closed.kind == AttemptKind::SetupProbe {
                setup
            } else {
                ruling
            };
        }
        if closed.kind == AttemptKind::SetupProbe {
            self.setup_test_rearm(now);
            return Ruling::Unfair;
        }
        ruling
    }
    fn succeeded_as(&mut self, closed: &Closed, target: usize, fresh: bool, now: u64) -> Ruling {
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
            AttemptKind::Ordinary | AttemptKind::Restore | AttemptKind::SetupProbe => {
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
}
