use std::time::{SystemTime, UNIX_EPOCH};

use crate::runtime::clock::TimestampMs;

pub(crate) fn now_ms() -> TimestampMs {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    TimestampMs(millis.min(i64::MAX as u128) as i64)
}
