//! Operation retirement is explicit. A remote or stream dropping only releases
//! its dependency; routes survive until their owner seals the operation.
//!
//! Why they must (F7, adaptive concurrency design §7.2): the endpoint never
//! learns which remote a stream belongs to, since no remote-instance field
//! crosses the wire. A discovery's stream ends when its advertisement has been
//! read, before the exchange that resolves the route it pinned has opened, and
//! two remotes of one operation that name the same URL must meet the one
//! write-once route (HTTPS design §5). A count of live dependents per route
//! would therefore release a route between its discovery and its exchange, so
//! the route is released with its whole operation and no limit is placed on
//! how many an operation holds.
use super::https_policy::Routes;
use gwz_transport::protocol::ErrorCode;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

/// Why an operation cannot take a dependent now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The table of operations is full of operations that still have
    /// dependents. Not a failure: the caller waits until one retires, as it
    /// waits for a stream (`Operations::freed`).
    WouldBlock,
    /// The operation is sealed, so it takes no new work.
    Sealed,
}
impl Refusal {
    /// The failure a caller that cannot wait reports. A full table is never a
    /// `Capacity` failure.
    pub(crate) fn code(self) -> ErrorCode {
        ErrorCode::Cancelled
    }
}

#[derive(Default)]
struct Entry {
    dependents: usize,
    sealed: bool,
}
struct State {
    entries: BTreeMap<String, Entry>,
    routes: Arc<Mutex<Routes>>,
}
#[derive(Clone)]
pub(crate) struct Operations {
    state: Arc<Mutex<State>>,
    /// Woken when an operation retires and frees a place in the table.
    freed: Arc<Notify>,
}
pub(crate) struct Dependency {
    state: Arc<Mutex<State>>,
    freed: Arc<Notify>,
    operation: String,
}
impl Operations {
    pub(crate) fn new(routes: Arc<Mutex<Routes>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: BTreeMap::new(),
                routes,
            })),
            freed: Arc::new(Notify::new()),
        }
    }
    /// The operations' table changes when one retires; a caller that was
    /// refused with `WouldBlock` waits on this.
    pub(crate) fn freed(&self) -> &Notify {
        &self.freed
    }
    pub(crate) fn acquire(&self, operation: &str) -> Result<Dependency, Refusal> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.entries.contains_key(operation) && state.entries.len() >= 64 {
            return Err(Refusal::WouldBlock);
        }
        let entry = state.entries.entry(operation.to_owned()).or_default();
        if entry.sealed {
            return Err(Refusal::Sealed);
        }
        entry.dependents += 1;
        Ok(Dependency {
            state: self.state.clone(),
            freed: self.freed.clone(),
            operation: operation.into(),
        })
    }
    pub(crate) fn finish(&self, operation: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = state.entries.get_mut(operation) {
            entry.sealed = true;
            if entry.dependents != 0 {
                return;
            }
        }
        retire(&mut state, operation);
        self.freed.notify_waiters();
    }
}
fn retire(state: &mut State, operation: &str) {
    state
        .routes
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .finish(operation);
    state.entries.remove(operation);
}
impl Drop for Dependency {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = state.entries.get_mut(&self.operation) {
            entry.dependents -= 1;
            if entry.dependents == 0 && entry.sealed {
                retire(&mut state, &self.operation);
                self.freed.notify_waiters();
            }
        }
    }
}
cfg_if::cfg_if! { if #[cfg(test)] {
mod tests {
    use super::*;
    use super::super::https_policy::RouteKey;
    use gwz_transport::protocol::GitService;
    #[test]
    fn an_operation_past_the_cap_waits_and_is_never_refused_with_capacity() {
        let operations = Operations::new(Arc::new(Mutex::new(Routes::new())));
        let held: Vec<_> = (0..64)
            .map(|n| operations.acquire(&format!("operation-{n}")).unwrap())
            .collect();
        assert_eq!(
            operations.acquire("operation-64").err(),
            Some(Refusal::WouldBlock),
            "a full table must make its caller wait, not fail it"
        );
        operations.finish("operation-0");
        drop(held);
        assert!(operations.acquire("operation-64").is_ok());
    }
    #[tokio::test]
    async fn a_waiter_on_a_full_table_is_woken_when_an_operation_retires() {
        let operations = Operations::new(Arc::new(Mutex::new(Routes::new())));
        let held: Vec<_> = (0..64)
            .map(|n| operations.acquire(&format!("operation-{n}")).unwrap())
            .collect();
        let waiter = tokio::spawn({
            let operations = operations.clone();
            async move {
                loop {
                    let freed = operations.freed().notified();
                    tokio::pin!(freed);
                    freed.as_mut().enable();
                    if operations.acquire("late").is_ok() {
                        return true;
                    }
                    freed.await;
                }
            }
        });
        tokio::task::yield_now().await;
        operations.finish("operation-3");
        drop(held);
        assert!(waiter.await.unwrap());
    }
    #[test]
    fn sealing_rejects_new_dependents_and_retires_only_after_last_release() {
        let routes = Arc::new(Mutex::new(Routes::new()));
        let operations = Operations::new(routes.clone());
        let a = operations.acquire("operation").unwrap();
        let b = operations.acquire("operation").unwrap();
        let key = RouteKey::new("operation", "https://original/repo", GitService::UploadPackAdvertisement);
        routes.lock().unwrap().admit(key.clone());
        routes.lock().unwrap().install(&key, "https://final/repo").unwrap();
        drop(a);
        assert_eq!(routes.lock().unwrap().get(&key).unwrap(), "https://final/repo");
        operations.finish("operation");
        assert!(matches!(operations.acquire("operation"), Err(Refusal::Sealed)));
        assert_eq!(routes.lock().unwrap().get(&key).unwrap(), "https://final/repo");
        drop(b);
        assert!(routes.lock().unwrap().get(&key).is_err());
        assert!(operations.acquire("operation").is_ok());
    }
}
} }
