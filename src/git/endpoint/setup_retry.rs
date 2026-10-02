//! Connection setup retry: the closed classifier of the retry plan's §4, used
//! by the SSH and the HTTPS endpoint alike (gwz-core
//! dev-docs/GwzRemoteTransportRetryPlan.md).
//!
//! The endpoint that owns a setup failure knows where it happened, so it
//! passes that [`Phase`] with the failure. The classifier then puts the
//! failure in one of three places: retried after a wait, closing its key for
//! the operation, or returned once to its member without moving the key.
use gwz_transport::{
    pool,
    protocol::{AuthMethod, ErrorCode, Facts, Failure, SetupFailureCause},
};

mod backoff;
mod machine;
mod operations;

pub(crate) use backoff::{Jitter, wait_bound_ms};
pub(crate) use machine::{Decision, Machine, Outcome};
pub(crate) use operations::{DEFAULT_MAX_RETRIES, Operations};

/// Where an open's failure happened, as the endpoint that owns it saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// A started setup failed before its session was admitted as reusable:
    /// on SSH before `setup_is_reusable` accepts the session, on HTTPS in the
    /// connect before the open's first request byte.
    Setup,
    /// Anything else: an open that waited out its allocation, was refused
    /// capacity, was cancelled or lost its endpoint, and every failure after
    /// the session became reusable. None of these is a setup's own outcome.
    Other,
}

/// What a failed open does to its member and its key (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// In the closed retriable set: the key waits, then one new attempt starts.
    Retry,
    /// A setup failure outside that set: the key is closed for the operation,
    /// and every member on it finishes with this failure.
    Close,
    /// Not the outcome of a setup: returned once to its own member, and the
    /// key does not move.
    Return,
}

/// The closed classifier. A failure is retried only when it is a setup's own
/// failure (`phase`), and its code is a setup `Timeout` whose origin is stall
/// or aggregate, `Io`, `CarrierLost`, or `Unavailable` caused by a refused
/// connection. `--max-retries 0` is the machine's: it closes the key on the
/// first retriable failure, as the attempt budget is then spent.
pub(crate) fn classify(failure: &Failure, phase: Phase) -> Verdict {
    if phase != Phase::Setup {
        return Verdict::Return;
    }
    match (failure.code, failure.setup_cause) {
        (ErrorCode::Timeout, Some(SetupFailureCause::Stall | SetupFailureCause::Aggregate))
        | (ErrorCode::Io | ErrorCode::CarrierLost, _)
        | (ErrorCode::Unavailable, Some(SetupFailureCause::ConnectionRefused)) => Verdict::Retry,
        // An allocation timeout ends an open that never set up. A cancelled
        // setup, or one a local budget refused, says nothing about the server.
        (ErrorCode::Timeout, Some(SetupFailureCause::Allocation))
        | (ErrorCode::Cancelled | ErrorCode::Capacity, _) => Verdict::Return,
        _ => Verdict::Close,
    }
}

/// The phase of an open that the transport pool failed: the pool's connect
/// failures, its aggregate and interaction clocks, and a mismatched identity
/// end a started setup; every other pool error ends an open before or
/// without one.
pub(crate) fn phase_of(error: &pool::Error) -> Phase {
    match error {
        pool::Error::ConnectFailed { .. }
        | pool::Error::ConnectTimeout
        | pool::Error::InteractionTimeout
        | pool::Error::IdentityMismatch => Phase::Setup,
        _ => Phase::Other,
    }
}

/// The attempt a final failure ended, as `(N, M)` for its display's
/// `attempt N of M` (the retry plan's §5), when the failure alone says so.
/// A setup timeout whose origin is stall or aggregate, and a refused
/// connection, can end an open only once its key has spent the budget of
/// `max_retries`: they are retried until then, and nothing outside a setup
/// reports them. Any other failure may end an earlier attempt, and the
/// transport's failure carries no attempt number to tell which.
pub(crate) fn spent_budget(failure: &Failure, max_retries: u32) -> Option<(u32, u32)> {
    let only_retried = matches!(
        (failure.code, failure.setup_cause),
        (
            ErrorCode::Timeout,
            Some(SetupFailureCause::Stall | SetupFailureCause::Aggregate)
        ) | (
            ErrorCode::Unavailable,
            Some(SetupFailureCause::ConnectionRefused)
        )
    );
    let attempts = max_retries.saturating_add(1);
    only_retried.then_some((attempts, attempts))
}

/// One member's attempts as its one diagnostic row: each intermediate
/// attempt is progress on the row its open already has (the retry plan's
/// §5). A later attempt's value wins where it has one, and a credential
/// offered in any attempt stays offered.
pub(crate) fn merged_facts(earlier: Option<Facts>, later: Option<Facts>) -> Option<Facts> {
    let (earlier, later) = match (earlier, later) {
        (Some(earlier), Some(later)) => (earlier, later),
        (earlier, later) => return later.or(earlier),
    };
    Some(Facts {
        method: if later.method == AuthMethod::None {
            earlier.method
        } else {
            later.method
        },
        credential_offered: earlier.credential_offered || later.credential_offered,
        authenticated: later.authenticated.or(earlier.authenticated),
        key_fingerprint: later.key_fingerprint.or(earlier.key_fingerprint),
        http_status: later.http_status.or(earlier.http_status),
        ssh_exit_status: later.ssh_exit_status.or(earlier.ssh_exit_status),
    })
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod classify_tests;
        mod machine_tests;
    }
}
