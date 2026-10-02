//! One pool key's retry state for one operation: the retry plan's §5, with
//! amendment 2's §3.20 (OD18) cold start.
//!
//! The machine opens nothing itself. Its endpoint asks it, for each member
//! that wants a connection, whether that member may start an attempt now,
//! and tells it how each attempt ended. A member that may not start waits in
//! the endpoint's own queue, so a wake with no member left opens no probe.
use super::{Verdict, backoff::wait_ms};
use gwz_transport::protocol::Failure;

/// The failure that finished a member, as attempt `attempt` of `attempts`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Final {
    pub(crate) failure: Failure,
    pub(crate) attempt: u32,
    pub(crate) attempts: u32,
}

impl Final {
    /// Project the endpoint's known count into the wire failure without
    /// changing the retry machine or inferring a count at the driver.
    pub(crate) fn wire_failure(self) -> Failure {
        let mut failure = self.failure;
        if self.attempt > 1 || super::classify(&failure, super::Phase::Setup) == Verdict::Retry {
            let detail = failure.detail.get_or_insert_with(Box::default);
            detail.retry_attempt = Some(gwz_transport::protocol::RetryAttempt {
                attempt: i64::from(self.attempt),
                attempts: i64::from(self.attempts),
            });
        }
        failure
    }
}

/// Whether a member that wants a connection may start an attempt now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Decision {
    /// Start one, and record it with [`Machine::start`].
    Start,
    /// Hold the member: the key opens no setup until its wake, or until its
    /// one probe ends.
    Wait,
    /// The key is closed for the operation: finish the member with this.
    Finish(Final),
}

/// What a failed attempt does to its member.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// The member returns to the key's queue.
    Retry,
    /// The member is finished with this failure.
    Finish(Final),
    /// The member is finished with its own failure, and the key did not move.
    Return,
}

/// A key's state. Cold: no setup has succeeded or been counted yet, and
/// the first wave starts in parallel, as far as the endpoint's per-host limit
/// lets it. Healthy: normal allocation. Waiting: a counted retriable failure
/// left attempts, and nothing opens until `until`. Degraded: one probe at a
/// time, `probe` while it runs. Closed: the operation's last word on the key.
enum State {
    Cold,
    Healthy,
    Waiting { until: u64 },
    Degraded { probe: bool },
    Closed(Final),
}

/// A started attempt: its member, the generation it started in, and the
/// attempt it counts as if it fails retriably.
struct Flight<M> {
    member: M,
    generation: u64,
    attempt: u32,
}

/// One pool key's retry machine within one operation.
pub(crate) struct Machine<M> {
    max_retries: u32,
    state: State,
    /// Moves on whenever the key leaves Cold or Healthy, and when it closes:
    /// an attempt that started before then is no longer the key's.
    generation: u64,
    /// Retriable failures counted since the key last set up a session.
    attempts: u32,
    flights: Vec<Flight<M>>,
}

