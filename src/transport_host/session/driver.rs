use super::*;
use crate::git::endpoint::agent_source::AgentSource;
use crate::git::endpoint::ssh_host_key::{HostKeyRefused, WINDOWS_SHELLS};

mod backstop;
pub(in crate::transport_host) mod native_hint;
mod opening;
mod pump;
pub(in crate::transport_host::session) use backstop::Backstop;
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
///
/// The bound covers attempts, not holds (`backstop.rs`): the time an endpoint
/// of this process holds the open, with its allocation clock stopped, is not
/// charged, and the backstop ends only its own stream. The waits between
/// attempts are holds, so the `wait_bound_ms` term is now margin.
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
/// An SSH open's failure, the attempt it ended as `(N, M)` when that is
/// known, and the stall bound the open ran under, in milliseconds (the
/// `--ssh-timeout` it was given; 0 when the stall clock is off). A stall's
/// display names that bound, and the attempt ends the display
/// `(attempt N of M)` (the retry plan's §5; T7).
///
/// The fourth field is how the open ended beyond its failure, which the wire has no field for and the endpoint left in
/// the handoff (`EndNote`): a Down key's members, the local wait that expired.
#[derive(Debug)]
pub(crate) struct SshOpenFailure(
    pub(crate) Failure,
    pub(crate) Option<(u32, u32)>,
    pub(crate) u64,
    pub(crate) Option<setup_retry::Ended>,
);
/// An open the endpoint refused: its failure, and how it ended beyond that when the endpoint left a note for the
/// driver (`EndNote`).
#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) failure: Failure,
    pub(crate) note: Option<setup_retry::EndNote>,
}
impl From<Failure> for Refusal {
    fn from(failure: Failure) -> Self {
        Self {
            failure,
            note: None,
        }
    }
}
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
            let hint = native_hint::native_switch_hint(cli_hint, WINDOWS_SHELLS);
            return Some(format!(
                "SSH authentication needs `git` on PATH: gwz runs `git credential fill` to ask your credential helpers, and {what}. Install or repair git, or {hint}."
            ));
        }
        None
    }
    /// The report of a final `Capacity` failure that says why (§5.3), with
    /// what the endpoint knew beyond the wire.
    fn capacity_report(&self) -> Option<setup_retry::CapacityReport> {
        setup_retry::CapacityReport::of_failure(&self.0)
            .map(|report| report.noted(self.3.as_ref().map(|ended| ended.note)))
    }
    pub(crate) fn model_error(&self, cli_hint: bool) -> Option<crate::model::ModelError> {
        // A member that gave up on a local wait has its own code (operator
        // decision 2026-10-13); a `Capacity` without a detail keeps the code it had.
        if let Some(report) = self.capacity_report() {
            return Some(crate::model::ModelError::new(
                crate::model::ErrorCode::Capacity,
                report.words(self.1),
            ));
        }
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
        // A Down key's member says so around the words of the failure it
        // shares (§5.3).
        if let Some(ended) = self.3.as_ref().filter(|ended| ended.is_down()) {
            let plain = Self(self.0.clone(), self.1, self.2, None);
            return f.write_str(&ended.down_words(plain.to_string()));
        }
        if let Some(report) = self.capacity_report() {
            return f.write_str(&report.words(self.1));
        }
        if let Some(reason) = self.helper_reason(false) {
            f.write_str(&reason)?;
        } else if self.0.code == gwz_transport::protocol::ErrorCode::Timeout {
            let stall = (self.0.setup_cause
                == Some(gwz_transport::protocol::SetupFailureCause::Stall))
            .then(|| setup_retry::stall_seconds(self.2))
            .flatten();
            if let Some(seconds) = stall {
                // The attempt goes inside the brackets that name the flag.
                let unit = setup_retry::seconds_unit(&seconds);
                write!(
                    f,
                    "ssh setup timeout: no progress for {seconds} {unit} (--ssh-timeout {seconds}"
                )?;
                if let Some((attempt, attempts)) = self.1 {
                    write!(f, "; attempt {attempt} of {attempts}")?;
                }
                return f.write_str(")");
            }
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
    /// The `known_hosts` file the endpoint read and where its home came from, which a host-key limit prints.
    known_hosts: Option<KnownHostsFile>,
    /// The key file the open selected, as the user can name it to a command, which a key-file limit prints.
    key_file: Option<String>,
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
        f.write_str(&self.limit.reason(
            &self.host,
            self.port,
            self.known_hosts.as_ref(),
            self.key_file.as_deref(),
        ))
    }
}
impl std::error::Error for SshLimitFailure {}
/// An SSH open whose agent step failed (plan steps 3.2b, 3.3 and 3.6): `SSH_AUTH_SOCK` refused, the selected agent not
/// usable, or Pageant not answering or holding no key. Its display is the whole message, worded from the agent source
/// the runtime chose.
#[derive(Debug)]
pub(crate) struct AgentFailure {
    reason: String,
    class: AgentFailureClass,
}
/// What the failure is, which decides the class it keeps for the callers that tell failures apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentFailureClass {
    /// The agent cannot be used at all: a refusal of the platform, a source that is not there.
    Refused,
    /// The agent did not answer in time.
    TimedOut,
    /// The agent answered and has no key to offer: an authentication failure.
    Authentication,
}
impl AgentFailure {
    pub(crate) fn model_error(&self) -> crate::model::ModelError {
        crate::model::ModelError::new(
            crate::model::ErrorCode::UnsupportedOperation,
            self.reason.clone(),
        )
    }
    /// The typed error to report for the attempt: only a refusal is one. A timeout and an authentication failure keep
    /// the classification they have without words of their own.
    pub(crate) fn reported(&self) -> Option<crate::model::ModelError> {
        (self.class == AgentFailureClass::Refused).then(|| self.model_error())
    }
    fn into_io(self) -> io::Error {
        let kind = match self.class {
            AgentFailureClass::Refused => io::ErrorKind::Unsupported,
            AgentFailureClass::TimedOut => io::ErrorKind::TimedOut,
            AgentFailureClass::Authentication => io::ErrorKind::PermissionDenied,
        };
        io::Error::new(kind, self)
    }
}
impl std::fmt::Display for AgentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for AgentFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        (self.class == AgentFailureClass::Authentication)
            .then_some(&crate::git::endpoint::ssh_remote::AuthenticationRejected as _)
    }
}
/// The words for an open that ended at its agent step before a key was offered, when `agent` has some. The endpoint
/// marks that step by the method its facts name (`ssh_local`); the wire carries no text, so the source supplies it.
fn agent_reason(failure: &Failure, agent: &AgentSource) -> Option<String> {
    use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode};
    let facts = failure.facts.as_ref()?;
    (failure.code == ErrorCode::Unavailable
        && failure.effect == Effect::None
        && facts.method == AuthMethod::SshAgent
        && !facts.credential_offered)
        .then(|| agent.unavailable_reason(failure.setup_cause))
        .flatten()
}
/// An SSH open that the host key did not satisfy (plan step 3.1; design section 14, item 13, U8), worded for the
/// destination and the file the endpoint read. Its source is the rejection, so the remote adapter reports it as the
/// native route reports a host key it does not trust: a failure of the SSH class and not a network one.
#[derive(Debug)]
pub(crate) struct SshTrustFailure {
    host: String,
    port: u16,
    known_hosts: Option<KnownHostsFile>,
    /// What the endpoint's trust check found, when it left a record (decision D12); otherwise the words name both.
    found: Option<HostKeyRefused>,
}
impl std::fmt::Display for SshTrustFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self.found {
            Some(found) => found.words(
                &self.host,
                self.port,
                self.known_hosts.as_ref(),
                WINDOWS_SHELLS,
            ),
            None => HostKeyRefused::reason(
                &self.host,
                self.port,
                self.known_hosts.as_ref(),
                WINDOWS_SHELLS,
            ),
        })
    }
}
impl std::error::Error for SshTrustFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // The marker the remote adapter looks for; what was found rides in it when it is known.
        Some(self.found.as_ref().unwrap_or(&HostKeyRefused::NotListed))
    }
}
/// The key file an open selected, as the user can give it to a command: an absolute path or a `~/` path as it was
/// written, a relative one under the base its request came from (the directory the command was run in). An open that
/// selected no key file names none.
fn key_file_name(identity: &gwz_transport::protocol::Identity) -> Option<String> {
    use gwz_transport::protocol::IdentityMode;
    if identity.mode != IdentityMode::ExplicitKey {
        return None;
    }
    let raw = identity.key_path.as_deref()?;
    let path = std::path::Path::new(raw);
    if raw.starts_with('~') || path.is_absolute() {
        return Some(raw.to_owned());
    }
    Some(match identity.path_base.as_deref() {
        Some(base) => std::path::Path::new(base)
            .join(path)
            .to_string_lossy()
            .into_owned(),
        None => raw.to_owned(),
    })
}
/// The error for an open that failed with `failure` for `host` and `port`,
/// under the stall bound `stall_ms`; `agent` is the source its runtime chose.
fn open_failure_io(
    failure: Failure,
    attempts: Option<(u32, u32)>,
    stall_ms: u64,
    host: String,
    port: u16,
    known_hosts: Option<KnownHostsFile>,
    key_file: Option<String>,
    agent: &AgentSource,
    trust: Option<HostKeyRefused>,
) -> io::Error {
    // Pageant's own failures: a request that outlasts the stall limit (text T6), a Pageant that is gone, and one that
    // holds no key this transport can use.
    if let Some(reason) = agent.pageant_reason(&failure, stall_ms, attempts) {
        let class = match failure.code {
            gwz_transport::protocol::ErrorCode::Timeout => AgentFailureClass::TimedOut,
            gwz_transport::protocol::ErrorCode::Authentication => AgentFailureClass::Authentication,
            _ => AgentFailureClass::Refused,
        };
        return AgentFailure { reason, class }.into_io();
    }
    // The ambient (all-keys) route's agent answered and no key got in. An open that selected a key file is the exact
    // route's, which words it by the key it named.
    if key_file.is_none()
        && let Some(reason) = agent.ambient_reason(&failure, &host, attempts)
    {
        return AgentFailure {
            reason,
            class: AgentFailureClass::Authentication,
        }
        .into_io();
    }
    if let Some(reason) = agent_reason(&failure, agent) {
        return AgentFailure {
            reason,
            class: AgentFailureClass::Refused,
        }
        .into_io();
    }
    match crate::git::endpoint::ssh_limits::SshLimit::of_failure(&failure) {
        Some(limit) => io::Error::new(
            io::ErrorKind::Unsupported,
            SshLimitFailure {
                limit,
                host,
                port,
                known_hosts,
                key_file,
            },
        ),
        None if HostKeyRefused::of_failure(&failure) => io::Error::new(
            io::ErrorKind::PermissionDenied,
            SshTrustFailure {
                host,
                port,
                known_hosts,
                found: trust,
            },
        ),
        None => failure_io(failure, attempts, stall_ms),
    }
}
fn failure_io(failure: Failure, attempts: Option<(u32, u32)>, stall_ms: u64) -> io::Error {
    let kind = match failure.code {
        gwz_transport::protocol::ErrorCode::Authentication => io::ErrorKind::PermissionDenied,
        gwz_transport::protocol::ErrorCode::Timeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, SshOpenFailure(failure, attempts, stall_ms, None))
}
/// Leaves `note`, how the open ended beyond its failure, with the SSH failure `error` holds, if it holds one: the
/// words of a Down key's member and of a local wait read it (§5.3).
fn note_ssh_end(error: &mut io::Error, note: Option<setup_retry::EndNote>, host: &str, port: u16) {
    if let Some(note) = note
        && let Some(failed) = error
            .get_mut()
            .and_then(|inner| inner.downcast_mut::<SshOpenFailure>())
    {
        failed.3 = Some(setup_retry::Ended::new(note, host, port));
    }
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

        fn capacity_failure(report: &crate::git::endpoint::setup_retry::CapacityReport) -> Failure {
            Failure {
                code: ErrorCode::Capacity,
                detail: Some(Box::new(gwz_transport::protocol::FailureDetail {
                    capacity: Some(report.detail()),
                    ..Default::default()
                })),
                ..Default::default()
            }
        }

        #[test]
        fn a_local_setup_wait_is_the_capacity_code_with_the_slot_words_and_no_attempt() {
            use crate::git::endpoint::setup_retry::CapacityReport;
            use crate::git::endpoint::setup_retry::{EndNote, LocalBudget, LocalWait};
            let text = "gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out); retry later";
            // An attempt count that rode along is not shown: the design omits it.
            for attempts in [None, Some((2, 4))] {
                let mut reported = failure_io(capacity_failure(&CapacityReport::Local), attempts, 9_000);
                note_ssh_end(&mut reported, Some(EndNote::LocalWait(LocalWait { budget: LocalBudget::JobBudget, waited_ms: 30_000 })), "git.example.com", 22);
                let failed = reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap();
                let model = failed.model_error(false).expect("a typed error");
                assert_eq!(model.code, crate::model::ErrorCode::Capacity);
                assert_eq!(model.message, text);
                assert_eq!(failed.to_string(), text);
            }
            // The wait that expired is the one named (Code P3-6): the shared reservation is no setup-slot wait.
            let mut reported = failure_io(capacity_failure(&CapacityReport::Local), None, 9_000);
            note_ssh_end(&mut reported, Some(EndNote::LocalWait(LocalWait { budget: LocalBudget::Reservation, waited_ms: 4_000 })), "git.example.com", 22);
            let words = reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().to_string();
            assert!(words.contains("in use for 4 s") && !words.contains("64") && !words.contains("stopped answering"), "{words}");
        }

        #[test]
        fn an_ssh_member_of_a_down_key_says_it_made_no_attempt_and_a_retests_failure_says_it_still_fails() {
            // Code P2-1, SSH: the recorded failure is a stall under `--ssh-timeout 9`.
            use crate::git::endpoint::setup_retry::EndNote;
            let stall = || Failure {
                code: ErrorCode::Timeout,
                setup_cause: Some(gwz_transport::protocol::SetupFailureCause::Stall),
                ..Default::default()
            };
            let said = |note: Option<EndNote>| {
                let mut reported = open_failure_io(stall(), None, 9_000, "github.com".into(), 22, None, None, &AgentSource::None, None);
                note_ssh_end(&mut reported, note, "github.com", 22);
                reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().to_string()
            };
            let plain = "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)";
            assert_eq!(said(None), plain);
            assert_eq!(
                said(Some(EndNote::NotAttempted)),
                format!("not attempted: github.com:22 failed earlier in this run ({plain})")
            );
            assert_eq!(
                said(Some(EndNote::StillFailing)),
                format!("{plain}; still failing after a retry about 30 s later")
            );
            assert!(!said(Some(EndNote::NotAttempted)).contains("(attempt"));
        }

        #[test]
        fn a_capacity_failure_without_a_detail_keeps_its_old_words_and_no_typed_error() {
            let plain = Failure { code: ErrorCode::Capacity, ..Default::default() };
            let reported = failure_io(plain, None, 9_000);
            let failed = reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap();
            assert!(failed.model_error(false).is_none());
            assert_eq!(failed.to_string(), "ssh setup failed: Capacity");
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
            let reported = failure_io(stamped(TimeoutReason::Stall), None, 3_000);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: no progress for 3 seconds (--ssh-timeout 3)"));
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
            let reported = failure_io(stamped(TimeoutReason::Aggregate), None, 1_000);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: aggregate"));
            assert_eq!(reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().0.setup_cause, Some(gwz_transport::protocol::SetupFailureCause::Aggregate));
        }

        #[test]
        fn a_spent_budget_ends_the_display_with_its_attempt() {
            // T7: a stall names the flag and its value, and the attempt follows inside the brackets.
            let reported = failure_io(stamped(TimeoutReason::Stall), Some((4, 4)), 9_000);
            assert_eq!(
                reported.to_string(),
                "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9; attempt 4 of 4)"
            );
            let reported = failure_io(stamped(TimeoutReason::Aggregate), Some((1, 1)), 9_000);
            assert_eq!(reported.to_string(), "ssh setup timeout: aggregate (attempt 1 of 1)");
            // Without an attempt count, the flag alone.
            let reported = failure_io(stamped(TimeoutReason::Stall), None, 9_000);
            assert_eq!(
                reported.to_string(),
                "ssh setup timeout: no progress for 9 seconds (--ssh-timeout 9)"
            );
        }

        #[test]
        fn a_stall_names_the_bound_the_user_set() {
            let reported = failure_io(stamped(TimeoutReason::Stall), Some((2, 4)), 20_000);
            assert_eq!(
                reported.to_string(),
                "ssh setup timeout: no progress for 20 seconds (--ssh-timeout 20; attempt 2 of 4)"
            );
            let reported = failure_io(stamped(TimeoutReason::Stall), None, 1_000);
            assert_eq!(
                reported.to_string(),
                "ssh setup timeout: no progress for 1 second (--ssh-timeout 1)"
            );
            // Only a test fixture sets a bound that is not whole seconds.
            let reported = failure_io(stamped(TimeoutReason::Stall), None, 300);
            assert_eq!(
                reported.to_string(),
                "ssh setup timeout: no progress for 0.3 seconds (--ssh-timeout 0.3)"
            );
        }

        #[test]
        fn no_stall_message_names_a_zero_bound() {
            // `--ssh-timeout 0` turns the stall clock off, so a stall cannot be its doing.
            // Should a failure stamped as a stall reach the display with no bound, it
            // keeps the old label rather than say "no progress for 0 seconds".
            let reported = failure_io(stamped(TimeoutReason::Stall), Some((1, 4)), 0);
            assert_eq!(reported.to_string(), "ssh setup timeout: stall (attempt 1 of 4)");
            let reported = failure_io(stamped(TimeoutReason::Stall), None, 0);
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
                9_000,
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
            // Each limit as the open's facts show it: a host key's before any credential, a key file's form after the
            // key was offered, a passphrase's when the key file was admitted, before any credential.
            for (limit, method, offered) in [
                (SshLimit::HostKeys, AuthMethod::None, false),
                (SshLimit::KeyFile, AuthMethod::SshKey, true),
                (SshLimit::EncryptedKeyFile, AuthMethod::SshKey, false),
            ] {
                let mut failure = limit.failure();
                failure.facts = Some(Facts { method, credential_offered: offered, ..Facts::default() });
                let file = crate::git::endpoint::ssh_home::KnownHostsFile {
                    path: std::path::PathBuf::from("known_hosts_of_the_open"),
                    source: crate::git::endpoint::ssh_home::HomeSource::UserProfile,
                };
                let key_file = Some("keys/id_work".to_owned());
                let reported = open_failure_io(failure, Some((1, 4)), 9_000, "git.example.com".into(), 2222, Some(file.clone()), key_file.clone(), &AgentSource::None, None);
                assert_eq!(reported.kind(), io::ErrorKind::Unsupported, "{limit:?}");
                let words = limit.reason("git.example.com", 2222, Some(&file), key_file.as_deref());
                assert_eq!(reported.to_string(), words, "{limit:?}");
                let failed = reported.get_ref().unwrap().downcast_ref::<SshLimitFailure>().unwrap();
                let model = failed.model_error();
                assert_eq!(model.code, crate::model::ErrorCode::UnsupportedOperation);
                assert_eq!(model.message, words);
            }
            // The passphrase's words name the key file the open selected.
            let mut failure = SshLimit::EncryptedKeyFile.failure();
            failure.facts = Some(Facts { method: AuthMethod::SshKey, ..Facts::default() });
            let reported = open_failure_io(failure, None, 9_000, "git.example.com".into(), 22, None, Some("keys/id_work".into()), &AgentSource::None, None);
            assert!(reported.to_string().contains("keys/id_work"), "{reported}");
            assert!(reported.to_string().contains("protected by a passphrase"), "{reported}");
            // The driver's own refusal of the same code carries no facts and keeps its generic display.
            let own = open_failure_io(limit_free_unsupported(), None, 9_000, "git.example.com".into(), 22, None, None, &AgentSource::None, None);
            assert!(own.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
        }

        /// The key file an open selected is named as the user can give it to a command: an absolute path as it was
        /// written, a relative one under the base the request came from, `~/` as written.
        #[test]
        fn the_selected_key_file_is_named_as_a_command_can_take_it() {
            use gwz_transport::protocol::{Identity, IdentityMode};
            let identity = |mode, key_path: Option<&str>, path_base: Option<&str>| Identity {
                mode,
                key_path: key_path.map(str::to_owned),
                path_base: path_base.map(str::to_owned),
            };
            let absolute = std::env::temp_dir().join("id_work");
            let absolute_text = absolute.to_string_lossy().into_owned();
            let explicit = IdentityMode::ExplicitKey;
            assert_eq!(key_file_name(&identity(explicit, Some(&absolute_text), None)), Some(absolute_text.clone()));
            assert_eq!(key_file_name(&identity(explicit, Some("~/.ssh/id_work"), None)), Some("~/.ssh/id_work".to_owned()));
            let base = std::env::temp_dir();
            assert_eq!(
                key_file_name(&identity(explicit, Some("keys/id_work"), Some(&base.to_string_lossy()))),
                Some(base.join("keys/id_work").to_string_lossy().into_owned())
            );
            // A relative path with no base is what was typed; an open that selected no key file names none.
            assert_eq!(key_file_name(&identity(explicit, Some("id_work"), None)), Some("id_work".to_owned()));
            assert_eq!(key_file_name(&identity(IdentityMode::Ambient, Some("id_work"), None)), None);
            assert_eq!(key_file_name(&identity(explicit, None, None)), None);
        }

        /// The agent step's failure is worded from the agent the runtime chose, which only the host knows (plan step
        /// 3.2b); the wire says no more than that the step failed before a credential was offered.
        #[test]
        fn an_agent_step_failure_is_worded_from_the_source_the_runtime_chose() {
            use crate::git::endpoint::agent_source::{AgentSource, select};
            use gwz_transport::protocol::AuthMethod;
            let AgentSource::Refused(refusal) = select(Some(std::ffi::OsStr::new("/tmp/agent")), || None) else {
                panic!("refused");
            };
            let step = |offered: bool, method: AuthMethod| Failure {
                code: ErrorCode::Unavailable,
                facts: Some(Facts { method, credential_offered: offered, ..Facts::default() }),
                ..Failure::default()
            };
            let source = AgentSource::Refused(refusal.clone());
            let reported = open_failure_io(step(false, AuthMethod::SshAgent), Some((1, 4)), 9_000, "git.example.com".into(), 22, None, None, &source, None);
            assert_eq!(reported.kind(), io::ErrorKind::Unsupported);
            assert_eq!(reported.to_string(), refusal.to_string());
            let failed = reported.get_ref().unwrap().downcast_ref::<AgentFailure>().unwrap();
            let model = failed.model_error();
            assert_eq!(model.code, crate::model::ErrorCode::UnsupportedOperation);
            assert_eq!(model.message, refusal.to_string());
            // A failure that was not the agent step, or that came after a key was offered, keeps its generic words.
            for other in [step(false, AuthMethod::None), step(true, AuthMethod::SshAgent), step(false, AuthMethod::Gh)] {
                let generic = open_failure_io(other, None, 9_000, "git.example.com".into(), 22, None, None, &source, None);
                assert!(generic.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
            }
            // So does a source with no words of its own.
            let generic = open_failure_io(step(false, AuthMethod::SshAgent), None, 9_000, "git.example.com".into(), 22, None, None, &AgentSource::None, None);
            assert!(generic.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
        }

        /// Pageant's own failures keep the class they have without words (plan step 3.6): a timeout stays a timeout, a
        /// list with no key an authentication failure, and a Pageant that is gone a refusal that is typed.
        #[test]
        fn a_pageant_failure_is_worded_from_the_source_and_keeps_its_class() {
            use crate::git::endpoint::agent_pageant::PageantPin;
            use gwz_transport::protocol::{AuthMethod, SetupFailureCause};
            let source = AgentSource::Pageant { sock_ignored: false, pin: PageantPin::numbered(1, 1) };
            let step = |code, cause, offered| Failure {
                code,
                setup_cause: cause,
                facts: Some(Facts { method: AuthMethod::SshAgent, credential_offered: offered, ..Facts::default() }),
                ..Failure::default()
            };
            let host = || "git.example.com".to_owned();
            let timed = open_failure_io(step(ErrorCode::Timeout, Some(SetupFailureCause::Stall), true), Some((2, 4)), 9_000, host(), 22, None, None, &source, None);
            assert_eq!(timed.kind(), io::ErrorKind::TimedOut);
            assert!(timed.to_string().starts_with("Pageant did not answer within 9 seconds (the SSH stall limit, set by --ssh-timeout)."), "{timed}");
            assert!(timed.to_string().ends_with("(attempt 2 of 4)"), "{timed}");
            let failed = timed.get_ref().unwrap().downcast_ref::<AgentFailure>().unwrap();
            assert!(failed.reported().is_none(), "a timeout is not a typed refusal");
            let empty = open_failure_io(step(ErrorCode::Authentication, None, false), None, 9_000, host(), 22, None, None, &source, None);
            assert_eq!(empty.kind(), io::ErrorKind::PermissionDenied);
            assert!(empty.to_string().starts_with("Pageant has no key this transport can use"), "{empty}");
            assert!(empty.get_ref().unwrap().source().is_some_and(|cause| cause.is::<crate::git::endpoint::ssh_remote::AuthenticationRejected>()));
            let gone = open_failure_io(step(ErrorCode::Unavailable, Some(SetupFailureCause::NotFound), true), None, 9_000, host(), 22, None, None, &source, None);
            assert_eq!(gone.kind(), io::ErrorKind::Unsupported);
            let failed = gone.get_ref().unwrap().downcast_ref::<AgentFailure>().unwrap();
            assert_eq!(failed.reported().unwrap().code, crate::model::ErrorCode::UnsupportedOperation);
            // The same failures of another source keep their generic words.
            let generic = open_failure_io(step(ErrorCode::Timeout, Some(SetupFailureCause::Stall), true), None, 9_000, host(), 22, None, None, &AgentSource::None, None);
            assert!(generic.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
        }

        /// The ambient (all-keys) agent route words its own failure: what was offered and the next action for the
        /// agent that answered, and the failure keeps the authentication class the callers tell it by.
        #[test]
        fn an_ambient_agent_that_answered_and_was_not_accepted_is_worded_and_keeps_its_class() {
            use crate::git::endpoint::agent_source::PipeName;
            use gwz_transport::protocol::AuthMethod;
            let step = |offered, authenticated| Failure {
                code: ErrorCode::Authentication,
                facts: Some(Facts { method: AuthMethod::SshAgent, credential_offered: offered, authenticated, ..Facts::default() }),
                ..Failure::default()
            };
            let host = || "git.example.com".to_owned();
            for source in [AgentSource::Path("agent.sock".into()), AgentSource::NamedPipe(PipeName::default_pipe())] {
                let rejected = open_failure_io(step(true, Some(false)), Some((1, 4)), 9_000, host(), 22, None, None, &source, None);
                assert_eq!(rejected.kind(), io::ErrorKind::PermissionDenied);
                assert!(rejected.to_string().starts_with("GWZ offered git.example.com every key the "), "{rejected}");
                assert!(rejected.to_string().ends_with("then run the command again. (attempt 1 of 4)"), "{rejected}");
                assert!(rejected.get_ref().unwrap().source().is_some_and(|cause| cause.is::<crate::git::endpoint::ssh_remote::AuthenticationRejected>()));
                let failed = rejected.get_ref().unwrap().downcast_ref::<AgentFailure>().unwrap();
                assert!(failed.reported().is_none(), "an authentication failure is not a typed refusal");
                let empty = open_failure_io(step(false, None), None, 9_000, host(), 22, None, None, &source, None);
                assert_eq!(empty.kind(), io::ErrorKind::PermissionDenied);
                assert!(empty.to_string().contains("has no key GWZ can offer git.example.com"), "{empty}");
                // An open that selected a key file is the exact route's, whose words are not these.
                let exact = open_failure_io(step(true, Some(false)), None, 9_000, host(), 22, None, Some("keys/id_work.pub".into()), &source, None);
                assert!(exact.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some(), "{exact}");
            }
            // A source with no agent keeps the generic words.
            let generic = open_failure_io(step(true, Some(false)), None, 9_000, host(), 22, None, None, &AgentSource::None, None);
            assert!(generic.get_ref().unwrap().downcast_ref::<SshOpenFailure>().is_some());
        }

        /// A default pipe that is not there is the stopped service, told from the wire's cause alone (plan step 3.3).
        #[test]
        fn a_pipe_that_is_not_there_is_worded_from_the_source_and_the_cause() {
            use crate::git::endpoint::agent_source::{AgentSource, PipeName};
            use gwz_transport::protocol::{AuthMethod, SetupFailureCause};
            let missing = Failure {
                code: ErrorCode::Unavailable,
                setup_cause: Some(SetupFailureCause::NotFound),
                facts: Some(Facts { method: AuthMethod::SshAgent, ..Facts::default() }),
                ..Failure::default()
            };
            let source = AgentSource::NamedPipe(PipeName::default_pipe());
            let reported = open_failure_io(missing, Some((1, 4)), 9_000, "git.example.com".into(), 22, None, None, &source, None);
            let failed = reported.get_ref().unwrap().downcast_ref::<AgentFailure>().unwrap();
            assert!(failed.to_string().starts_with("The OpenSSH Authentication Agent service (ssh-agent) is not running."), "{failed}");
        }

        #[test]
        fn a_refused_host_key_is_worded_with_the_file_and_the_home_it_came_from() {
            use crate::git::endpoint::ssh_home::HomeSource;
            let file = KnownHostsFile {
                path: std::path::PathBuf::from("known_hosts_of_the_open"),
                source: HomeSource::HomeDrivePath,
            };
            let failure = HostKeyRefused::failure();
            let reported = open_failure_io(failure.clone(), Some((1, 4)), 9_000, "git.example.com".into(), 2222, Some(file.clone()), None, &AgentSource::None, None);
            assert_eq!(reported.kind(), io::ErrorKind::PermissionDenied);
            let words = HostKeyRefused::reason("git.example.com", 2222, Some(&file), WINDOWS_SHELLS);
            assert_eq!(reported.to_string(), words);
            assert!(words.contains("known_hosts_of_the_open") && words.contains("HOMEDRIVE and HOMEPATH"), "{words}");
            // The remote adapter finds the rejection under the failure, as it finds an authentication refusal.
            let cause = reported.get_ref().unwrap();
            assert!(cause.is::<SshTrustFailure>());
            assert!(cause.source().is_some_and(|source| source.is::<HostKeyRefused>()));
            // Without a known file the words still stand.
            let bare = open_failure_io(failure, None, 9_000, "h.example".into(), 22, None, None, &AgentSource::None, None);
            assert!(bare.to_string().contains("h.example"));
        }

        #[test]
        fn a_refused_host_key_is_worded_by_what_the_endpoint_found() {
            use crate::git::endpoint::ssh_home::HomeSource;
            let file = KnownHostsFile {
                path: std::path::PathBuf::from("known_hosts_of_the_open"),
                source: HomeSource::Home,
            };
            for found in [HostKeyRefused::NotListed, HostKeyRefused::Changed { line: Some(3) }, HostKeyRefused::Changed { line: None }] {
                let reported = open_failure_io(HostKeyRefused::failure(), None, 9_000, "h.example".into(), 22, Some(file.clone()), None, &AgentSource::None, Some(found));
                assert_eq!(reported.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(reported.to_string(), found.words("h.example", 22, Some(&file), WINDOWS_SHELLS));
                // The remote adapter still finds the rejection, and the finding rides in it.
                let cause = reported.get_ref().unwrap();
                assert!(cause.source().is_some_and(|source| source.downcast_ref::<HostKeyRefused>() == Some(&found)));
            }
        }

        fn limit_free_unsupported() -> Failure {
            Failure { code: ErrorCode::UnsupportedOperation, ..Failure::default() }
        }
    }
}
