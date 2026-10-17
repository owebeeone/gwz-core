//! A rendezvous for tests that ask whether several pieces of synchronous work run at once (decision D33, run
//! `2026-10-17-https611`): the work of a connection's setup is a handshake's CPU inside one poll, so a runtime or a
//! lock that lets only one poll run at a time serializes the setups of N members.
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

/// Blocks each caller until `members` callers are inside, or `patience` has passed: with enough threads every caller
/// returns as the last one enters; with too few, or with a lock that holds the others out, the first ones are never
/// released. Nothing is measured: `patience` is only how long a failure takes.
pub(crate) struct Gate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
    members: usize,
}
impl Gate {
    pub(crate) fn new(members: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((0, false)),
            changed: Condvar::new(),
            members,
        })
    }
    /// True if all the members were inside at once.
    pub(crate) fn pass(&self, patience: Duration) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 += 1;
        self.changed.notify_all();
        let deadline = Instant::now() + patience;
        while state.0 < self.members && !state.1 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                // Give up for everyone: the ones after it do not wait their turn.
                state.1 = true;
                self.changed.notify_all();
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        state.0 >= self.members
    }
}
