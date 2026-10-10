use super::*;

mod opening;
mod pump;
/// The endpoint messages one pass hands to the mux, at most.
const MAX_HANDOFFS: usize = 64;
/// How long an open or a check without its own allocation deadline waits for
/// the mux to have a stream to spare; an open's default allocation is as long.
const ADMISSION: Duration = Duration::from_secs(30);
/// The messages one pass forwards from one client stream, at most.
const MAX_STREAM_MESSAGES: usize = 8;
/// How long an open waits for its endpoint's answer: each of its
/// `max_retries + 1` setup attempts may use the 120 s interaction allowance,
/// the 30 s aggregate, a stall and the Open's cleanup allowance, and the
/// waits between them come on top (the retry plan's §5 bound). The cleanup
/// allowance counts because the endpoint disposes a failed setup, joining
/// its setup thread, before it reports the failure.
fn open_backstop_ms(io_timeout_ms: u64, max_retries: u32) -> u64 {
    let attempt = OPEN_ATTEMPT_MS
        .saturating_add(io_timeout_ms)
        .saturating_add(OPEN_CLEANUP_MS);
    u64::from(max_retries)
        .saturating_add(1)
        .saturating_mul(attempt)
        .saturating_add(setup_retry::wait_bound_ms(max_retries))
}
const OPEN_ATTEMPT_MS: u64 = 150_000;
/// The cleanup allowance each Open grants its endpoint.
const OPEN_CLEANUP_MS: u64 = 5_000;
fn network_deadlines(io_timeout_ms: u64, connect_timeout_ms: u64, allocation_ms: i64) -> Deadlines {
    Deadlines {
        allocation_ms,
        connect_ms: connect_timeout_ms as i64,
        io_ms: io_timeout_ms as i64,
        interaction_ms: 120000,
        cleanup_ms: OPEN_CLEANUP_MS as i64,
    }
}
/// An SSH open's failure, and the attempt it ended as `(N, M)` when that is
/// known: its display then ends `(attempt N of M)` (the retry plan's §5).
#[derive(Debug)]
pub(crate) struct SshOpenFailure(pub(crate) Failure, pub(crate) Option<(u32, u32)>);
impl SshOpenFailure {
    fn helper_reason(&self, cli_hint: bool) -> Option<String> {
        use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, SetupFailureCause};
        if let Some(reason) = crate::transport_host::helper_failure::timeout_reason(&self.0) {
            return Some(reason);
        }
        if self.0.code == ErrorCode::Unavailable
            && self.0.effect == Effect::None
            && self
                .0
                .facts
                .as_ref()
                .is_some_and(|facts| facts.method == AuthMethod::Gh)
        {
            let what = if self.0.setup_cause == Some(SetupFailureCause::NotFound) {
                "found no `git`"
            } else {
                "could not start the `git` it found"
            };
            let hint = if cli_hint {
                "run with --transport native to use libgit2's native transport, as gwz 1.0 did"
            } else {
                "set GWZ_TRANSPORT=native to use libgit2's native transport, as gwz 1.0 did"
            };
            return Some(format!(
                "SSH authentication needs `git` on PATH: gwz runs `git credential fill` to ask your credential helpers, and {what}. Install or repair git, or {hint}."
            ));
        }
        None
    }
    pub(crate) fn model_error(&self, cli_hint: bool) -> Option<crate::model::ModelError> {
        let mut message = self.helper_reason(cli_hint)?;
        if let Some((attempt, attempts)) = self.1 {
            message.push_str(&format!(" (attempt {attempt} of {attempts})"));
        }
        let code = if self.0.code == gwz_transport::protocol::ErrorCode::Timeout {
            crate::model::ErrorCode::CredentialHelperTimeout
        } else {
            crate::model::ErrorCode::ExternalToolMissing
        };
        Some(crate::model::ModelError::new(code, message))
    }
}
impl std::fmt::Display for SshOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = self.helper_reason(false) {
            f.write_str(&reason)?;
        } else if self.0.code == gwz_transport::protocol::ErrorCode::Timeout {
            let label = setup_retry::timeout_origin(self.0.setup_cause).unwrap_or("unknown");
            write!(f, "ssh setup timeout: {label}")?;
        } else {
            write!(f, "ssh setup failed: {:?}", self.0.code)?;
        }
        if let Some((attempt, attempts)) = self.1 {
            write!(f, " (attempt {attempt} of {attempts})")?;
        }
        Ok(())
    }
}
impl std::error::Error for SshOpenFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if self.0.code == gwz_transport::protocol::ErrorCode::Authentication {
            Some(&crate::git::endpoint::ssh_remote::AuthenticationRejected)
        } else {
            None
        }
    }
}
/// An SSH open that met a limit of the SSH library on this platform (TD5), worded for the destination it was for. Its
/// display is the whole message: the cause, then the fix.
#[derive(Debug)]
pub(crate) struct SshLimitFailure {
    limit: crate::git::endpoint::ssh_limits::SshLimit,
    host: String,
    port: u16,
}
impl SshLimitFailure {
    pub(crate) fn model_error(&self) -> crate::model::ModelError {
        crate::model::ModelError::new(
            crate::model::ErrorCode::UnsupportedOperation,
            self.to_string(),
        )
    }
}
impl std::fmt::Display for SshLimitFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.limit.reason(&self.host, self.port))
    }
}
impl std::error::Error for SshLimitFailure {}
/// The error for an open that failed with `failure` for `host` and `port`.
fn open_failure_io(
    failure: Failure,
    attempts: Option<(u32, u32)>,
    host: String,
    port: u16,
) -> io::Error {
    match crate::git::endpoint::ssh_limits::SshLimit::of_failure(&failure) {
        Some(limit) => io::Error::new(
            io::ErrorKind::Unsupported,
            SshLimitFailure { limit, host, port },
        ),
        None => failure_io(failure, attempts),
    }
}
fn failure_io(failure: Failure, attempts: Option<(u32, u32)>) -> io::Error {
    let kind = match failure.code {
        gwz_transport::protocol::ErrorCode::Authentication => io::ErrorKind::PermissionDenied,
        gwz_transport::protocol::ErrorCode::Timeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, SshOpenFailure(failure, attempts))
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        use crate::git::endpoint::agent_job::{
            Control, ManualClock, TimeoutReason, timeout_reason,
        };
        use gwz_transport::protocol::{Effect, ErrorCode, Facts, Failure};

        fn stamped(reason: TimeoutReason) -> Failure {
            Failure {
                detail: None,
                setup_cause: Some(reason.setup_cause()),
                code: ErrorCode::Timeout,
                effect: Effect::None,
                facts: Some(Facts::default()),
            }
        }

        #[test]
        fn default_ssh_open_splits_stall_and_aggregate() {
            let deadlines = network_deadlines(9_000, 30_000, 30_000);
            assert_eq!(deadlines.io_ms, 9_000);
            assert_eq!(deadlines.connect_ms, 30_000);
        }

        #[test]
        fn default_https_open_splits_stall_and_aggregate() {
            let deadlines = network_deadlines(9_000, 30_000, 30_000);
            assert_eq!(deadlines.io_ms, 9_000);
            assert_eq!(deadlines.connect_ms, 30_000);
        }

        #[test]
        fn the_open_backstop_covers_every_attempt_and_the_waits_between_them() {
            // One attempt: the interaction allowance, the aggregate, a stall,
            // and the cleanup allowance the failed setup's disposal may use
            // before the endpoint reports it.
            assert_eq!(open_backstop_ms(9_000, 0), 164_000);
            // The default four attempts and their waits, at most 7.75 s.
            assert_eq!(open_backstop_ms(9_000, 3), 4 * 164_000 + 7_750);
            assert_eq!(open_backstop_ms(u64::MAX, u32::MAX), u64::MAX);
        }

        #[test]
        fn the_open_backstop_covers_the_full_bound_also_below_a_five_second_stall() {
            // The retry plan's §5 full bound: each attempt's 30 s aggregate,
            // 120 s interaction allowance and 5 s cleanup allowance, and the
            // waits. A stall shorter than the cleanup allowance does not stand
            // in for it.
            for max_retries in [0, 3, 10] {
                let full = u64::from(max_retries + 1) * (30_000 + 120_000 + 5_000)
                    + setup_retry::wait_bound_ms(max_retries);
                for stall in [1_000, 4_999, 9_000] {
                    assert!(
                        open_backstop_ms(stall, max_retries) >= full,
                        "stall {stall} ms, --max-retries {max_retries}"
                    );
                }
            }
        }

        #[test]
        fn disabled_native_timeout_clears_both_open_deadlines() {
            let deadlines = network_deadlines(0, 0, 30_000);
            assert_eq!(deadlines.io_ms, 0);
            assert_eq!(deadlines.connect_ms, 0);
        }

        #[test]
        fn idle_wait_is_a_setup_stall_not_a_peer_failure() {
            let clock = ManualClock::new();
            let start = clock.now();
            let control = Control::scripted(
                Some(start + std::time::Duration::from_secs(10)),
                std::time::Duration::from_secs(3),
                std::time::Duration::from_secs(5),
                clock.clock(),
            );
            control.begin_slice().unwrap();
            clock.advance(std::time::Duration::from_secs(3));
            let error = control.end_slice(false).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
            let reported = failure_io(stamped(TimeoutReason::Stall), None);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: stall"));
            assert_eq!(reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().0.setup_cause, Some(gwz_transport::protocol::SetupFailureCause::Stall));
        }

        #[test]
        fn short_waits_past_the_aggregate_name_that_reason() {
            let clock = ManualClock::new();
            let start = clock.now();
            let control = Control::scripted(
                Some(start + std::time::Duration::from_millis(2_500)),
                std::time::Duration::from_millis(1_000),
                std::time::Duration::from_secs(5),
                clock.clock(),
            );
            for _ in 0..2 {
                control.begin_slice().unwrap();
                clock.advance(std::time::Duration::from_millis(800));
                control.end_slice(true).unwrap();
            }
            control.begin_slice().unwrap();
            clock.advance(std::time::Duration::from_millis(900));
            let error = control.end_slice(true).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
            let reported = failure_io(stamped(TimeoutReason::Aggregate), None);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: aggregate"));
            assert_eq!(reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().0.setup_cause, Some(gwz_transport::protocol::SetupFailureCause::Aggregate));
        }

        #[test]
        fn a_spent_budget_ends_the_display_with_its_attempt() {
            let reported = failure_io(stamped(TimeoutReason::Stall), Some((4, 4)));
            assert_eq!(reported.to_string(), "ssh setup timeout: stall (attempt 4 of 4)");
            let reported = failure_io(stamped(TimeoutReason::Aggregate), Some((1, 1)));
            assert_eq!(reported.to_string(), "ssh setup timeout: aggregate (attempt 1 of 1)");
            // The reason string stays as it was where the attempt is not known.
            let reported = failure_io(stamped(TimeoutReason::Stall), None);
            assert_eq!(reported.to_string(), "ssh setup timeout: stall");
        }

        #[test]
        fn authentication_failure_stays_authentication() {
            let reported = failure_io(
                Failure {
                    detail: None,
                    setup_cause: None,
                    code: ErrorCode::Authentication,
                    effect: Effect::None,
                    facts: None,
                },
                None,
            );
            assert_eq!(reported.kind(), io::ErrorKind::PermissionDenied);
            assert!(
                reported
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<SshOpenFailure>()
                    .is_some()
            );
            assert!(reported.get_ref().unwrap().source().is_some_and(|source| {
                source.is::<crate::git::endpoint::ssh_remote::AuthenticationRejected>()
            }));
        }

        #[test]
        fn an_open_that_met_a_platform_limit_says_what_and_how_to_fix_it_for_its_destination() {
            use crate::git::endpoint::ssh_limits::SshLimit;
            use gwz_transport::protocol::AuthMethod;
            for (limit, method) in [(SshLimit::HostKeys, AuthMethod::None), (SshLimit::KeyFile, AuthMethod::SshKey)] {
                let mut failure = limit.failure();
                failure.facts = Some(Facts { method, ..Facts::default() });
                let reported = open_failure_io(failure, Some((1, 4)), "git.example.com".into(), 2222);
                assert_eq!(reported.kind(), io::ErrorKind::Unsupported, "{limit:?}");
                let words = limit.reason("git.example.com", 2222);
                assert_eq!(reported.to_string(), words, "{limit:?}");
                let failed = reported.get_ref().unwrap().downcast_ref::<SshLimitFailure>().unwrap();
                let model = failed.model_error();
                assert_eq!(model.code, crate::model::ErrorCode::UnsupportedOperation);
                assert_eq!(model.message, words);
            }
            // The driver's own refusal of the same code carries no facts and keeps its generic display.
            let own = open_failure_io(limit_free_unsupported(), None, "git.example.com".into(), 22);
            assert!(own.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
        }

        fn limit_free_unsupported() -> Failure {
            Failure { code: ErrorCode::UnsupportedOperation, ..Failure::default() }
        }
    }
}
