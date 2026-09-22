//! Operation retirement is explicit. A remote or stream dropping only releases
//! its dependency; routes survive until their owner seals the operation.
use super::https_policy::Routes;
use gwz_transport::protocol::ErrorCode;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

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
}
pub(crate) struct Dependency {
    state: Arc<Mutex<State>>,
    operation: String,
}
impl Operations {
    pub(crate) fn new(routes: Arc<Mutex<Routes>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries: BTreeMap::new(),
                routes,
            })),
        }
    }
    pub(crate) fn acquire(&self, operation: &str) -> Result<Dependency, ErrorCode> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.entries.contains_key(operation) && state.entries.len() >= 64 {
            return Err(ErrorCode::Capacity);
        }
        let entry = state.entries.entry(operation.to_owned()).or_default();
        if entry.sealed {
            return Err(ErrorCode::Cancelled);
        }
        entry.dependents += 1;
        Ok(Dependency {
            state: self.state.clone(),
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
    fn sealing_rejects_new_dependents_and_retires_only_after_last_release() {
        let routes = Arc::new(Mutex::new(Routes::new(64)));
        let operations = Operations::new(routes.clone());
        let a = operations.acquire("operation").unwrap();
        let b = operations.acquire("operation").unwrap();
        let key = RouteKey::new("operation", "https://original/repo", GitService::UploadPackAdvertisement);
        routes.lock().unwrap().admit(key.clone()).unwrap();
        routes.lock().unwrap().install(&key, "https://final/repo").unwrap();
        drop(a);
        assert_eq!(routes.lock().unwrap().get(&key).unwrap(), "https://final/repo");
        operations.finish("operation");
        assert!(matches!(operations.acquire("operation"), Err(ErrorCode::Cancelled)));
        assert_eq!(routes.lock().unwrap().get(&key).unwrap(), "https://final/repo");
        drop(b);
        assert!(routes.lock().unwrap().get(&key).is_err());
        assert!(operations.acquire("operation").is_ok());
    }
}
} }
