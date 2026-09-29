//! The session limits of the core session contract's §1 and their
//! validation (session plan CS1.4), moved from gwz-core's
//! `session_host/limits.rs` with a crate-local error.
//!
//! Each limit is set at `open` (§9) unless the contract marks it fixed. The
//! fixed ones are [`MAX_READ_WAIT`] here and the frame size, which is
//! `gwz_session_contract::MAX_FRAME_BYTES`. The host enforces the others:
//! the running limit, the queue and the operation table in admission (§5.1,
//! §5.4), the direct workers in §5.2, the event log and the open logs in
//! §5.4, the read size in §4.2 and the close wait in §8. The channel
//! enforces the outstanding calls and the control reserve (§3), which
//! [`validate_limits`] hands it.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use gwz_session_contract::{Limits as ChannelLimits, LimitsError, MAX_FRAME_BYTES};

/// The longest wait a held log read may ask for, fixed at 30 seconds (§1,
/// §4.2).
pub const MAX_READ_WAIT: Duration = Duration::from_secs(30);

/// The longest `close_wait` that validation accepts.
const MAX_CLOSE_WAIT: Duration = Duration::from_secs(60 * 60);

/// The limits of one session, set at `open` (§1, §9).
///
/// `Default` gives the contract's defaults. The type is `non_exhaustive`:
/// start from `Limits::default()` and change the fields a driver needs.
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

/// Validates `limits` as `open` must (§1) and returns the channel's two,
/// which gwz-session-channel's `pair` takes (§3).
///
/// The contract states one rule: the operation table holds at least the
/// running plus the queued operations. The others refuse values under which a
/// rule the contract states could never hold:
/// - each count is at least 1: with no running slot, queue place, direct
///   worker, open log or outstanding call, an operation could never run, wait
///   (§5.1), be read (§5.4) or be sent at all, and with no control reserve
///   `session.close` could never be sent (§3);
/// - the event log holds at least 2, a reset event and a later one (§5.4);
/// - a read's byte limit is at most half the frame size, leaving the other
///   half for its reply's envelope (§3);
/// - a channel queue's capacity, the outstanding calls plus the control
///   reserve (§3), does not overflow;
/// - close waits at most an hour, so its deadline cannot overflow (§8).
///
/// It is a function rather than a method so that gwz-core's frozen
/// `session_host::Limits`, which re-exports this type, keeps exactly its
/// frozen surface.
///
/// # Errors
///
/// The first rule `limits` breaks, in the order above. gwz-core reports it
/// as `invalid_request` with its text, before any effect.
pub fn validate_limits(limits: &Limits) -> Result<ChannelLimits, InvalidLimits> {
    let minimums = [
        ("running_operations", limits.running_operations, 1),
        ("queued_operations", limits.queued_operations, 1),
        ("direct_workers", limits.direct_workers, 1),
        ("event_log", limits.event_log, 2),
        ("open_logs", limits.open_logs, 1),
        ("outstanding_calls", limits.outstanding_calls, 1),
        ("control_reserve", limits.control_reserve, 1),
        ("read_bytes", limits.read_bytes, 1),
    ];
    for (limit, value, minimum) in minimums {
        if value < minimum {
            return Err(InvalidLimits::TooSmall { limit, minimum });
        }
    }
    let live = limits
        .running_operations
        .checked_add(limits.queued_operations);
    if live.is_none_or(|live| limits.operation_table < live) {
        return Err(InvalidLimits::TableTooSmall);
    }
    if limits.read_bytes > MAX_FRAME_BYTES / 2 {
        return Err(InvalidLimits::ReadTooLarge);
    }
    let channel = ChannelLimits::new(limits.outstanding_calls, limits.control_reserve);
    let channel = channel.map_err(|error| match error {
        LimitsError::NoOutstandingCalls => InvalidLimits::TooSmall {
            limit: "outstanding_calls",
            minimum: 1,
        },
        LimitsError::NoControlReserve => InvalidLimits::TooSmall {
            limit: "control_reserve",
            minimum: 1,
        },
        LimitsError::Overflow => InvalidLimits::QueueOverflow,
    })?;
    if limits.close_wait > MAX_CLOSE_WAIT {
        return Err(InvalidLimits::CloseWaitTooLong);
    }
    Ok(channel)
}

/// Limits a session cannot have. The texts are those of gwz-core's `open`
/// refusals, which it reports as `invalid_request` (§1, §9).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InvalidLimits {
    /// A count below its minimum.
    TooSmall {
        /// The limit's field name.
        limit: &'static str,
        /// The least value it takes.
        minimum: usize,
    },
    /// An operation table smaller than the running plus the queued
    /// operations.
    TableTooSmall,
    /// A read's byte limit above half the frame size.
    ReadTooLarge,
    /// Outstanding calls plus the control reserve overflow a queue's
    /// capacity.
    QueueOverflow,
    /// A close wait above one hour.
    CloseWaitTooLong,
}

impl fmt::Display for InvalidLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidLimits::TooSmall { limit, minimum } => {
                write!(formatter, "session limit {limit} must be at least {minimum}")
            }
            InvalidLimits::TableTooSmall => formatter.write_str(
                "session limit operation_table must hold at least running_operations plus queued_operations",
            ),
            InvalidLimits::ReadTooLarge => formatter
                .write_str("session limit read_bytes must be at most half the 64 MiB frame size"),
            InvalidLimits::QueueOverflow => formatter.write_str(
                "session limits outstanding_calls plus control_reserve overflow a channel queue",
            ),
            InvalidLimits::CloseWaitTooLong => {
                formatter.write_str("session limit close_wait must be at most one hour")
            }
        }
    }
}

impl Error for InvalidLimits {}

mod tests;
