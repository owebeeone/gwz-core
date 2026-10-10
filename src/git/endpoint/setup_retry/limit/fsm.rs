//! The operator's machine (§4.6, Appendix A): the believed limit `N` and the
//! state around it. It assigns `N` only from observations (§4.2).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Discovering,
    Stable,
    Probing,
    Saturated,
    /// After an outage that lowered `N`: judged doubling steps back up to
    /// `N_good` (§5.5). The probe timer is suspended.
    Restoring,
}

/// What a refused probe did to the probe timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backoff {
    /// A fair refusal in PROBING: `T := min(2T, Tmax)`.
    Double,
    /// A refusal in DISCOVERING: `T := T0`.
    Reset,
}

pub(crate) struct Fsm {
    ceiling: usize,
    n: usize,
    state: State,
}

impl Fsm {
    /// Starts SATURATED at `N = C`: the first wave is parallel (OD18).
    pub(crate) fn new(ceiling: usize) -> Self {
        Self {
            ceiling,
            n: ceiling,
            state: State::Saturated,
        }
    }
    pub(crate) fn n(&self) -> usize {
        self.n
    }
    pub(crate) fn state(&self) -> State {
        self.state
    }
    /// Any success: `N := min(C, max(N, Connected))`.
    pub(crate) fn success(&mut self, connected: usize) {
        self.assign(self.n.max(connected));
    }
    /// `N` reaching the ceiling by any route is SATURATED, and SATURATED is
    /// only that: STABLE and PROBING are never entered with `N = C`.
    fn assign(&mut self, n: usize) {
        self.n = n.clamp(1, self.ceiling);
        if self.n == self.ceiling {
            self.state = State::Saturated;
        }
    }
    /// The retry machine is Healthy again with `N < N_good` (§5.5).
    pub(crate) fn begin_restore(&mut self) {
        if self.state != State::Saturated {
            self.state = State::Restoring;
        }
    }
    /// The restore ended short of the ceiling: STABLE at the current `N`.
    pub(crate) fn end_restore(&mut self) {
        if self.state == State::Restoring {
            self.state = State::Stable;
        }
    }
    /// A probe or a DISCOVERING test has started.
    pub(crate) fn test_started(&mut self) {
        if self.state == State::Stable {
            self.state = State::Probing;
        }
    }
    /// A probe test succeeded. Fair: PROBING moves to DISCOVERING, or to
    /// SATURATED at the ceiling. Unfair: PROBING returns to STABLE.
    pub(crate) fn test_succeeded(&mut self, connected: usize, fair: bool) {
        self.success(connected);
        self.state = match self.state {
            State::Probing | State::Discovering if fair => State::Discovering,
            State::Probing => State::Stable,
            other => other,
        };
    }
    /// A fair refused probe: STABLE, `N` unchanged.
    pub(crate) fn probe_refused(&mut self) -> Backoff {
        let backoff = if self.state == State::Discovering {
            Backoff::Reset
        } else {
            Backoff::Double
        };
        // A test in flight when a restore began is judged, but the restore
        // goes on.
        if matches!(self.state, State::Probing | State::Discovering) {
            self.state = State::Stable;
        }
        backoff
    }
    /// A probe that said nothing (unfair, or ended): PROBING returns to
    /// STABLE.
    pub(crate) fn test_inconclusive(&mut self) {
        if self.state == State::Probing {
            self.state = State::Stable;
        }
    }
    /// An Overload: `N := max(1, min(Connected, hi))`, then STABLE.
    pub(crate) fn overload(&mut self, connected: usize, hi: usize) {
        self.state = State::Stable;
        self.assign(connected.min(hi));
    }
}
