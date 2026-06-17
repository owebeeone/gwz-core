use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};



use super::*;

impl OperationRecord {
    pub(crate) fn new(event_capacity: usize) -> Self {
        Self {
            state: Mutex::new(OperationState {
                events: VecDeque::with_capacity(event_capacity),
                event_capacity,
                next_sequence: 0,
                result: None,
            }),
            complete: Condvar::new(),
        }
    }

    pub(crate) fn complete(&self, result: crate::OperationResult) {
        let mut state = self.state.lock().expect("operation record poisoned");
        state.result = Some(result);
        self.complete.notify_all();
    }
}

