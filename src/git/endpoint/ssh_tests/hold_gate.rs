//! A hold on one sign reply of a fixture agent, which the row releases (finding F19). The timed `delay_next` orders a
//! row by the clock, and a clock is not an order: when the host is slow the delay ends before the event it was meant to
//! outlast. A hold ends when the row says so, after the event it must outlast has happened. Both agent twins (the Unix
//! proxy of `reuse_agent`, the pipe agent of `agent_fixture_pipe`) use this one gate.
use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Default)]
struct Hold {
    /// The next sign reply is held.
    armed: bool,
    /// Held replies are let go.
    open: bool,
    /// How many replies were held, counting the one now waiting.
    reached: usize,
}

#[derive(Default)]
pub(crate) struct HoldGate {
    state: Mutex<Hold>,
    changed: Condvar,
}

impl HoldGate {
    fn lock(&self) -> MutexGuard<'_, Hold> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The next sign reply waits for `release`.
    pub(crate) fn arm(&self) {
        let mut state = self.lock();
        state.armed = true;
        state.open = false;
    }

    /// The agent has a sign reply ready: it waits here for `release` when the gate is armed, and goes on at once
    /// otherwise.
    pub(crate) fn hold_if_armed(&self) {
        let mut state = self.lock();
        if !std::mem::take(&mut state.armed) {
            return;
        }
        state.reached += 1;
        self.changed.notify_all();
        while !state.open {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// How many replies have reached the gate and waited: one once the armed request's reply is ready and held.
    pub(crate) fn reached(&self) -> usize {
        self.lock().reached
    }

    /// Lets the held reply go, and any later one: the agent is done holding.
    pub(crate) fn release(&self) {
        self.lock().open = true;
        self.changed.notify_all();
    }
}