impl<M: PartialEq> Machine<M> {
    pub(crate) fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            state: State::Cold,
            generation: 0,
            attempts: 0,
            flights: Vec::new(),
        }
    }

    /// Whether a member that wants a connection at `now` may start an
    /// attempt. Past its wake, a waiting key admits one probe.
    pub(crate) fn decide(&mut self, now: u64) -> Decision {
        if let State::Waiting { until } = self.state
            && now >= until
        {
            self.state = State::Degraded { probe: false };
        }
        match &self.state {
            State::Cold | State::Healthy | State::Degraded { probe: false } => Decision::Start,
            State::Waiting { .. } | State::Degraded { probe: true } => Decision::Wait,
            State::Closed(last) => Decision::Finish(last.clone()),
        }
    }

    /// Records the attempt `member` starts, which `decide` allowed.
    pub(crate) fn start(&mut self, member: M) {
        if let State::Degraded { probe } = &mut self.state {
            *probe = true;
        }
        self.flights.push(Flight {
            member,
            generation: self.generation,
            attempt: self.attempts.saturating_add(1),
        });
    }

    /// `member`'s attempt has a session. `fresh` is false when it leased an
    /// idle connection, which is no setup and moves nothing. Returns whether
    /// the session is admitted for reuse: a setup from a generation the key
    /// has left completes its member, but neither heals the key nor is
    /// admitted (the retry plan's §4).
    pub(crate) fn succeeded(&mut self, member: &M, fresh: bool) -> bool {
        let Some(flight) = self.take(member) else {
            return true;
        };
        let current = flight.generation == self.generation;
        match self.state {
            State::Cold | State::Healthy if current && fresh => {
                self.state = State::Healthy;
                self.attempts = 0;
            }
            State::Degraded { probe: true } if current => {
                self.state = if fresh {
                    self.attempts = 0;
                    State::Healthy
                } else {
                    State::Degraded { probe: false }
                };
            }
            _ => {}
        }
        current || !fresh
    }

    /// `member`'s attempt failed with `verdict`. A counted retriable failure
    /// that leaves attempts starts a wait of [`wait_ms`] plus `jitter` from
    /// `now`.
    pub(crate) fn failed(
        &mut self,
        member: &M,
        verdict: Verdict,
        failure: Failure,
        now: u64,
        jitter: u64,
    ) -> Outcome {
        let flight = self.take(member);
        if verdict == Verdict::Return {
            if let Some(flight) = &flight {
                self.release(flight);
            }
            return Outcome::Return;
        }
        let attempt = flight
            .as_ref()
            .map_or(self.attempts.saturating_add(1), |flight| flight.attempt);
        let current = flight
            .as_ref()
            .is_some_and(|flight| flight.generation == self.generation);
        if let State::Closed(last) = &self.state {
            return match verdict {
                // It returns to the key's queue, where the key finishes it.
                Verdict::Retry => Outcome::Finish(last.clone()),
                _ => Outcome::Finish(self.last(failure, attempt)),
            };
        }
        match verdict {
            // From a generation the key has left: no count, and the member
            // waits with the others.
            Verdict::Retry if !current => Outcome::Retry,
            Verdict::Retry => {
                if matches!(self.state, State::Cold | State::Healthy) {
                    self.generation += 1;
                }
                self.attempts = attempt;
                if attempt <= self.max_retries {
                    let until = now.saturating_add(wait_ms(attempt)).saturating_add(jitter);
                    self.state = State::Waiting { until };
                    Outcome::Retry
                } else {
                    Outcome::Finish(self.close(failure, attempt))
                }
            }
            _ => Outcome::Finish(self.close(failure, attempt)),
        }
    }

    /// `member`'s attempt ended without a verdict: it was cancelled, or ran
    /// out of its own time, before its setup had an outcome.
    pub(crate) fn abandoned(&mut self, member: &M) {
        if let Some(flight) = self.take(member) {
            self.release(&flight);
        }
    }

    fn take(&mut self, member: &M) -> Option<Flight<M>> {
        let index = self.flights.iter().position(|f| f.member == *member)?;
        Some(self.flights.swap_remove(index))
    }
    /// A probe that ended without a verdict frees the key for the next.
    fn release(&mut self, flight: &Flight<M>) {
        if flight.generation == self.generation
            && let State::Degraded { probe } = &mut self.state
        {
            *probe = false;
        }
    }
    fn last(&self, failure: Failure, attempt: u32) -> Final {
        Final {
            failure,
            attempt,
            attempts: self.max_retries.saturating_add(1),
        }
    }
    /// Closes the key for the rest of the operation with `failure`.
    fn close(&mut self, failure: Failure, attempt: u32) -> Final {
        let last = self.last(failure, attempt);
        self.generation += 1;
        self.state = State::Closed(last.clone());
        last
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl<M> Machine<M> {
            /// When a waiting key admits its next probe.
            pub(crate) fn wake_at(&self) -> Option<u64> {
                match self.state {
                    State::Waiting { until } => Some(until),
                    _ => None,
                }
            }
        }
    }
}
