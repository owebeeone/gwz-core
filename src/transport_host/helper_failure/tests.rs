//! The words for a lookup that gave up waiting for the host's spawn gate (WH2 contract C9): it is a busy host, not a
//! helper that never answered, because no helper of this lookup was started.
use super::*;
use crate::git::endpoint::{https_auth::AuthError, https_worker::helper_timeout};
use gwz_transport::protocol::Facts;

#[test]
fn a_gate_wait_that_ran_out_is_worded_as_a_busy_helper() {
    let failure = helper_timeout(AuthError::GateTimeout, &Facts::default(), 0, 2_500);
    assert_eq!(failure.code, ErrorCode::Timeout);
    assert_eq!(failure.setup_cause, Some(SetupFailureCause::Allocation));
    let reason = timeout_reason(&failure).expect("a gate timeout is worded");
    assert!(reason.starts_with(HELPER_ALLOCATION_PREFIX), "{reason}");
    assert!(reason.contains("2.5 seconds"), "{reason}");
    assert!(
        reason.contains("Other helpers may be busy, waiting for sign-ins"),
        "{reason}"
    );
    assert!(!reason.contains(HELPER_INTERACTION_PREFIX), "{reason}");
}

#[test]
fn an_interaction_timeout_keeps_its_words() {
    let failure = helper_timeout(AuthError::Timeout, &Facts::default(), 0, 2_500);
    assert_eq!(failure.setup_cause, Some(SetupFailureCause::Interaction));
    let reason = timeout_reason(&failure).expect("an interaction timeout is worded");
    assert!(reason.starts_with(HELPER_INTERACTION_PREFIX), "{reason}");
}
