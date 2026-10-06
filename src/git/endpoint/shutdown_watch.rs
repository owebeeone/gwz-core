//! Whoever waits on a worker's shutdown, woken when the worker has ended.
use std::{
    sync::{Arc, Mutex},
    task::Waker,
};

/// The waker of a worker's watcher, which the worker wakes when its shutdown
/// has settled: the watcher parks between passes, and would otherwise learn
/// of the end only at its next timer tick.
#[derive(Clone, Default)]
pub(crate) struct Watch(Arc<Mutex<Option<Waker>>>);
impl Watch {
    /// Makes `waker` the one to wake. A watcher that registers on every pass
    /// with the waker it already has costs no clone.
    pub(crate) fn register(&self, waker: &Waker) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.as_ref().is_some_and(|held| held.will_wake(waker)) {
            *slot = Some(waker.clone());
        }
    }
    /// Wakes the watcher, if there is one.
    pub(crate) fn notify(&self) {
        let waker = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
