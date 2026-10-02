use super::*;

pub(super) struct Wait<T: Clone> {
    value: Mutex<Option<T>>,
    changed: Condvar,
}
impl<T: Clone> Wait<T> {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            changed: Condvar::new(),
        })
    }
    pub(super) fn complete(&self, value: T) {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_none() {
            *state = Some(value);
            self.changed.notify_all();
        }
    }
    pub(super) fn complete_with(&self, value: T, before: impl FnOnce()) -> bool {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_some() {
            return false;
        }
        before();
        *state = Some(value);
        self.changed.notify_all();
        true
    }
    pub(super) fn get(&self) -> T {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(value) = &*state {
                return value.clone();
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}
