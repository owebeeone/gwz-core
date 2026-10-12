//! What a failed attempt is to the limit machine, and what its member does
//! about it (adaptive concurrency design §3, §4.8, §5.1 and §5.3): a failure
//! that looks like a limit is *requeued*, not counted against the key; a
//! member that has spent its `--max-retries + 1` attempts finishes, and one
//! whose own final failure is a throttle finishes with `Capacity`.
use super::{
    Phase, Signal,
    limit::{CapacityReport, Ruling, ServerCapacity},
};
use gwz_transport::protocol::{ErrorCode, Failure, FailureDetail, RetryAttempt, SetupFailureCause};

/// The class of a setup failure that may be a concurrency limit (§3.2), if it
/// is one. A failure that is not a candidate returns `None` and stays the
/// retry machine's.
///
/// On SSH a refused connect and a connect stall or aggregate timeout are
/// candidates for `N`; a drop at the banner or key exchange (`Io`,
/// `CarrierLost`, which `ssh_network` also makes of a Windows abort) is the
/// setup limit `Ns`'s evidence (§4.10) when it came before authentication:
/// the setup had offered no credential. A failure with no facts says nothing
/// of its stage and stays the retry machine's.
pub(crate) fn suspect(failure: &Failure, phase: Phase, ssh: bool) -> Option<Signal> {
    if phase != Phase::Setup {
        return None;
    }
    match (failure.code, failure.setup_cause) {
        (ErrorCode::Io | ErrorCode::CarrierLost, _) if !ssh => Some(Signal::Suspect),
        (ErrorCode::Io | ErrorCode::CarrierLost, _)
            if failure
                .facts
                .as_ref()
                .is_some_and(|facts| !facts.credential_offered) =>
        {
            Some(Signal::SetupDrop)
        }
        (ErrorCode::Unavailable, Some(SetupFailureCause::ConnectionRefused))
        | (ErrorCode::Timeout, Some(SetupFailureCause::Stall | SetupFailureCause::Aggregate)) => {
            Some(Signal::Suspect)
        }
        _ => None,
    }
}

/// Whether the member is requeued rather than handled as before: a throttle
/// always is (a throttled key is healthy and narrow, §5.1), and any other
/// refusal when the machine took it as evidence. A refusal with nothing else
/// counted (`hi = 0`), or one the machine did not begin, is the retry
/// machine's, as it was.
pub(crate) fn requeues(ruling: Option<Ruling>, throttled: bool) -> bool {
    throttled || ruling.is_some_and(|ruling| ruling != Ruling::RetryMachine)
}

