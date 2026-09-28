//! The session limits of the core session contract's §1, and `open`'s
//! validation of them (session plan CS1.4).
//!
//! Each limit is set at `open` (§9) unless the contract marks it fixed; the
//! fixed ones are the two constants. Later steps enforce them: the running
//! limit, the queue and the operation table in admission (§5.1, §5.4), the
//! direct workers in §5.2, the event log and the open logs in §5.4, the
//! outstanding calls and the control reserve in the channel (§3), the read
//! size in §4.2 and the close wait in §8.

use std::time::Duration;

use crate::model::{ErrorCode, ModelError, ModelResult};

/// The longest wait a held log read may ask for, fixed at 30 seconds (§1, §4.2).
pub const MAX_READ_WAIT: Duration = Duration::from_secs(30);

/// The largest frame, fixed at 64 MiB (§1, §3).
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// The longest `close_wait` `open` accepts.
const MAX_CLOSE_WAIT: Duration = Duration::from_secs(60 * 60);

/// The limits of one session, set at `open` (§1, §9).
///
/// `Default` gives the contract's defaults. The type is `non_exhaustive`: start
/// from `Limits::default()` and change the fields a driver needs.
///
/// Validation bounds each count from below and against the others, not from
/// above, so nothing pre-allocates by a limit: a queue or table grows to its
/// limit, never starts at it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Limits {
    /// Operations running at once in the session (§5.1). Default 8.
    pub running_operations: usize,
    /// Operations waiting in the session's one FIFO queue (§5.1). Default 64.
    pub queued_operations: usize,
    /// Operation-table entries: live operations and terminal submitted
    /// records (§5.4). Default 128.
    pub operation_table: usize,
    /// Direct-method workers, apart from the running limit (§5.2). Default 8.
    pub direct_workers: usize,
    /// Events kept in one operation's event log (§5.4). Default 4096.
    pub event_log: usize,
    /// `diff.output` and `log.output` logs open at once (§5.4). Default 64.
    pub open_logs: usize,
    /// Ordinary calls outstanding at once (§3, §5.1). Default 1024.
    pub outstanding_calls: usize,
    /// Frames each channel queue holds for control calls, beyond the
    /// outstanding calls (§3). Default 64.
    pub control_reserve: usize,
    /// Bytes one log read returns at most (§4.2). Default 1 MiB, and at most
    /// half the frame size, 32 MiB. The read path (CS2.10) counts each
    /// record's encoded size against it, so a reply is at most `read_bytes`
    /// plus its envelope, which the frame's other half bounds (§3).
    pub read_bytes: usize,
    /// How long close waits for workers before it detaches them (§8).
    /// Default 60 seconds, and at most one hour, so a close deadline of now
    /// plus `close_wait` cannot overflow. Zero means close detaches every
    /// running worker at once.
    pub close_wait: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            running_operations: 8,
            queued_operations: 64,
            operation_table: 128,
            direct_workers: 8,
            event_log: 4096,
            open_logs: 64,
            outstanding_calls: 1024,
            control_reserve: 64,
            read_bytes: 1024 * 1024,
            close_wait: Duration::from_secs(60),
        }
    }
}

impl Limits {
    /// `open`'s validation (§1). A refusal is `invalid_request`, before any
    /// effect, and names the limit.
    ///
    /// The contract states one rule: the operation table holds at least the
    /// running plus the queued operations. The others refuse values under
    /// which a rule the contract states could never hold:
    /// - each count is at least 1: with no running slot, queue place, direct
    ///   worker, open log or outstanding call, an operation could never run,
    ///   wait (§5.1), be read (§5.4) or be sent at all, and with no control
    ///   reserve `session.close` could never be sent (§3);
    /// - the event log holds at least 2, a reset event and a later one (§5.4);
    /// - a read's byte limit is at most half the frame size, leaving the other
    ///   half for its reply's envelope (§3);
    /// - a channel queue's capacity, the outstanding calls plus the control
    ///   reserve (§3), does not overflow;
    /// - close waits at most an hour, so its deadline cannot overflow (§8).
    pub(crate) fn validate(&self) -> ModelResult<()> {
        let minimums = [
            ("running_operations", self.running_operations, 1),
            ("queued_operations", self.queued_operations, 1),
            ("direct_workers", self.direct_workers, 1),
            ("event_log", self.event_log, 2),
            ("open_logs", self.open_logs, 1),
            ("outstanding_calls", self.outstanding_calls, 1),
            ("control_reserve", self.control_reserve, 1),
            ("read_bytes", self.read_bytes, 1),
        ];
        for (name, value, minimum) in minimums {
            if value < minimum {
                return Err(refuse(format!(
                    "session limit {name} must be at least {minimum}"
                )));
            }
        }
        let live = self.running_operations.checked_add(self.queued_operations);
        if live.is_none_or(|live| self.operation_table < live) {
            return Err(refuse(
                "session limit operation_table must hold at least running_operations plus queued_operations",
            ));
        }
        if self.read_bytes > MAX_FRAME_BYTES / 2 {
            return Err(refuse(
                "session limit read_bytes must be at most half the 64 MiB frame size",
            ));
        }
        if self
            .outstanding_calls
            .checked_add(self.control_reserve)
            .is_none()
        {
            return Err(refuse(
                "session limits outstanding_calls plus control_reserve overflow a channel queue",
            ));
        }
        if self.close_wait > MAX_CLOSE_WAIT {
            return Err(refuse("session limit close_wait must be at most one hour"));
        }
        Ok(())
    }
}

