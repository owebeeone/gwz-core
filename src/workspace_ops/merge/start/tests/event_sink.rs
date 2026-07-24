use crate::operation::EventSink;

use super::*;

impl EventSink for TraceSink<'_> {
    fn deliver(&self, event: crate::OperationEvent) {
        self.0.events.lock().unwrap().push(event.clone());
        self.0
            .trace
            .lock()
            .unwrap()
            .push(format!("event:{:?}", event.kind));
    }
}
