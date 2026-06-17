use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::Mutex;




/// Delivery seam for operation events: an implementation decides what to do
/// with each event (buffer it, stream it as JSONL, render progress, drop it).
/// Handlers stay producers; consumers plug in here without the thread runtime.
pub trait EventSink: Send + Sync {
    fn deliver(&self, event: crate::OperationEvent);
}

/// Discards every event. Default for callers that do not consume events.
pub struct NullSink;

impl EventSink for NullSink {
    fn deliver(&self, _event: crate::OperationEvent) {}
}

/// Builds protocol `OperationEvent`s (envelope + monotonic sequence) and
/// forwards them to a sink. A handler holds one per operation and emits as it
/// works; the sink decides how to consume them.
pub struct EventEmitter<'a> {
    pub(crate) operation_id: String,
    pub(crate) request_id: String,
    pub(crate) attribution: Option<crate::OperationAttribution>,
    pub(crate) sequence: AtomicI64,
    /// Minimum ms between member_progress events per member; 0 = no limit.
    pub(crate) progress_min_interval_ms: i64,
    pub(crate) last_progress_ms: Mutex<HashMap<String, i64>>,
    pub(crate) sink: &'a dyn EventSink,
}

