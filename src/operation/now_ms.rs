use std::time::{SystemTime, UNIX_EPOCH};

use crate::runtime::clock::TimestampMs;

use super::*;

impl RuntimeEventSink {
    pub fn emit(
        &self,
        kind: crate::EventKind,
        severity: crate::Severity,
        member_id: Option<String>,
        member_path: Option<String>,
        message: Option<String>,
    ) {
        let mut state = self.record.state.lock().expect("operation record poisoned");
        push_event(&mut state, &self.context);
        let target_kind = member_id.as_ref().map(|_| crate::TargetKind::Member);
        let event = crate::OperationEvent {
            operation_id: self.context.operation_id.clone(),
            request_id: self.context.request_id.clone(),
            sequence: state.next_sequence,
            timestamp_ms: now_ms().0,
            kind,
            severity,
            member_id,
            member_path,
            message,
            member: None,
            error: None,
            attribution: self.context.attribution.as_ref().map(Into::into),
            target_kind,
            progress: None,
            merge_state: None,
            merge_member: None,
            artifact_path: None,
        };
        state.next_sequence += 1;
        state.events.push_back(event);
    }
}

pub(crate) fn now_ms() -> TimestampMs {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    TimestampMs(millis.min(i64::MAX as u128) as i64)
}
