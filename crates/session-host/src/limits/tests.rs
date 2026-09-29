#![cfg(test)]
//! Tests of the session limits (session plan CS1.4), moved from gwz-core's
//! `session_host/limits.rs` with their names. A refusal is now the crate's
//! `InvalidLimits`, whose text is core's `open` refusal; core maps it to
//! `invalid_request` (its own tests keep that half).

use std::time::Duration;

use crate::{InvalidLimits, Limits, MAX_READ_WAIT, validate_limits};
use gwz_session_contract::MAX_FRAME_BYTES;

fn refused(limits: &Limits) -> InvalidLimits {
    validate_limits(limits).expect_err("the limits are refused")
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
    validate_limits(&limits).expect("the defaults are valid");
}

#[test]
fn the_operation_table_holds_at_least_the_running_plus_the_queued_operations() {
    let mut limits = Limits {
        running_operations: 3,
        queued_operations: 5,
        operation_table: 8,
        ..Limits::default()
    };
    validate_limits(&limits).expect("a table of exactly running plus queued is valid");
    limits.operation_table = 7;
    assert_eq!(refused(&limits), InvalidLimits::TableTooSmall);
    // A sum that overflows can never be held.
    limits.running_operations = usize::MAX;
    limits.operation_table = usize::MAX;
    assert_eq!(refused(&limits), InvalidLimits::TableTooSmall);
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
        let message = refused(&limits).to_string();
        assert!(message.contains(name), "{name}: {message}");
    }
}

#[test]
fn a_channel_queue_cannot_overflow() {
    let limits = Limits {
        outstanding_calls: usize::MAX,
        control_reserve: 1,
        ..Limits::default()
    };
    assert_eq!(refused(&limits), InvalidLimits::QueueOverflow);
}

#[test]
fn a_read_leaves_half_the_frame_for_its_reply_envelope() {
    let mut limits = Limits {
        read_bytes: MAX_FRAME_BYTES / 2,
        ..Limits::default()
    };
    validate_limits(&limits).expect("half the frame size is accepted");
    for read_bytes in [MAX_FRAME_BYTES / 2 + 1, MAX_FRAME_BYTES - 1] {
        limits.read_bytes = read_bytes;
        let error = refused(&limits);
        assert_eq!(error, InvalidLimits::ReadTooLarge, "{read_bytes}");
        assert!(error.to_string().contains("read_bytes"), "{error}");
    }
}

#[test]
fn close_wait_is_at_most_one_hour() {
    let hour = Duration::from_secs(60 * 60);
    let mut limits = Limits::default();
    for accepted in [Duration::ZERO, hour] {
        limits.close_wait = accepted;
        validate_limits(&limits).expect("zero and one hour are accepted");
    }
    for too_long in [hour + Duration::from_nanos(1), Duration::MAX] {
        limits.close_wait = too_long;
        let error = refused(&limits);
        assert_eq!(error, InvalidLimits::CloseWaitTooLong);
        assert!(error.to_string().contains("close_wait"), "{error}");
    }
}

#[test]
fn valid_limits_give_the_channel_its_two() {
    // The channel's queues hold the outstanding calls plus the control
    // reserve (contract §3), which gwz-session-channel's `pair` takes.
    let limits = Limits {
        outstanding_calls: 3,
        control_reserve: 2,
        ..Limits::default()
    };
    let channel = validate_limits(&limits).expect("valid limits");
    assert_eq!(channel.outstanding_calls(), 3);
    assert_eq!(channel.control_reserve(), 2);
    let defaults = validate_limits(&Limits::default()).expect("the defaults are valid");
    assert_eq!(defaults, gwz_session_contract::Limits::default());
}

#[test]
fn refusals_say_what_core_says_today() {
    // gwz-core's `open` reports each with `invalid_request` and this text,
    // as `Limits::validate` worded it before the move.
    let texts = [
        (
            InvalidLimits::TooSmall {
                limit: "event_log",
                minimum: 2,
            },
            "session limit event_log must be at least 2",
        ),
        (
            InvalidLimits::TableTooSmall,
            "session limit operation_table must hold at least running_operations plus queued_operations",
        ),
        (
            InvalidLimits::ReadTooLarge,
            "session limit read_bytes must be at most half the 64 MiB frame size",
        ),
        (
            InvalidLimits::QueueOverflow,
            "session limits outstanding_calls plus control_reserve overflow a channel queue",
        ),
        (
            InvalidLimits::CloseWaitTooLong,
            "session limit close_wait must be at most one hour",
        ),
    ];
    for (error, text) in texts {
        assert_eq!(error.to_string(), text);
    }
}
