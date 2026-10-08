//! Who is told when something changes, in the places that used to learn of it
//! on a timer. Each is owned by the thing it wakes, never shared process-wide.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Wake, Waker},
};
use tokio::{sync::Notify, time::Instant};

/// What wakes a pool's supervisor: the wakers the pool's resources register,
/// and `poke`, for a change made to the pool from outside it (a checkout, a
/// release, a shutdown). The supervisor sleeps until one of those or its
/// next deadline. A poke made before it sleeps is kept, so none is lost.
#[derive(Default)]
pub(crate) struct PoolWake {
    notify: Notify,
    steps: AtomicU64,
}
impl Wake for PoolWake {
    fn wake(self: Arc<Self>) {
        self.notify.notify_one();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.notify.notify_one();
    }
}
impl PoolWake {
    /// The waker the pool's resources are polled with.
    pub(crate) fn waker(self: &Arc<Self>) -> Waker {
        Waker::from(self.clone())
    }
    pub(crate) fn poke(&self) {
        self.notify.notify_one();
    }
    /// Counts one turn of the supervisor.
    pub(crate) fn stepped(&self) {
        self.steps.fetch_add(1, Ordering::Relaxed);
    }
    /// A guard that pokes when it is dropped: for a scope after which the pool
    /// has something to see, however the scope ends (including by being dropped
    /// at an await).
    pub(crate) fn poke_on_drop(self: &Arc<Self>) -> PokeOnDrop {
        PokeOnDrop(self.clone())
    }
    cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
        /// The turns the supervisor has taken.
        pub(crate) fn steps(&self) -> u64 {
            self.steps.load(Ordering::Relaxed)
        }
    } }
    /// Returns when poked, or at `deadline` if there is one.
    pub(crate) async fn parked(&self, deadline: Option<Instant>) {
        if let Some(at) = deadline {
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep_until(at) => {}
            }
        } else {
            self.notify.notified().await;
        }
    }
}

/// See `PoolWake::poke_on_drop`.
pub(crate) struct PokeOnDrop(Arc<PoolWake>);
impl Drop for PokeOnDrop {
    fn drop(&mut self) {
        self.0.poke();
    }
}

/// Told that a stream changed for its worker, which waits to complete the
/// close the initiator asked for. Whoever moves the stream's messages (the
/// host's `accept`, `step` and `take_outbound`, or a fixture's loop) calls
/// `notify` after each change.
#[derive(Clone, Default)]
pub(crate) struct CloseWake(Arc<Notify>);
impl CloseWake {
    pub(crate) fn notify(&self) {
        self.0.notify_one();
    }
    /// Returns after a `notify` since the last return, at once if there was one.
    pub(crate) async fn changed(&self) {
        self.0.notified().await;
    }
}
