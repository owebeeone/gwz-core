use std::sync::Arc;



use super::*;

pub struct EventSubscription {
    pub(crate) record: Arc<OperationRecord>,
    pub(crate) next_sequence: i64,
}

impl EventSubscription {
    pub fn drain(&mut self) -> Vec<crate::OperationEvent> {
        let state = self.record.state.lock().expect("operation record poisoned");
        let events: Vec<_> = state
            .events
            .iter()
            .filter(|event| event.sequence >= self.next_sequence)
            .cloned()
            .collect();
        if let Some(last) = events.last() {
            self.next_sequence = last.sequence + 1;
        }
        events
    }
}

