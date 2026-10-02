use super::*;
cfg_if::cfg_if! {
    if #[cfg(test)] {
        use super::agent_job::SetupTimeout;

        #[test]
        fn success_before_the_aggregate_is_reusable() {
            let start = Instant::now();
            let decision = classify_setup_result(
                start + Duration::from_secs(5),
                Some(start + Duration::from_secs(10)),
                true,
                true,
            );
            assert!(setup_is_reusable(&decision));
        }

        #[test]
        fn success_after_the_aggregate_is_a_setup_timeout() {
            let start = Instant::now();
            let decision = classify_setup_result(
                start + Duration::from_secs(10),
                Some(start + Duration::from_secs(10)),
                true,
                true,
            );
            assert!(!setup_is_reusable(&decision));
            let failure = decision.expect_err("late success");
            assert_eq!(failure.code, ErrorCode::Timeout);
            assert_ne!(failure.code, ErrorCode::Authentication);
        }

        #[test]
        fn identity_mismatch_stays_authentication() {
            let decision = classify_setup_result(Instant::now(), None, false, true);
            assert_eq!(decision.expect_err("mismatch").code, ErrorCode::Authentication);
        }

        #[test]
        fn stall_rejection_is_not_reusable() {
            let error = io::Error::new(
                io::ErrorKind::TimedOut,
                SetupTimeout {
                    reason: TimeoutReason::Stall,
                },
            );
            let failure = failure_from_io(&error);
            assert_eq!(failure.code, ErrorCode::Timeout);
            assert_eq!(failure.setup_cause, Some(SetupFailureCause::Stall));
            assert_eq!(failure.facts.as_ref().and_then(|facts| facts.key_fingerprint.as_deref()), None);
            assert!(!setup_is_reusable(&Err(failure)));
        }

        #[test]
        fn unavailable_setup_kinds_are_distinct_and_generic_io_has_no_cause() {
            for (kind, cause) in [
                (io::ErrorKind::ConnectionRefused, SetupFailureCause::ConnectionRefused),
                (io::ErrorKind::NotFound, SetupFailureCause::NotFound),
                (io::ErrorKind::AddrNotAvailable, SetupFailureCause::AddressNotAvailable),
            ] {
                let failure = failure_from_io(&io::Error::from(kind));
                assert_eq!(failure.code, ErrorCode::Unavailable);
                assert_eq!(failure.setup_cause, Some(cause));
            }
            let generic = failure_from_io(&io::Error::from(io::ErrorKind::Other));
            assert_eq!(generic.code, ErrorCode::Io);
            assert_eq!(generic.setup_cause, None);
        }

        #[test]
        fn the_kinds_a_dropped_setup_reports_are_retried_and_a_cancelled_one_is_not() {
            use crate::git::endpoint::setup_retry::{Phase, Verdict, classify};
            // A server's MaxStartups drop reaches the handshake as a reset or an
            // end of stream, which libssh2 reports as its own error and ssh2 as
            // `Other` (ssh_tests::max_startups). Each such kind is an `Io`.
            for kind in [
                io::ErrorKind::Other,
                io::ErrorKind::ConnectionReset,
                io::ErrorKind::UnexpectedEof,
                io::ErrorKind::BrokenPipe,
            ] {
                let failure = failure_from_io(&io::Error::from(kind));
                assert_eq!(failure.code, ErrorCode::Io, "{kind:?}");
                assert_eq!(classify(&failure, Phase::Setup), Verdict::Retry, "{kind:?}");
            }
            // The transport's own cancellation of a setup job reports
            // `ConnectionAborted`, which is never retried.
            let aborted = failure_from_io(&io::Error::from(io::ErrorKind::ConnectionAborted));
            assert_eq!(aborted.code, ErrorCode::Cancelled);
            assert_eq!(classify(&aborted, Phase::Setup), Verdict::Return);
        }
    }
}
