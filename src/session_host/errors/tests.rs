//! Tests of how core reports the session crates' errors (crate map §6 step
//! 4): each keeps the code and the words it had before the move.

use super::*;
use crate::session_host::context::test_session;
use gwz_session_host::CallControls;

#[test]
fn a_refused_crossing_is_cancelled_with_its_text() {
    let cases = [
        (Refused::Cancelled, "the operation's token is cancelled"),
        (Refused::Revoked, "the operation's gate is revoked"),
    ];
    for (refused, text) in cases {
        let error = ModelError::from(refused);
        assert_eq!(error.code, ErrorCode::Cancelled, "{text}");
        assert_eq!(error.message, text);
    }
}

#[test]
fn a_gate_over_the_session_context_reaches_its_limits() {
    // Core's session context is the crate's per-session data: a crossing
    // reaches it through its scope, and a refusal reports as `cancelled`.
    let channel = test_session();
    let (controls, mut gate) = CallControls::new(&channel.session());
    let running = gate.effect(|scope| scope.session().limits().running_operations);
    assert_eq!(running, Ok(8));
    controls.cancel();
    let refused = gate
        .effect(|scope| scope.session().limits().open_logs)
        .map_err(ModelError::from)
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Cancelled);
    assert_eq!(refused.message, "the operation's token is cancelled");
}