/// The failure a member finishes with when its `attempts` allowed attempts
/// are spent: its own final failure, carrying the attempt count once, and
/// `Capacity` with the `capacity` detail of `throttle` when that failure is a
/// throttle (§5.3: one rule decides the code).
pub(crate) fn spent(
    mut failure: Failure,
    throttle: Option<ServerCapacity>,
    attempt: u32,
    attempts: u32,
) -> Failure {
    if let Some(server) = throttle {
        // A throttle is the whole story: whatever else the refusal carried
        // (the transport admits `capacity` beside nothing but the count) is
        // not about the throttle.
        failure.code = ErrorCode::Capacity;
        failure.setup_cause = None;
        failure.detail = Some(Box::new(FailureDetail {
            capacity: Some(CapacityReport::Server(server).detail()),
            ..FailureDetail::default()
        }));
    }
    let detail = failure
        .detail
        .get_or_insert_with(|| Box::new(FailureDetail::default()));
    detail.retry_attempt = Some(RetryAttempt {
        attempt: i64::from(attempt),
        attempts: i64::from(attempts),
    });
    failure
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use gwz_transport::protocol::{Effect, Facts};

            fn failure(code: ErrorCode, cause: Option<SetupFailureCause>) -> Failure {
                Failure { detail: None, setup_cause: cause, code, effect: Effect::None, facts: None }
            }

            #[test]
            fn setup_failures_that_look_like_a_limit_are_suspects() {
                let refused = failure(ErrorCode::Unavailable, Some(SetupFailureCause::ConnectionRefused));
                let stall = failure(ErrorCode::Timeout, Some(SetupFailureCause::Stall));
                let aggregate = failure(ErrorCode::Timeout, Some(SetupFailureCause::Aggregate));
                let reset = failure(ErrorCode::Io, None);
                for ssh in [false, true] {
                    for suspect_failure in [&refused, &stall, &aggregate] {
                        assert_eq!(suspect(suspect_failure, Phase::Setup, ssh), Some(Signal::Suspect));
                    }
                }
                // A reset or an end of stream is a limit's evidence over HTTPS.
                assert_eq!(suspect(&reset, Phase::Setup, false), Some(Signal::Suspect));
            }

            fn with_facts(mut failed: Failure, credential_offered: bool) -> Failure {
                failed.facts = Some(Facts { credential_offered, ..Facts::default() });
                failed
            }

            #[test]
            fn an_ssh_drop_before_authentication_is_the_setup_limits_and_never_ns_for_n() {
                for code in [ErrorCode::Io, ErrorCode::CarrierLost] {
                    let dropped = with_facts(failure(code, None), false);
                    assert_eq!(suspect(&dropped, Phase::Setup, true), Some(Signal::SetupDrop), "{code:?}");
                    // After a credential was offered it is not a setup drop; nor
                    // is a failure that says nothing of its stage.
                    let later = with_facts(failure(code, None), true);
                    assert_eq!(suspect(&later, Phase::Setup, true), None, "{code:?}");
                    assert_eq!(suspect(&failure(code, None), Phase::Setup, true), None, "{code:?}");
                    assert_eq!(suspect(&dropped, Phase::Other, true), None, "{code:?}");
                }
            }

            #[test]
            fn what_is_not_the_servers_is_never_a_suspect() {
                let local = [
                    failure(ErrorCode::Timeout, Some(SetupFailureCause::Allocation)),
                    failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction)),
                    failure(ErrorCode::Capacity, None),
                    failure(ErrorCode::Cancelled, None),
                    failure(ErrorCode::Authentication, None),
                    failure(ErrorCode::Unavailable, Some(SetupFailureCause::NotFound)),
                ];
                for ssh in [false, true] {
                    for failed in &local {
                        assert_eq!(suspect(failed, Phase::Setup, ssh), None, "{failed:?}");
                    }
                    // After the first request byte nothing is a setup's own.
                    assert_eq!(suspect(&failure(ErrorCode::Io, None), Phase::Other, ssh), None);
                }
            }

            #[test]
            fn a_throttle_is_requeued_and_other_refusals_when_the_machine_took_them() {
                assert!(requeues(None, true), "even with nothing in flight to judge");
                assert!(requeues(Some(Ruling::RetryMachine), true));
                for ruling in [Ruling::Inconclusive, Ruling::HoldOnly, Ruling::Confirmation, Ruling::RefusedTest, Ruling::Unfair] {
                    assert!(requeues(Some(ruling), false), "{ruling:?}");
                }
                assert!(requeues(Some(Ruling::Overload { n: 3 }), false));
                assert!(!requeues(Some(Ruling::RetryMachine), false), "hi = 0 is the retry machine's");
                assert!(!requeues(None, false));
            }

            fn throttle() -> ServerCapacity {
                ServerCapacity {
                    site: None,
                    limit_applied: Some(2),
                    max_per_host: 8,
                    elapsed_ms: 5_000,
                    last_signal: Some(crate::git::endpoint::setup_retry::LastSignal::Http429),
                    retry_after_ms: None,
                }
            }

            #[test]
            fn a_spent_member_reports_capacity_only_for_a_throttle_and_its_attempts_once() {
                let reset = spent(failure(ErrorCode::Io, None), None, 4, 4);
                assert_eq!(reset.code, ErrorCode::Io);
                let detail = reset.detail.unwrap();
                assert!(detail.capacity.is_none());
                let count = detail.retry_attempt.unwrap();
                assert_eq!((count.attempt, count.attempts), (4, 4));
                let throttled = spent(failure(ErrorCode::Io, None), Some(throttle()), 4, 4);
                assert_eq!(throttled.code, ErrorCode::Capacity);
                let detail = throttled.detail.unwrap();
                assert_eq!(detail.retry_attempt.unwrap().attempt, 4);
                let capacity = detail.capacity.unwrap();
                assert_eq!(capacity.cause, gwz_transport::protocol::CapacityCause::ServerThrottle);
                assert_eq!((capacity.limit_applied, capacity.max_per_host), (Some(2), Some(8)));
                assert_eq!(capacity.elapsed_ms, Some(5_000));
            }

            #[test]
            fn a_throttle_carries_the_capacity_detail_beside_nothing_else_the_refusal_had() {
                let mut refused = failure(ErrorCode::RepositoryRefused, Some(SetupFailureCause::Stall));
                refused.detail = Some(Box::new(FailureDetail {
                    schemes: Some(vec!["Basic".into()]),
                    ..FailureDetail::default()
                }));
                let throttled = spent(refused, Some(throttle()), 2, 4);
                assert_eq!((throttled.code, throttled.setup_cause), (ErrorCode::Capacity, None));
                let detail = throttled.detail.unwrap();
                assert!(detail.schemes.is_none());
                assert!(detail.capacity.is_some() && detail.retry_attempt.is_some());
            }
        }
    }
}
