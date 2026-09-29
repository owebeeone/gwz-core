//! The channel's limits (§1, §3).

use std::error::Error;
use std::fmt;

use crate::channel::Lane;

/// The channel's limits: each queue holds `outstanding_calls` frames on the
/// call lane and `control_reserve` frames on the control lane (§3).
///
/// They are two of the contract's §1 limits, the two the channel enforces.
/// The others, such as running operations, the queue and the operation
/// table, are the session host's, and `open` validates all of them (§1, §9).
/// A host builds this value from its own, as core's `session_host::Limits`
/// holds both today.
///
/// A `Limits` is always valid. [`Limits::new`] refuses a limit of zero, since
/// without an outstanding call nothing could be sent and without a control
/// reserve `session.close` could never be sent (§3). It also refuses limits
/// whose sum, a queue's capacity, would overflow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Limits {
    outstanding_calls: usize,
    control_reserve: usize,
}

impl Limits {
    /// The contract's default outstanding-call limit (§1).
    pub const DEFAULT_OUTSTANDING_CALLS: usize = 1024;
    /// The contract's default control reserve, per queue (§1).
    pub const DEFAULT_CONTROL_RESERVE: usize = 64;

    /// Limits of `outstanding_calls` on the call lane and `control_reserve`
    /// on the control lane.
    ///
    /// # Errors
    ///
    /// A limit of zero, checked in that order, or a sum that overflows.
    pub const fn new(
        outstanding_calls: usize,
        control_reserve: usize,
    ) -> Result<Self, LimitsError> {
        if outstanding_calls == 0 {
            Err(LimitsError::NoOutstandingCalls)
        } else if control_reserve == 0 {
            Err(LimitsError::NoControlReserve)
        } else if outstanding_calls.checked_add(control_reserve).is_none() {
            Err(LimitsError::Overflow)
        } else {
            Ok(Self {
                outstanding_calls,
                control_reserve,
            })
        }
    }

    /// Ordinary calls outstanding at once: the call lane's room.
    pub const fn outstanding_calls(self) -> usize {
        self.outstanding_calls
    }

    /// Control frames beyond the outstanding calls: the control lane's room.
    pub const fn control_reserve(self) -> usize {
        self.control_reserve
    }

    /// The room `lane` has in each queue.
    pub const fn lane_capacity(self, lane: Lane) -> usize {
        match lane {
            Lane::Call => self.outstanding_calls,
            Lane::Control => self.control_reserve,
        }
    }

    /// A queue's capacity: the outstanding-call limit plus the control
    /// reserve (§3). [`Limits::new`] ensures the sum does not overflow.
    pub const fn queue_capacity(self) -> usize {
        self.outstanding_calls + self.control_reserve
    }
}

impl Default for Limits {
    /// The contract's defaults: 1024 outstanding calls and a control reserve
    /// of 64 frames (§1).
    fn default() -> Self {
        Self {
            outstanding_calls: Self::DEFAULT_OUTSTANDING_CALLS,
            control_reserve: Self::DEFAULT_CONTROL_RESERVE,
        }
    }
}

/// Limits a channel cannot have. The texts are core's `open` refusals for the
/// same limits, which it reports as `invalid_request` (§1, §9).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LimitsError {
    /// An outstanding-call limit of zero.
    NoOutstandingCalls,
    /// A control reserve of zero.
    NoControlReserve,
    /// Limits whose sum, a queue's capacity, overflows.
    Overflow,
}

impl fmt::Display for LimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            LimitsError::NoOutstandingCalls => "session limit outstanding_calls must be at least 1",
            LimitsError::NoControlReserve => "session limit control_reserve must be at least 1",
            LimitsError::Overflow => {
                "session limits outstanding_calls plus control_reserve overflow a channel queue"
            }
        })
    }
}

impl Error for LimitsError {}