fn refuse(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(limits: &Limits) -> ModelError {
        limits.validate().expect_err("the limits are refused")
    }

    #[test]
    fn the_defaults_are_the_contract_table_and_are_valid() {
        let limits = Limits::default();
        assert_eq!(limits.running_operations, 8);
        assert_eq!(limits.queued_operations, 64);
        assert_eq!(limits.operation_table, 128);
        assert_eq!(limits.direct_workers, 8);
        assert_eq!(limits.event_log, 4096);
        assert_eq!(limits.open_logs, 64);
        assert_eq!(limits.outstanding_calls, 1024);
        assert_eq!(limits.control_reserve, 64);
        assert_eq!(limits.read_bytes, 1024 * 1024);
        assert_eq!(limits.close_wait, Duration::from_secs(60));
        assert_eq!(MAX_READ_WAIT, Duration::from_secs(30));
        assert_eq!(MAX_FRAME_BYTES, 64 * 1024 * 1024);
        limits.validate().expect("the defaults are valid");
    }

    #[test]
    fn the_operation_table_holds_at_least_the_running_plus_the_queued_operations() {
        let mut limits = Limits {
            running_operations: 3,
            queued_operations: 5,
            operation_table: 8,
            ..Limits::default()
        };
        limits
            .validate()
            .expect("a table of exactly running plus queued is valid");
        limits.operation_table = 7;
        assert_eq!(refused(&limits).code, ErrorCode::InvalidRequest);
        // A sum that overflows can never be held.
        limits.running_operations = usize::MAX;
        limits.operation_table = usize::MAX;
        assert_eq!(refused(&limits).code, ErrorCode::InvalidRequest);
    }

    /// A limit's name, and a change that leaves it no capacity.
    type Case = (&'static str, fn(&mut Limits));

    #[test]
    fn each_limit_refuses_a_value_that_leaves_it_no_capacity() {
        let cases: [Case; 9] = [
            ("running_operations", |limits| limits.running_operations = 0),
            ("queued_operations", |limits| limits.queued_operations = 0),
            ("direct_workers", |limits| limits.direct_workers = 0),
            // An overflowed log keeps a reset event followed by later events.
            ("event_log", |limits| limits.event_log = 1),
            ("open_logs", |limits| limits.open_logs = 0),
            ("outstanding_calls", |limits| limits.outstanding_calls = 0),
            // Without a reserve, `session.close` could never be sent.
            ("control_reserve", |limits| limits.control_reserve = 0),
            ("read_bytes", |limits| limits.read_bytes = 0),
            // A read's reply must fit in a frame.
            ("read_bytes", |limits| limits.read_bytes = MAX_FRAME_BYTES),
        ];
        for (name, change) in cases {
            let mut limits = Limits::default();
            change(&mut limits);
            let error = refused(&limits);
            assert_eq!(error.code, ErrorCode::InvalidRequest, "{name}");
            assert!(error.message.contains(name), "{name}: {}", error.message);
        }
    }

    #[test]
    fn a_channel_queue_cannot_overflow() {
        let limits = Limits {
            outstanding_calls: usize::MAX,
            control_reserve: 1,
            ..Limits::default()
        };
        assert_eq!(refused(&limits).code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn a_read_leaves_half_the_frame_for_its_reply_envelope() {
        let mut limits = Limits {
            read_bytes: MAX_FRAME_BYTES / 2,
            ..Limits::default()
        };
        limits.validate().expect("half the frame size is accepted");
        for read_bytes in [MAX_FRAME_BYTES / 2 + 1, MAX_FRAME_BYTES - 1] {
            limits.read_bytes = read_bytes;
            let error = refused(&limits);
            assert_eq!(error.code, ErrorCode::InvalidRequest, "{read_bytes}");
            assert!(error.message.contains("read_bytes"), "{}", error.message);
        }
    }

    #[test]
    fn close_wait_is_at_most_one_hour() {
        let hour = Duration::from_secs(60 * 60);
        let mut limits = Limits::default();
        for accepted in [Duration::ZERO, hour] {
            limits.close_wait = accepted;
            limits.validate().expect("zero and one hour are accepted");
        }
        for too_long in [hour + Duration::from_nanos(1), Duration::MAX] {
            limits.close_wait = too_long;
            let error = refused(&limits);
            assert_eq!(error.code, ErrorCode::InvalidRequest);
            assert!(error.message.contains("close_wait"), "{}", error.message);
        }
    }
}
