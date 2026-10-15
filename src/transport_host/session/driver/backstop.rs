//! The driver's per-open backstop: the safety net that ends one open whose
//! endpoint never answers.
//!
//! **What it bounds.** The backstop covers the attempts of an open (the retry
//! plan's §5 full bound, `open_backstop_ms`). It does not cover a hold: while
//! the endpoint holds the open (waiting for room on its site or its host,
//! behind a hold of its site, or in its key's retry wait), the open's
//! allocation clock is stopped (adaptive concurrency §5.2), and the user-set
//! time such an open may wait is the server's and the limit machine's, not a
//! fault of the endpoint. The backstop therefore stops charging while the
//! endpoint reports the open held and resumes with what is left when an
//! attempt starts, so a long hold never spends the budget of the attempts.
//!
//! **What it ends.** Only its own stream: the pump completes the open's reply,
//! sends the open's own `Cancel` to the endpoint, cancels the stream and
//! disconnects its peer. A request's other streams, mid-transfer or held,
//! are untouched, and the request is not sealed (§5.3: a member's failure is
//! its own).
//!
//! **Where it cannot see.** A Cli-placed request's endpoint is beyond the
//! session port, which reports no holds; its opens are charged for the whole
//! wait, and the backstop still ends only the stream it was armed for.

/// One open's budget of attempt time.
pub(in crate::transport_host::session) struct Backstop {
    /// What is left of the budget, in milliseconds.
    left_ms: u64,
    /// The clock at the last look.
    seen_ms: u64,
    /// Whether the endpoint held the open at the last look.
    held: bool,
}

impl Backstop {
    /// A backstop of `bound_ms` armed at `now_ms`, for an open that is not
    /// yet held.
    pub(in crate::transport_host::session) fn new(bound_ms: u64, now_ms: u64) -> Self {
        Self {
            left_ms: bound_ms,
            seen_ms: now_ms,
            held: false,
        }
    }

    /// Looks at the open at `now_ms`, `held` saying whether the endpoint holds
    /// it now. The time since the last look is charged only when the open was
    /// not held at either look: a look that finds the open held, or that last
    /// found it held, is a hold starting or ending somewhere in the interval,
    /// and a hold is never charged. (The passes look every few milliseconds,
    /// so what this leaves uncharged is those few milliseconds.) Returns
    /// whether the budget is spent and the open is not held, which is when
    /// the backstop ends the open.
    pub(in crate::transport_host::session) fn look(&mut self, now_ms: u64, held: bool) -> bool {
        let span = now_ms.saturating_sub(self.seen_ms);
        if !self.held && !held {
            self.left_ms = self.left_ms.saturating_sub(span);
        }
        self.seen_ms = self.seen_ms.max(now_ms);
        self.held = held;
        !held && self.left_ms == 0
    }
}

cfg_if::cfg_if! { if #[cfg(test)] {
mod tests {
    use super::*;

    #[test]
    fn an_open_that_is_never_held_runs_out_at_its_bound() {
        let mut backstop = Backstop::new(1_000, 10);
        assert!(!backstop.look(500, false));
        assert!(!backstop.look(1_009, false));
        assert!(backstop.look(1_010, false));
    }

    #[test]
    fn a_hold_spends_none_of_the_budget_and_the_attempt_gets_what_was_left() {
        let mut backstop = Backstop::new(1_000, 0);
        // 300 ms of attempt, then held for a very long time.
        assert!(!backstop.look(300, false));
        assert!(!backstop.look(310, true));
        assert!(!backstop.look(10_000_000, true));
        // The attempt starts: the rest of the budget runs again.
        assert!(!backstop.look(10_000_100, false));
        assert!(!backstop.look(10_000_600, false));
        assert!(backstop.look(10_000_800, false));
    }

    #[test]
    fn an_interval_that_ends_or_begins_a_hold_is_not_charged() {
        // One look after a long interval finds the open held: the interval
        // may have been all hold, so it spends nothing, and neither does the
        // look that finds the hold over.
        let mut backstop = Backstop::new(100, 0);
        assert!(!backstop.look(1_000_000, true));
        assert!(!backstop.look(1_000_001, false));
        assert!(!backstop.look(1_000_099, false));
        assert!(backstop.look(1_000_101, false));
    }

    #[test]
    fn a_clock_that_does_not_advance_charges_nothing() {
        let mut backstop = Backstop::new(100, 50);
        assert!(!backstop.look(40, false));
        assert!(!backstop.look(149, false));
        assert!(backstop.look(150, false));
    }
}
} }
