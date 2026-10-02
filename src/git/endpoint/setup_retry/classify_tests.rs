//! The retry plan's §4 sets, one row per code and origin.
use super::*;
use gwz_transport::protocol::Effect;

fn failure(code: ErrorCode, setup_cause: Option<SetupFailureCause>) -> Failure {
    Failure {
        setup_cause,
        code,
        effect: Effect::None,
        facts: None,
    }
}

#[test]
fn stall_and_aggregate_setup_timeouts_are_retried_and_their_origin_decides() {
    for cause in [SetupFailureCause::Stall, SetupFailureCause::Aggregate] {
        let timeout = failure(ErrorCode::Timeout, Some(cause));
        assert_eq!(
            classify(&timeout, Phase::Setup),
            Verdict::Retry,
            "{cause:?}"
        );
    }
    // `ErrorCode::Timeout` alone is not enough: a timeout without an origin
    // is not in the retriable set.
    let bare = failure(ErrorCode::Timeout, None);
    assert_eq!(classify(&bare, Phase::Setup), Verdict::Close);
}

#[test]
fn an_interaction_timeout_closes_and_an_allocation_timeout_is_returned_once() {
    let interaction = failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction));
    assert_eq!(classify(&interaction, Phase::Setup), Verdict::Close);
    // An allocation timeout ends an open that never set up.
    let allocation = failure(ErrorCode::Timeout, Some(SetupFailureCause::Allocation));
    assert_eq!(classify(&allocation, Phase::Setup), Verdict::Return);
    assert_eq!(classify(&allocation, Phase::Other), Verdict::Return);
}

#[test]
fn io_and_carrier_loss_in_setup_are_retried() {
    for code in [ErrorCode::Io, ErrorCode::CarrierLost] {
        assert_eq!(
            classify(&failure(code, None), Phase::Setup),
            Verdict::Retry,
            "{code:?}"
        );
    }
}

#[test]
fn a_refused_connection_is_retried_and_a_missing_agent_or_address_is_not() {
    let refused = failure(
        ErrorCode::Unavailable,
        Some(SetupFailureCause::ConnectionRefused),
    );
    assert_eq!(classify(&refused, Phase::Setup), Verdict::Retry);
    for cause in [
        Some(SetupFailureCause::NotFound),
        Some(SetupFailureCause::AddressNotAvailable),
        None,
    ] {
        let unavailable = failure(ErrorCode::Unavailable, cause);
        assert_eq!(
            classify(&unavailable, Phase::Setup),
            Verdict::Close,
            "{cause:?}"
        );
    }
}

#[test]
fn authentication_trust_and_protocol_failures_close_the_key() {
    for code in [
        ErrorCode::Authentication,
        ErrorCode::Trust,
        ErrorCode::Protocol,
        ErrorCode::InvalidRequest,
        ErrorCode::RepositoryRefused,
        ErrorCode::UnsupportedOperation,
        ErrorCode::UnsupportedVersion,
    ] {
        assert_eq!(
            classify(&failure(code, None), Phase::Setup),
            Verdict::Close,
            "{code:?}"
        );
    }
}

#[test]
fn cancellation_and_capacity_are_returned_without_moving_the_key() {
    // A cancelled setup and one refused by a local budget say nothing about
    // the server: §4 does not retry them, and neither closes the key.
    for code in [ErrorCode::Cancelled, ErrorCode::Capacity] {
        assert_eq!(
            classify(&failure(code, None), Phase::Setup),
            Verdict::Return,
            "{code:?}"
        );
    }
}

#[test]
fn nothing_after_the_session_is_reusable_is_retried_or_closes_the_key() {
    // A timeout while a fetch or push body is in progress, a reset, and a
    // refusal outside setup are all returned once.
    for (code, cause) in [
        (ErrorCode::Timeout, Some(SetupFailureCause::Stall)),
        (ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)),
        (ErrorCode::Io, None),
        (ErrorCode::CarrierLost, None),
        (
            ErrorCode::Unavailable,
            Some(SetupFailureCause::ConnectionRefused),
        ),
        (ErrorCode::Authentication, None),
    ] {
        assert_eq!(
            classify(&failure(code, cause), Phase::Other),
            Verdict::Return,
            "{code:?} {cause:?}"
        );
    }
}

#[test]
fn the_pool_errors_that_end_a_started_setup_have_the_setup_phase() {
    let connect = pool::Error::ConnectFailed {
        code: ErrorCode::Io,
        effect: Effect::None,
        setup_cause: None,
    };
    for error in [
        connect,
        pool::Error::ConnectTimeout,
        pool::Error::InteractionTimeout,
        pool::Error::IdentityMismatch,
    ] {
        assert_eq!(phase_of(&error), Phase::Setup, "{error:?}");
    }
    for error in [
        pool::Error::AllocationTimeout,
        pool::Error::Capacity,
        pool::Error::WouldBlock,
        pool::Error::Cancelled,
        pool::Error::Shutdown,
        pool::Error::DriverLost,
        pool::Error::Stale,
        pool::Error::WrongState,
        pool::Error::InvalidRequest,
        pool::Error::InvalidConfig,
        pool::Error::ActiveOperation,
    ] {
        assert_eq!(phase_of(&error), Phase::Other, "{error:?}");
    }
}

#[test]
fn only_a_spent_budget_shows_its_attempt_on_the_failure_alone() {
    for spent in [
        failure(ErrorCode::Timeout, Some(SetupFailureCause::Stall)),
        failure(ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)),
        failure(
            ErrorCode::Unavailable,
            Some(SetupFailureCause::ConnectionRefused),
        ),
    ] {
        assert_eq!(spent_budget(&spent, 3), Some((4, 4)), "{spent:?}");
        assert_eq!(spent_budget(&spent, 0), Some((1, 1)), "{spent:?}");
    }
    // These can end an earlier attempt, or an open outside setup: the
    // failure alone does not say which attempt it ended.
    for other in [
        failure(ErrorCode::Io, None),
        failure(ErrorCode::Authentication, None),
        failure(ErrorCode::Timeout, None),
        failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction)),
        failure(ErrorCode::Unavailable, Some(SetupFailureCause::NotFound)),
    ] {
        assert_eq!(spent_budget(&other, 3), None, "{other:?}");
    }
}

#[test]
fn a_members_attempts_merge_into_its_one_diagnostic_row() {
    use gwz_transport::protocol::{AuthMethod, Facts};
    let offered = Facts {
        method: AuthMethod::SshAgent,
        credential_offered: true,
        authenticated: Some(false),
        key_fingerprint: Some("SHA256:first".into()),
        ..Facts::default()
    };
    // A later attempt that stalled before it offered anything keeps the
    // earlier offer on the row.
    let merged = merged_facts(Some(offered.clone()), Some(Facts::default())).unwrap();
    assert_eq!(merged, offered);
    // A later attempt's own values win.
    let succeeded = Facts {
        method: AuthMethod::SshKey,
        credential_offered: true,
        authenticated: Some(true),
        ..Facts::default()
    };
    let merged = merged_facts(Some(offered.clone()), Some(succeeded)).unwrap();
    assert_eq!(merged.method, AuthMethod::SshKey);
    assert_eq!(merged.authenticated, Some(true));
    assert_eq!(merged.key_fingerprint.as_deref(), Some("SHA256:first"));
    assert_eq!(
        merged_facts(None, Some(offered.clone())),
        Some(offered.clone())
    );
    assert_eq!(merged_facts(Some(offered.clone()), None), Some(offered));
    assert_eq!(merged_facts(None, None), None);
}
