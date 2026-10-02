//! An open's allocation clock while its endpoint holds it before an attempt.
//! The clock runs while the operation's limits hold the open, and it stops
//! while the open's key holds it, since a wait for the key is no allocation.

/// What is left of an open's allocation, and since when the clock runs, if it
/// runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AllocationClock {
    left_ms: u64,
    running_since: Option<u64>,
}

impl AllocationClock {
    /// A clock of `allocation_ms` that runs from `now`.
    pub(crate) fn new(now: u64, allocation_ms: u64) -> Self {
        Self {
            left_ms: allocation_ms,
            running_since: Some(now),
        }
    }
    /// What is left at `now`.
    pub(crate) fn left(&self, now: u64) -> u64 {
        match self.running_since {
            Some(since) => self.left_ms.saturating_sub(now.saturating_sub(since)),
            None => self.left_ms,
        }
    }
    /// The open's key holds it: the clock stops at `now`.
    pub(crate) fn stop(&mut self, now: u64) {
        self.left_ms = self.left(now);
        self.running_since = None;
    }
    /// The open may start, or the operation's limits hold it: a stopped
    /// clock runs again from `now`.
    pub(crate) fn run(&mut self, now: u64) {
        self.running_since.get_or_insert(now);
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::AllocationClock;

            #[test]
            fn a_wait_for_the_operations_limits_is_allocation_and_a_wait_for_the_key_is_not() {
                let mut clock = AllocationClock::new(0, 900);
                // The per-host limit holds the open for 800 ms: they are spent.
                clock.run(400);
                assert_eq!(clock.left(800), 100);
                // Its key then holds it until its wake: the clock stops.
                clock.stop(800);
                clock.stop(1_500);
                assert_eq!(clock.left(1_800), 100);
                // Past the wake it runs again from where it stopped.
                clock.run(1_800);
                clock.run(1_820);
                assert_eq!(clock.left(1_850), 50);
                assert_eq!(clock.left(1_900), 0);
                assert_eq!(clock.left(5_000), 0, "never below zero");
            }
        }
    }
}
