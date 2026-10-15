use crate::git::endpoint::{https_connection::MachineSetting, setup_retry};
use crate::transport_host::helper_failure::{
    self, HELPER_ALLOCATION_PREFIX, HELPER_INTERACTION_PREFIX, MEMBER_URL,
};

/// The machine proxy asked for a sign-in (HTTP 407 to the CONNECT). Only the Windows machine proxy produces this
/// failure, so the text names it; it points to no native option, because the native route refuses a 407 the same way
/// (design §6, "407").
const PROXY_SIGN_IN_UNSUPPORTED: &str = "The proxy in the Windows machine proxy setting (netsh winhttp show proxy) asked for a sign-in before it would connect to the server (HTTP 407), and GWZ does not support proxy sign-in. No credential was sent to the proxy. Ask your administrator for access through the proxy that does not need a sign-in for this server.";

/// Text U2 (design §14 item 13, decision D7): a direct https connection failed while the Windows machine proxy setting
/// names no proxy. The design gave no wording; this is the lane's.
const NO_PROXY_HINT: &str = "The Windows machine proxy setting (netsh winhttp show proxy) says direct access, so GWZ connected to the server directly, and that connection failed. If your network reaches this server only through a proxy, ask your administrator to set one with netsh winhttp set proxy proxy-server=\"host:port\" or, to copy your Internet Options settings, netsh winhttp import proxy source=ie. GWZ reads only that machine setting, not your browser's settings and not HTTPS_PROXY.";

#[derive(Clone, Debug)]
pub(crate) struct HttpsAttemptReceipt {
    pub(crate) failure: gwz_transport::protocol::Failure,
}
#[derive(Debug)]
pub(crate) struct HttpsOpenFailure {
    pub(crate) failure: gwz_transport::protocol::Failure,
    pub(crate) anonymous: Option<HttpsAttemptReceipt>,
    /// The attempt the failure ended, as `(N, M)`, when the failure alone
    /// reports it (the retry plan's §5; `setup_retry::reported_attempt`).
    pub(crate) attempts: Option<(u32, u32)>,
    pub(crate) service: Option<gwz_transport::protocol::GitService>,
    pub(crate) helpers_disabled: bool,
    pub(crate) cli_hint: bool,
    /// The open ran under a Windows policy (the logon session, with or without the credential helper), so
    /// its words are the Windows ones.
    pub(crate) windows: bool,
    /// The remote's host, which a message may name; absent when the failure does not know it.
    pub(crate) host: Option<String>,
    /// The redirect from https to http that ended the open, with the address the server sent (text T5).
    pub(crate) redirect: Option<crate::git::endpoint::https_worker::redirect_log::Refused>,
    /// What the machine proxy setting said (Windows): a refused one is the whole reason for the open's failure
    /// (decision D6), and a direct one is named when a direct connection fails (decision D7, text U2).
    pub(crate) machine: crate::git::endpoint::https_connection::MachineSetting,
    /// How the open ended beyond its failure, which the wire has no field for and the endpoint left in the handoff
    /// (`EndNote`): a Down key's members, a throttled member's refusals, the local wait that expired.
    pub(crate) ended: Option<setup_retry::Ended>,
}
/// What the policy an open ran under says about its words: whether credential helpers were off, and whether
/// it was a Windows policy.
pub(crate) fn policy_words(policy: Option<gwz_transport::protocol::AuthPolicy>) -> (bool, bool) {
    use gwz_transport::protocol::AuthPolicy;
    (
        matches!(
            policy,
            Some(AuthPolicy::Anonymous | AuthPolicy::WindowsDefault)
        ),
        matches!(
            policy,
            Some(AuthPolicy::WindowsConfigured | AuthPolicy::WindowsDefault)
        ),
    )
}
/// The host of an HTTPS remote's URL, for the messages that name it.
pub(crate) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
}
impl HttpsOpenFailure {
    pub(crate) fn model_error(&self) -> Option<crate::model::ModelError> {
        use gwz_transport::protocol::{AuthMethod, ErrorCode};
        // A member that gave up on a throttle, or on a local wait, has its own code
        // (operator decision 2026-10-13); the pool's own limit has no detail and
        // keeps the code it had.
        if self.capacity_report().is_some() {
            return Some(crate::model::ModelError::new(
                crate::model::ErrorCode::Capacity,
                self.reason(),
            ));
        }
        let facts = self.failure.facts.as_ref()?;
        let code = if self.helper_timeout() {
            crate::model::ErrorCode::CredentialHelperTimeout
        } else if self.failure.code == ErrorCode::Unavailable && facts.method == AuthMethod::Gh {
            crate::model::ErrorCode::ExternalToolMissing
        } else if self.failure.code == ErrorCode::Authentication {
            if facts.http_status == Some(407) {
                // A proxy's sign-in demand is not a refusal by the remote.
                crate::model::ErrorCode::GitCommandFailed
            } else if facts.method == AuthMethod::Sspi
                || facts.authenticated == Some(false)
                || self
                    .failure
                    .detail
                    .as_ref()
                    .and_then(|d| d.schemes.as_ref())
                    .is_some_and(|names| names.iter().any(|n| n.eq_ignore_ascii_case("Negotiate")))
            {
                crate::model::ErrorCode::GitCommandFailed
            } else {
                crate::model::ErrorCode::RemoteRejected
            }
        } else if self.failure.code == ErrorCode::RepositoryRefused {
            crate::model::ErrorCode::RemoteRejected
        } else {
            return None;
        };
        Some(crate::model::ModelError::new(code, self.reason()))
    }
    /// The report of a final `Capacity` failure that says why (§5.3), with
    /// what the endpoint knew beyond the wire.
    fn capacity_report(&self) -> Option<setup_retry::CapacityReport> {
        setup_retry::CapacityReport::of_failure(&self.failure)
            .map(|report| report.noted(self.ended.as_ref().map(|ended| ended.note)))
    }
    pub(crate) fn is_helper_timeout_message(message: &str) -> bool {
        message.starts_with(HELPER_INTERACTION_PREFIX)
            || message.starts_with(HELPER_ALLOCATION_PREFIX)
    }
    pub(crate) fn helper_timeout(&self) -> bool {
        helper_failure::timeout_budget(&self.failure).is_some()
    }
    /// The words for a failure of the configured helpers, by cause. Three are about the helpers as a whole, not about
    /// what a helper answered: git had none (`NoHelper`; text T4 under a Windows policy), the one it had gave no
    /// password (`Declined`), and gwz refused to read the configuration that names them (`ConfigurationRefused`).
    fn helper_cause_words(
        &self,
        cause: gwz_transport::protocol::HelperFailureCause,
        pipe: Option<&str>,
    ) -> String {
        use gwz_transport::protocol::HelperFailureCause::*;
        let unusable = |fault: String| {
            format!(
                "gwz could not use `git credential fill`'s answer: {fault}. No credential was sent. `git config --get-urlmatch credential.helper` with this member's URL {MEMBER_URL} can select a different helper through repository settings or conditional includes. Check the helper chain GWZ uses. See Troubleshooting: HTTPS Credential Failure."
            )
        };
        match cause {
            NoHelper if self.windows => NO_HELPER_WINDOWS.into(),
            NoHelper => no_credential_given(),
            Declined => {
                let host = self.host.as_deref().unwrap_or("this server");
                format!(
                    "The credential helper in your git configuration gave no password for {host}: it may have been cancelled, may hold no sign-in for this server, or may have failed. No credential was sent. Check the helper's sign-in: `git ls-remote` with this member's URL {MEMBER_URL} runs the helper where it can ask you or show its own error. Then retry. See Troubleshooting: HTTPS Credential Failure."
                )
            }
            ConfigurationRefused => CONFIGURATION_REFUSED.into(),
            PipeFailure => unusable(format!(
                "the pipe to git failed ({})",
                pipe.unwrap_or("Other")
            )),
            OutputLimit => unusable("its answer passed 16 KiB".into()),
            ControlCharacter => unusable("the credential holds a control character".into()),
            UsernameColon => unusable("the username holds a colon".into()),
            NotUtf8 => unusable("its answer is not UTF-8 text".into()),
            MissingNewline => unusable("its answer has no final newline".into()),
            MissingField => unusable("a field is missing".into()),
            MalformedOutput => {
                unusable("its answer has a malformed or duplicate username/password field".into())
            }
        }
    }
    /// The failure in words (`plain_reason`), put as §5.3 says when it is a
    /// Down key's: `not attempted: <host:port> failed earlier in this run
    /// (<reason>)` for a member that made no attempt, and the reason then
    /// `; still failing after a retry about 30 s later` for a failed retest.
    pub(crate) fn reason(&self) -> String {
        let plain = self.plain_reason();
        match &self.ended {
            Some(ended) if ended.is_down() => ended.down_words(plain),
            _ => plain,
        }
    }
    /// The failure in words: its code, a timeout's origin (stall,
    /// aggregate, interaction or allocation, as on SSH), and the attempt it
    /// ended when that is known.
    fn plain_reason(&self) -> String {
        use gwz_transport::protocol::{ErrorCode, SetupFailureCause};
        if let MachineSetting::Refused(text) = &self.machine {
            // T1 and the other machine-proxy refusals, verbatim: the setting is the whole reason (decision D6).
            return text.to_string();
        }
        if let Some(reason) = helper_failure::timeout_reason(&self.failure) {
            return reason;
        }
        if let Some(report) = self.capacity_report() {
            return report.words(self.attempts);
        }
        if let Some(redirect) = &self.redirect {
            // Text T5. The address in quotes is the server's own text: it is shown, not advised.
            return format!(
                "{} redirected to \"{}\", which is not https. GWZ does not follow a redirect from https to http, because the request would then travel unencrypted. The address in quotes was sent by the server: treat it as untrusted text, and do not use it. Ask the server's administrator to fix the redirect.",
                redirect.from, redirect.to
            );
        }
        if self.failure.code == ErrorCode::Authentication
            && let Some(detail) = self.failure.detail.as_ref()
            && let Some(cause) = detail.helper_cause
        {
            return self.helper_cause_words(cause, detail.pipe_kind.as_deref());
        }
        use gwz_transport::protocol::{AuthMethod, GitService};
        let facts = self.failure.facts.as_ref();
        let helper = facts.is_some_and(|f| f.method == AuthMethod::Gh);
        let status = facts.and_then(|f| f.http_status);
        if self.failure.code == ErrorCode::Authentication && status == Some(407) {
            return PROXY_SIGN_IN_UNSUPPORTED.into();
        }
        if helper && self.failure.code == ErrorCode::Unavailable {
            let what = if self.failure.setup_cause == Some(SetupFailureCause::NotFound) {
                "found no `git`"
            } else {
                "could not start the `git` it found"
            };
            // The one hint of the SSH twin (lane B, 2026-10-11): the flag, else the switch in both Windows shells.
            let hint =
                crate::transport_host::session::native_switch_hint(self.cli_hint, self.windows);
            return format!(
                "HTTPS authentication needs `git` on PATH: gwz runs `git credential fill` to ask your credential helpers, and {what}. Install or repair git, or {hint}."
            );
        }
        if helper && self.failure.code == ErrorCode::Authentication {
            if facts.is_some_and(|f| f.authenticated == Some(false)) {
                return format!(
                    "The server rejected the credential your credential helper gave (HTTP 401), and gwz does not erase or replace it. Renew it where it is kept. With a sign-in tool, sign in again, for example `gh auth refresh` or `gh auth login`. With a stored password or token, run `git ls-remote` with this member's URL {MEMBER_URL}: its first run fails and erases the old credential, and the next asks for the new one and stores it. Then retry. See Troubleshooting: HTTPS Credential Failure."
                );
            }
            return no_credential_given();
        }
        if helper && status == Some(403) {
            return format!(
                "The server refused this account (HTTP 403) after gwz sent your credential helper's credential. The account may lack access to the repository, a token scope, or single sign-on authorization; `git ls-remote` with this member's URL {MEMBER_URL} shows the server's own explanation. See Troubleshooting: HTTPS Credential Failure."
            );
        }
        if status == Some(403)
            && facts.is_some_and(|f| f.method == AuthMethod::Sspi && f.credential_offered)
        {
            // The `gh` path's refusal, for the credential this open sent: the logon session's, not a helper's.
            return format!(
                "The server refused this account (HTTP 403) after gwz sent your Windows sign-in. The account may lack access to the repository; `git ls-remote` with this member's URL {MEMBER_URL} shows the server's own explanation. See Troubleshooting: HTTPS Credential Failure."
            );
        }
        if self.failure.code == ErrorCode::Authentication
            && facts.is_some_and(|f| f.method == AuthMethod::Sspi && f.authenticated == Some(false))
            && let Some(scheme) = facts
                .and_then(|f| f.native.as_ref())
                .and_then(|n| native_scheme_name(n.scheme))
        {
            // The logon identity was refused (T3). Nothing else is tried: the helper is not asked for a
            // Windows sign-in, and the identity is not replayed.
            let host = self.host.as_deref().unwrap_or("The server");
            return format!(
                "{host} rejected your Windows account ({scheme}). GWZ tries nothing else. Ask the server's administrator for access for this account, or use an SSH remote."
            );
        }
        if let Some(words) = self.sign_in_words() {
            return words;
        }
        if self.failure.code == ErrorCode::Authentication && status == Some(401) {
            if self.service.is_some_and(|s| {
                matches!(
                    s,
                    GitService::UploadPackExchange | GitService::ReceivePackExchange
                )
            }) {
                return "The server let this member's discovery through anonymously, then asked for a credential on the push or fetch request itself, which gwz does not answer. Run that push or fetch with git for this member.".into();
            }
            if let Some(schemes) = self
                .failure
                .detail
                .as_ref()
                .and_then(|d| d.schemes.as_ref())
            {
                let named = if schemes.is_empty() {
                    " (the server named no scheme)".into()
                } else {
                    format!(
                        " (the schemes the server named: {})",
                        schemes
                            .iter()
                            .map(|s| format!("\"{s}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                if self.windows && schemes.iter().any(|s| s.eq_ignore_ascii_case("Digest")) {
                    // Digest is answered by neither transport (U3), so no pointer to the native one.
                    return format!(
                        "The server asks for HTTPS authentication by Digest{named}. Neither of GWZ's transports, the default one and --transport native, answers Digest, and gwz 1.0 did not either. Use an SSH remote for this member, or ask the server's administrator to accept Basic authentication with a token, or Negotiate or NTLM."
                    );
                }
                return format!(
                    "The server asks for HTTPS authentication by a scheme gwz does not answer here, and nor did gwz 1.0{named}: gwz answers Basic, and on Windows NTLM and Negotiate too. Use git itself for this member, or ask the server's administrator to accept Basic authentication with a token."
                );
            }
            if self.helpers_disabled {
                return "The server asked for an HTTPS credential, and credential helpers are off for this operation: the program running gwz built it with `Git2Backend::without_credential_helpers()`, in gwz-core's Rust API; neither the gwz CLI nor gwz-py does so. Build it with `Git2Backend::new()` to use your helpers.".into();
            }
        }
        let mut reason = format!("HTTPS endpoint request failed: {:?}", self.failure.code);
        if self.failure.code == gwz_transport::protocol::ErrorCode::Timeout
            && let Some(origin) = setup_retry::timeout_origin(self.failure.setup_cause)
        {
            reason.push_str(": ");
            reason.push_str(origin);
        }
        if let Some((attempt, attempts)) = self.attempts {
            reason.push_str(&format!(" (attempt {attempt} of {attempts})"));
        }
        if self.failed_direct_connection() {
            reason.push(' ');
            reason.push_str(NO_PROXY_HINT);
        }
        reason
    }
    /// The words for a Windows sign-in (SSPI) that ended without a verdict on the logon identity, or `None`: the
    /// sign-in worker that gwz installs was missing, from another build, or not the program gwz installed (it never
    /// began, so no token was made and no status came), and a redirect that answered the exchange.
    fn sign_in_words(&self) -> Option<String> {
        use gwz_transport::protocol::{AuthMethod, ErrorCode, NativeObservation};
        let facts = self.failure.facts.as_ref()?;
        if facts.method != AuthMethod::Sspi {
            return None;
        }
        if self.failure.code == ErrorCode::Authentication
            && let Some(status @ 300..=399) = facts.http_status
        {
            // The exchange ends at a redirect: nothing follows it, and the server's address is not quoted (it is the
            // server's text, and T5 is the place that shows one).
            return Some(format!(
                "The server answered GWZ's Windows sign-in with a redirect (HTTP {status}). GWZ does not follow a redirect once a Windows sign-in has started, so your credential is not carried to another address, and nothing was sent to the new location. Ask the server's administrator to fix the redirect, or for the address that serves the repository directly."
            ));
        }
        let began = facts.credential_offered
            || facts.http_status.is_some()
            || facts
                .native
                .as_ref()
                .is_none_or(|native| native.observation != NativeObservation::NotStarted);
        if began {
            return None;
        }
        let hint = crate::transport_host::session::native_switch_hint(self.cli_hint, self.windows);
        match self.failure.code {
            ErrorCode::Unavailable => Some(format!(
                "GWZ could not use its Windows sign-in worker, the program installed with gwz that answers Negotiate and NTLM: it is missing, or it is from a different build of gwz. No credential was sent. Reinstall gwz so that its parts match, or {hint}."
            )),
            ErrorCode::Protocol => Some(format!(
                "GWZ's Windows sign-in worker did not answer as the one installed with gwz does: the file may have been replaced by another program, or Windows refused to start the sign-in. No credential was sent. Reinstall gwz, or {hint}."
            )),
            _ => None,
        }
    }
    /// Whether this is a direct https connection that failed before any server answered, under a machine setting that
    /// says direct (text U2, decision D7). A server's answer (a status or a method in the facts), a trust failure, a sign-in, a helper's failure
    /// or a local wait for a place or for the user is not one, and neither is a failure under a named proxy or where no machine setting was read.
    fn failed_direct_connection(&self) -> bool {
        use gwz_transport::protocol::{AuthMethod, ErrorCode, SetupFailureCause};
        // An open's facts are present but empty when nothing was exchanged: no status, no method, no verdict.
        let nothing_answered = self.failure.facts.as_ref().is_none_or(|facts| {
            facts.http_status.is_none()
                && facts.method == AuthMethod::None
                && facts.authenticated.is_none()
        });
        // A wait for a place (the route gate, a setup slot) or for the user ended before any connection was made.
        let waited_locally = matches!(
            self.failure.setup_cause,
            Some(SetupFailureCause::Allocation | SetupFailureCause::Interaction)
        );
        self.machine == MachineSetting::Direct
            && nothing_answered
            && !waited_locally
            && matches!(
                self.failure.code,
                ErrorCode::Unavailable | ErrorCode::Io | ErrorCode::Timeout
            )
    }
}

/// Text T4 (design section 14, item 13): `git credential fill` failed because git has no credential helper to ask.
const NO_HELPER_WINDOWS: &str = "The server asks for a user name and password (Basic) and no git credential helper is configured. Configure one (git config --global credential.helper manager; manager is Git Credential Manager, which comes with Git for Windows) and retry.";

/// Refusals of the configuration view (`https_auth::view`) all come to the host as one cause; this names the kinds.
const CONFIGURATION_REFUSED: &str = "gwz did not use your git configuration to find a credential helper, because it refused to read it. It asked no helper and sent no credential. It refuses a configuration that is too large, include.path chains that are too deep or too many, an include it cannot follow (unreadable, not a regular file, or written as `~user/` or `%(prefix)/`), a value too large for the environment of the git it starts, and a `git config` that itself fails. `git config --list --show-origin` shows what git reads: reduce it or fix the include it names, then retry. See Troubleshooting: HTTPS Credential Failure.";

/// The words for a server that wanted a credential when nothing says more than that none came, and for no helper
/// off Windows.
fn no_credential_given() -> String {
    format!(
        "The server asked for an HTTPS credential, and no credential helper gave one. gwz asks only the helpers in your global and system git configuration, not those in a repository's own configuration or under any `includeIf`, whether `gitdir:` or `hasconfig:remote.*.url:`, and it does not prompt. With a sign-in tool, sign in with it, for example `gh auth login`. With a storing helper, run `git ls-remote` with this member's URL {MEMBER_URL}: git asks for the credential and stores it, or shows its own error. With no helper, set one up first. Then retry. See Troubleshooting: HTTPS Credential Failure."
    )
}

/// The scheme a native exchange named, as the server spells it; `None` for a scheme the exchange never answers.
fn native_scheme_name(scheme: gwz_transport::protocol::NativeScheme) -> Option<&'static str> {
    use gwz_transport::protocol::NativeScheme;
    match scheme {
        NativeScheme::Negotiate => Some("Negotiate"),
        NativeScheme::Ntlm => Some("NTLM"),
        NativeScheme::Digest => None,
    }
}

impl std::fmt::Display for HttpsOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self
            .anonymous
            .as_ref()
            .and_then(|r| r.failure.facts.as_ref())
            .and_then(|f| f.http_status)
        {
            write!(f, "anonymous discovery returned HTTP {status}; ")?;
        }
        f.write_str(&self.reason())
    }
}
impl std::error::Error for HttpsOpenFailure {}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use gwz_transport::protocol::*;

            #[test]
            fn helper_specific_timeouts_render_exact_captured_seconds_only() {
                let message = |cause, budget| HttpsOpenFailure { ended: None,
                    failure: Failure {
                        code: ErrorCode::Timeout, effect: Effect::None,
                        setup_cause: Some(cause),
                        detail: Some(Box::new(FailureDetail { helper_budget_ms: Some(budget), ..Default::default() })),
                        ..Default::default()
                    }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None, redirect: None, machine: Default::default(),
                }.reason();
                let interaction = message(SetupFailureCause::Interaction, 1_250);
                assert!(interaction.starts_with("No credential helper answered within 1.25 seconds, a fixed bound"));
                assert!(interaction.ends_with("`->`), then retry."));
                let allocation = message(SetupFailureCause::Allocation, 0);
                assert!(allocation.starts_with("No credential helper could start in the 0 seconds available to wait for resources."));
                assert!(allocation.contains("Other helpers may be busy"));
                assert!(!allocation.contains("all busy"));
                assert!(allocation.ends_with("See Troubleshooting: HTTPS Credential Failure."));
            }

            #[test]
            fn a_proxy_407_names_the_machine_proxy_setting_and_points_to_no_native_option() {
                let failure = HttpsOpenFailure { ended: None, failure: Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { http_status: Some(407), ..Default::default() }),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: true, host: None, redirect: None, machine: Default::default() };
                let reason = failure.reason();
                assert!(reason.contains("Windows machine proxy setting"), "{reason}");
                assert!(reason.contains("does not support proxy sign-in"), "{reason}");
                assert!(reason.contains("No credential was sent to the proxy"), "{reason}");
                for pointer in ["--transport", "GWZ_TRANSPORT", "native"] {
                    assert!(!reason.contains(pointer), "{pointer} in {reason}");
                }
                // The proxy's demand is not a refusal by the remote, and it is not retried as one.
                assert_eq!(failure.model_error().unwrap().code, crate::model::ErrorCode::GitCommandFailed);
            }

            #[test]
            fn missing_provenance_keeps_general_timeout_and_fixed_cause_hides_input() {
                let failure = HttpsOpenFailure { ended: None, failure: Failure {
                    code: ErrorCode::Timeout, setup_cause: Some(SetupFailureCause::Allocation),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None, redirect: None, machine: Default::default() };
                assert_eq!(failure.reason(), "HTTPS endpoint request failed: Timeout: allocation");
                let malformed = HttpsOpenFailure { ended: None, failure: Failure {
                    code: ErrorCode::Authentication,
                    detail: Some(Box::new(FailureDetail { helper_cause: Some(HelperFailureCause::MalformedOutput), ..Default::default() })),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None, redirect: None, machine: Default::default() }.reason();
                assert!(malformed.starts_with("gwz could not use `git credential fill`'s answer: its answer has a malformed or duplicate username/password field. No credential was sent."));
                assert!(malformed.contains("repository settings or conditional includes"));
                assert!(malformed.ends_with("See Troubleshooting: HTTPS Credential Failure."));
            }

            /// A failed open as the Windows policies end it.
            fn windows(failure: Failure, helpers_disabled: bool, host: Option<&str>) -> HttpsOpenFailure {
                HttpsOpenFailure { ended: None,
                    failure, anonymous: None, attempts: None, service: Some(GitService::UploadPackAdvertisement),
                    helpers_disabled, cli_hint: false, windows: true, host: host.map(str::to_owned), redirect: None, machine: Default::default(),
                }
            }
            fn rejected_logon(scheme: NativeScheme) -> Failure {
                Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts {
                        method: AuthMethod::Sspi, credential_offered: true, authenticated: Some(false), http_status: Some(401),
                        native: Some(NativeFacts { scheme, ..Default::default() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }

            #[test]
            fn a_rejected_logon_names_the_host_and_the_scheme_and_the_next_action_t3() {
                for (scheme, name) in [(NativeScheme::Ntlm, "NTLM"), (NativeScheme::Negotiate, "Negotiate")] {
                    let reason = windows(rejected_logon(scheme), false, Some("git.example.test")).reason();
                    assert_eq!(
                        reason,
                        format!("git.example.test rejected your Windows account ({name}). GWZ tries nothing else. Ask the server's administrator for access for this account, or use an SSH remote.")
                    );
                }
                // A host the failure does not know is "The server", never an empty name.
                assert!(windows(rejected_logon(NativeScheme::Ntlm), false, None).reason().starts_with("The server rejected your Windows account (NTLM)."));
            }

            /// A Basic-only server whose `git credential fill` failed for `cause` (none: the failure says no more).
            fn helper_failed(cause: Option<HelperFailureCause>, windows_policy: bool, host: Option<&str>) -> HttpsOpenFailure {
                let failure = Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { method: AuthMethod::Gh, http_status: Some(401), ..Default::default() }),
                    detail: cause.map(|helper_cause| Box::new(FailureDetail { helper_cause: Some(helper_cause), ..Default::default() })),
                    ..Default::default()
                };
                let mut failed = windows(failure, false, host);
                failed.windows = windows_policy;
                failed
            }
            const T4: &str = "The server asks for a user name and password (Basic) and no git credential helper is configured. Configure one (git config --global credential.helper manager; manager is Git Credential Manager, which comes with Git for Windows) and retry.";

            #[test]
            fn basic_only_with_no_helper_says_how_to_configure_one_t4() {
                assert_eq!(helper_failed(Some(HelperFailureCause::NoHelper), true, Some("git.example.test")).reason(), T4);
                // Off Windows the existing message stands.
                let elsewhere = helper_failed(Some(HelperFailureCause::NoHelper), false, None).reason();
                assert!(elsewhere.starts_with("The server asked for an HTTPS credential, and no credential helper gave one."), "{elsewhere}");
            }

            #[test]
            fn t4_is_said_only_when_no_helper_is_configured() {
                // A failure that says no more than "no credential came" does not claim that none is configured.
                let unknown = helper_failed(None, true, None).reason();
                assert!(unknown.starts_with("The server asked for an HTTPS credential, and no credential helper gave one."), "{unknown}");
                for cause in [HelperFailureCause::Declined, HelperFailureCause::ConfigurationRefused] {
                    for windows_policy in [true, false] {
                        let said = helper_failed(Some(cause), windows_policy, None).reason();
                        assert!(!said.contains("no git credential helper is configured") && !said.contains("credential.helper manager"), "{cause:?}: {said}");
                    }
                }
            }

            #[test]
            fn a_helper_that_gave_no_password_says_so_and_names_the_host() {
                for windows_policy in [true, false] {
                    let said = helper_failed(Some(HelperFailureCause::Declined), windows_policy, Some("git.example.test")).reason();
                    assert!(
                        said.starts_with("The credential helper in your git configuration gave no password for git.example.test: it may have been cancelled, may hold no sign-in for this server, or may have failed. No credential was sent."),
                        "{said}"
                    );
                    assert!(said.contains("Check the helper's sign-in"), "{said}");
                    assert!(said.contains("`git ls-remote` with this member's URL"), "{said}");
                    assert!(said.ends_with("See Troubleshooting: HTTPS Credential Failure."), "{said}");
                }
                // A host the failure does not know is "this server", never an empty name.
                assert!(helper_failed(Some(HelperFailureCause::Declined), false, None).reason().contains("gave no password for this server:"));
            }

            #[test]
            fn a_refused_configuration_names_the_reasons_gwz_refuses_one() {
                for windows_policy in [true, false] {
                    let said = helper_failed(Some(HelperFailureCause::ConfigurationRefused), windows_policy, Some("git.example.test")).reason();
                    assert!(
                        said.starts_with("gwz did not use your git configuration to find a credential helper, because it refused to read it. It asked no helper and sent no credential."),
                        "{said}"
                    );
                    for reason in ["too large", "too deep", "an include it cannot follow", "`git config` that itself fails"] {
                        assert!(said.contains(reason), "{reason} in {said}");
                    }
                    assert!(said.contains("`git config --list --show-origin`"), "{said}");
                    assert!(said.ends_with("See Troubleshooting: HTTPS Credential Failure."), "{said}");
                    // It is a refusal of the configuration, not a verdict on the helper's answer.
                    assert!(!said.contains("could not use `git credential fill`'s answer"), "{said}");
                }
            }

            #[test]
            fn a_helper_credential_the_server_rejected_keeps_its_own_message_on_windows() {
                let rejected = Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { method: AuthMethod::Gh, authenticated: Some(false), http_status: Some(401), ..Default::default() }),
                    ..Default::default()
                };
                assert!(windows(rejected, false, None).reason().starts_with("The server rejected the credential your credential helper gave (HTTP 401)"));
            }

            fn schemes(names: &[&str]) -> Failure {
                Failure {
                    code: ErrorCode::Authentication,
                    detail: Some(Box::new(FailureDetail { schemes: Some(names.iter().map(|name| (*name).to_owned()).collect()), ..Default::default() })),
                    facts: Some(Facts { http_status: Some(401), ..Default::default() }),
                    ..Default::default()
                }
            }

            #[test]
            fn digest_only_is_refused_naming_digest_with_no_pointer_to_the_native_transport() {
                let reason = windows(schemes(&["Digest"]), false, Some("git.example.test")).reason();
                assert!(reason.contains("Digest"), "{reason}");
                assert!(reason.contains("Neither of GWZ's transports"), "{reason}");
                assert!(reason.contains("SSH"), "{reason}");
                // The native transport is named only to say that it does not answer Digest either.
                assert!(!reason.contains("--transport native to") && !reason.contains("use --transport native") && !reason.contains("GWZ_TRANSPORT"), "{reason}");
            }

            #[test]
            fn the_unsupported_scheme_message_no_longer_says_windows_answers_digest() {
                for windows_policy in [false, true] {
                    let mut failure = windows(schemes(&["Bearer"]), false, None);
                    failure.windows = windows_policy;
                    let reason = failure.reason();
                    assert!(reason.contains("the schemes the server named: \"Bearer\""), "{reason}");
                    assert!(reason.contains("gwz answers Basic, and on Windows NTLM and Negotiate too."), "{reason}");
                    assert!(!reason.contains("Digest too") && !reason.contains("and Digest"), "{reason}");
                }
            }

            #[test]
            fn credential_helpers_off_no_longer_blames_windows_qualification_builds() {
                let basic_only = Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { http_status: Some(401), ..Default::default() }),
                    ..Default::default()
                };
                let reason = windows(basic_only, true, None).reason();
                assert!(reason.contains("credential helpers are off for this operation"), "{reason}");
                assert!(!reason.contains("qualification"), "{reason}");
                assert!(reason.contains("without_credential_helpers"), "{reason}");
            }

            #[test]
            fn a_redirect_from_https_to_http_is_refused_with_text_t5_verbatim() {
                use crate::git::endpoint::https_worker::redirect_log::Refused;
                let mut refused = windows(
                    Failure { code: ErrorCode::InvalidRequest, ..Default::default() },
                    false,
                    Some("git.example.test"),
                );
                refused.redirect = Some(Refused {
                    from: "https://git.example.test/team/repo".into(),
                    to: "http://elsewhere.example:8080/team/repo/info/refs?service=git-upload-pack".into(),
                });
                for windows_policy in [true, false] {
                    refused.windows = windows_policy;
                    assert_eq!(
                        refused.reason(),
                        "https://git.example.test/team/repo redirected to \"http://elsewhere.example:8080/team/repo/info/refs?service=git-upload-pack\", which is not https. GWZ does not follow a redirect from https to http, because the request would then travel unencrypted. The address in quotes was sent by the server: treat it as untrusted text, and do not use it. Ask the server's administrator to fix the redirect."
                    );
                }
                // Neither the server's address nor advice about it appears outside the quotes.
                let reason = refused.reason();
                let outside = reason.replace("http://elsewhere.example:8080/team/repo/info/refs?service=git-upload-pack", "");
                for advice in ["set the remote", "use the http", "http://"] {
                    assert!(!outside.contains(advice), "{advice} in {outside}");
                }
                // Without a refusal on record the failure is worded as it always was.
                refused.redirect = None;
                assert_eq!(refused.reason(), "HTTPS endpoint request failed: InvalidRequest");
            }

            #[test]
            fn the_missing_git_hint_shows_the_cmd_and_powershell_forms_on_windows() {
                let missing = Failure {
                    code: ErrorCode::Unavailable,
                    setup_cause: Some(SetupFailureCause::NotFound),
                    facts: Some(Facts { method: AuthMethod::Gh, ..Default::default() }),
                    ..Default::default()
                };
                let reason = windows(missing.clone(), false, None).reason();
                assert!(reason.contains("set GWZ_TRANSPORT=native"), "{reason}");
                assert!(reason.contains("$env:GWZ_TRANSPORT = \"native\""), "{reason}");
                let mut elsewhere = windows(missing, false, None);
                elsewhere.windows = false;
                assert!(!elsewhere.reason().contains("$env:"), "{}", elsewhere.reason());
            }

            const DIRECT_WORDS: &str = "says direct access, so GWZ connected to the server directly";

            fn connect_failure(code: ErrorCode, cause: Option<SetupFailureCause>) -> Failure {
                Failure { code, setup_cause: cause, ..Default::default() }
            }

            #[test]
            fn a_failed_direct_connection_says_winhttp_has_no_proxy_u2() {
                use crate::git::endpoint::https_connection::MachineSetting;
                for failure in [
                    connect_failure(ErrorCode::Unavailable, Some(SetupFailureCause::ConnectionRefused)),
                    connect_failure(ErrorCode::Unavailable, Some(SetupFailureCause::NotFound)),
                    connect_failure(ErrorCode::Unavailable, Some(SetupFailureCause::AddressNotAvailable)),
                    connect_failure(ErrorCode::Io, None),
                    connect_failure(ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)),
                ] {
                    let mut open = windows(failure.clone(), false, None);
                    open.machine = MachineSetting::Direct;
                    let reason = open.reason();
                    assert!(reason.starts_with("HTTPS endpoint request failed: "), "{reason}");
                    assert!(reason.contains(DIRECT_WORDS), "{reason}");
                    assert!(reason.contains("netsh winhttp show proxy"), "{reason}");
                    assert!(reason.contains("netsh winhttp import proxy source=ie"), "{reason}");
                    // Only a direct machine setting says so: a named proxy, a refusal's own text and no machine
                    // setting at all (every Unix build) never do.
                    for other in [MachineSetting::Unread, MachineSetting::Named] {
                        open.machine = other;
                        assert!(!open.reason().contains(DIRECT_WORDS), "{}", open.reason());
                    }
                }
            }

            #[test]
            fn the_no_proxy_hint_is_for_a_connection_that_failed_and_for_nothing_else() {
                use crate::git::endpoint::https_connection::MachineSetting;
                let mut cases = vec![
                    ("a trust failure", connect_failure(ErrorCode::Trust, None)),
                    ("a protocol failure", connect_failure(ErrorCode::Protocol, None)),
                    ("a cancellation", connect_failure(ErrorCode::Cancelled, None)),
                    ("a 401", Failure {
                        code: ErrorCode::Authentication,
                        facts: Some(Facts { http_status: Some(401), ..Default::default() }),
                        ..Default::default()
                    }),
                    ("a proxy's 407", Failure {
                        code: ErrorCode::Authentication,
                        facts: Some(Facts { http_status: Some(407), ..Default::default() }),
                        ..Default::default()
                    }),
                    ("a server's answer", Failure {
                        code: ErrorCode::Io,
                        facts: Some(Facts { http_status: Some(502), ..Default::default() }),
                        ..Default::default()
                    }),
                    ("missing git", Failure {
                        code: ErrorCode::Unavailable,
                        setup_cause: Some(SetupFailureCause::NotFound),
                        facts: Some(Facts { method: AuthMethod::Gh, ..Default::default() }),
                        ..Default::default()
                    }),
                ];
                cases.push(("a helper's timeout", Failure {
                    code: ErrorCode::Timeout,
                    setup_cause: Some(SetupFailureCause::Interaction),
                    detail: Some(Box::new(FailureDetail { helper_budget_ms: Some(1_000), ..Default::default() })),
                    ..Default::default()
                }));
                for (name, failure) in cases {
                    let mut open = windows(failure, false, None);
                    open.machine = MachineSetting::Direct;
                    assert!(!open.reason().contains(DIRECT_WORDS), "{name}: {}", open.reason());
                }
            }

            /// A failure of a Windows sign-in that never began: the SSPI facts a native open leaves before its first step.
            fn sign_in_not_started(code: ErrorCode) -> Failure {
                Failure {
                    code,
                    facts: Some(Facts {
                        method: AuthMethod::Sspi,
                        native: Some(NativeFacts { scheme: NativeScheme::Negotiate, ..Default::default() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            }

            #[test]
            fn a_missing_or_mismatched_sign_in_worker_is_named_with_the_remedy() {
                // Code review P3-11: both gave "HTTPS endpoint request failed: Unavailable" or "Protocol".
                let missing = windows(sign_in_not_started(ErrorCode::Unavailable), false, None).reason();
                assert!(missing.starts_with("GWZ could not use its Windows sign-in worker"), "{missing}");
                assert!(missing.contains("missing"), "{missing}");
                assert!(missing.contains("different build of gwz"), "{missing}");
                assert!(missing.contains("No credential was sent."), "{missing}");
                assert!(missing.contains("Reinstall gwz"), "{missing}");
                assert!(missing.contains("--transport native") || missing.contains("GWZ_TRANSPORT=native"), "{missing}");
                // The flag where the caller has one, the environment switch in both Windows shells where it has not.
                let mut env = windows(sign_in_not_started(ErrorCode::Unavailable), false, None);
                env.cli_hint = false;
                let env = env.reason();
                assert!(env.contains("set GWZ_TRANSPORT=native (cmd)") && env.contains("$env:GWZ_TRANSPORT"), "{env}");
                let replaced = windows(sign_in_not_started(ErrorCode::Protocol), false, None).reason();
                assert!(replaced.starts_with("GWZ's Windows sign-in worker did not answer as the one installed with gwz does"), "{replaced}");
                assert!(replaced.contains("replaced by another program"), "{replaced}");
                assert!(replaced.contains("Reinstall gwz"), "{replaced}");
                assert_ne!(missing, replaced);
                // Once a token has gone, or a status came, the failure is not a worker's: the exchange's own words stand.
                let mut begun = sign_in_not_started(ErrorCode::Protocol);
                begun.facts.as_mut().unwrap().credential_offered = true;
                assert_eq!(windows(begun, false, None).reason(), "HTTPS endpoint request failed: Protocol");
                let mut answered = sign_in_not_started(ErrorCode::Unavailable);
                answered.facts.as_mut().unwrap().http_status = Some(401);
                assert_eq!(windows(answered, false, None).reason(), "HTTPS endpoint request failed: Unavailable");
            }

            #[test]
            fn a_redirect_during_the_sign_in_exchange_has_its_own_words() {
                // Code review P3-11: it was "HTTPS endpoint request failed: Authentication".
                let mut redirected = rejected_logon(NativeScheme::Ntlm);
                {
                    let facts = redirected.facts.as_mut().unwrap();
                    facts.authenticated = None;
                    facts.http_status = Some(302);
                }
                let said = windows(redirected, false, Some("git.example.test")).reason();
                assert!(said.starts_with("The server answered GWZ's Windows sign-in with a redirect (HTTP 302)."), "{said}");
                assert!(said.contains("does not follow a redirect once a Windows sign-in has started"), "{said}");
                assert!(said.contains("nothing was sent to the new location"), "{said}");
                assert!(said.contains("Ask the server's administrator"), "{said}");
                // The server's address is untrusted text and is not quoted; the logon identity was not refused either.
                assert!(!said.contains("rejected your Windows account"), "{said}");
            }

            #[test]
            fn a_local_wait_that_ended_an_open_is_not_a_failed_direct_connection() {
                use crate::git::endpoint::https_connection::MachineSetting;
                // Code review P3-3: no connection was made, so the words must not say one failed. The allocation
                // timeout is what an open that waited for its route gate or a setup place ends with.
                let local = [
                    ("an allocation timeout", crate::git::endpoint::setup_retry::allocation_timeout()),
                    ("an interaction timeout", connect_failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction))),
                ];
                for (name, failure) in local {
                    let mut open = windows(failure, false, None);
                    open.machine = MachineSetting::Direct;
                    let reason = open.reason();
                    assert!(!reason.contains(DIRECT_WORDS), "{name}: {reason}");
                    assert!(!reason.contains("netsh winhttp"), "{name}: {reason}");
                }
                // A timeout that ended a connection's setup still carries the hint.
                let mut open = windows(connect_failure(ErrorCode::Timeout, Some(SetupFailureCause::Stall)), false, None);
                open.machine = MachineSetting::Direct;
                assert!(open.reason().contains(DIRECT_WORDS), "{}", open.reason());
            }

            #[test]
            fn a_refused_machine_setting_is_the_whole_reason_with_its_text_verbatim() {
                use crate::git::endpoint::https_connection::MachineSetting;
                let text = "The Windows machine proxy setting has no entry for https: \"http=proxy:8080\". T1";
                let mut open = windows(connect_failure(ErrorCode::UnsupportedOperation, None), false, None);
                open.machine = MachineSetting::Refused(text.into());
                assert_eq!(open.reason(), text);
            }

            /// A final failure with the `capacity` detail of `report`.
            fn capacity_failure(report: &crate::git::endpoint::setup_retry::CapacityReport, attempt: Option<(i64, i64)>) -> Failure {
                Failure {
                    code: ErrorCode::Capacity,
                    detail: Some(Box::new(FailureDetail {
                        capacity: Some(report.detail()),
                        retry_attempt: attempt.map(|(attempt, attempts)| RetryAttempt { attempt, attempts }),
                        ..Default::default()
                    })),
                    ..Default::default()
                }
            }

            fn throttled(retry_after_ms: Option<u64>) -> crate::git::endpoint::setup_retry::CapacityReport {
                use crate::git::endpoint::setup_retry::{CapacityReport, LastSignal, ServerCapacity};
                CapacityReport::Server(ServerCapacity {
                    site: Some(gwz_transport::pool::Site { scheme: Scheme::Https, host: "github.com".into(), port: 443 }),
                    limit_applied: Some(8),
                    max_per_host: 32,
                    elapsed_ms: 21_000,
                    last_signal: Some(LastSignal::Http429),
                    retry_after_ms,
                    refusals: None,
                })
            }

            /// What the endpoint leaves beside a failure it ends (`EndNote`), as the open's failure holds it.
            fn ended(note: crate::git::endpoint::setup_retry::EndNote) -> Option<crate::git::endpoint::setup_retry::Ended> {
                Some(crate::git::endpoint::setup_retry::Ended::new(note, "github.com", 443))
            }

            #[test]
            fn a_throttled_member_is_the_capacity_code_with_the_throttle_words_and_its_attempt() {
                use crate::git::endpoint::setup_retry::EndNote;
                let failure = capacity_failure(&throttled(None), Some((4, 4)));
                let open = HttpsOpenFailure { ended: ended(EndNote::Refused { count: 4 }),
                    attempts: crate::git::endpoint::setup_retry::reported_attempt(&failure),
                    ..windows(failure, false, None)
                };
                let error = open.model_error().unwrap();
                assert_eq!(error.code, crate::model::ErrorCode::Capacity);
                assert_eq!(
                    error.message,
                    "github.com:443 kept refusing connections (using 8 at a time, ceiling 32 from --max-per-host); gave up after 21 s; retry later, or lower --max-per-host (attempt 4 of 4)"
                );
                assert_eq!(open.reason(), error.message);
            }

            #[test]
            fn one_refusal_among_resets_says_the_last_attempt_was_refused_and_not_that_the_host_kept_refusing() {
                // The 5.6 decision: reset, reset, reset, 429. The wire carries no refusal count; the endpoint's note does.
                use crate::git::endpoint::setup_retry::EndNote;
                let say = |note: Option<EndNote>| {
                    let failure = capacity_failure(&throttled(None), Some((4, 4)));
                    HttpsOpenFailure { ended: note.and_then(|note| ended(note)),
                        attempts: crate::git::endpoint::setup_retry::reported_attempt(&failure),
                        ..windows(failure, false, None)
                    }
                    .reason()
                };
                let last = "github.com:443 refused the last attempt (using 8 at a time, ceiling 32 from --max-per-host); gave up after 21 s; retry later, or lower --max-per-host (attempt 4 of 4)";
                assert_eq!(say(Some(EndNote::Refused { count: 1 })), last);
                assert_eq!(say(None), last, "a count nobody left says only what is certain");
                let kept = say(Some(EndNote::Refused { count: 2 }));
                assert!(kept.starts_with("github.com:443 kept refusing connections ("), "{kept}");
            }

            #[test]
            fn a_member_of_a_down_key_that_made_no_attempt_says_so_and_names_the_failure_and_the_site() {
                // Code P2-1: the recorded failure is a stall; the member shares it with no attempt count.
                use crate::git::endpoint::setup_retry::EndNote;
                let stall = Failure {
                    code: ErrorCode::Timeout,
                    setup_cause: Some(SetupFailureCause::Stall),
                    ..Default::default()
                };
                let plain = HttpsOpenFailure { ended: None, ..windows(stall.clone(), false, None) };
                let down = HttpsOpenFailure { ended: ended(EndNote::NotAttempted), ..windows(stall.clone(), false, None) };
                assert_eq!(plain.reason(), "HTTPS endpoint request failed: Timeout: stall");
                assert_eq!(
                    down.reason(),
                    "not attempted: github.com:443 failed earlier in this run (HTTPS endpoint request failed: Timeout: stall)"
                );
                assert!(!down.reason().contains("(attempt"));
                // The retest's own failure, and the members parked on it.
                let retest = HttpsOpenFailure { ended: ended(EndNote::StillFailing), ..windows(stall, false, None) };
                assert_eq!(
                    retest.reason(),
                    "HTTPS endpoint request failed: Timeout: stall; still failing after a retry about 30 s later"
                );
            }

            #[test]
            fn a_long_retry_after_is_the_capacity_code_with_its_own_words() {
                let failure = capacity_failure(&throttled(Some(120_000)), Some((1, 4)));
                let open = HttpsOpenFailure { ended: None,
                    attempts: crate::git::endpoint::setup_retry::reported_attempt(&failure),
                    ..windows(failure, false, None)
                };
                let error = open.model_error().unwrap();
                assert_eq!(error.code, crate::model::ErrorCode::Capacity);
                assert_eq!(
                    error.message,
                    "github.com:443 asked to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min (attempt 1 of 4)"
                );
            }

            #[test]
            fn a_local_wait_is_the_capacity_code_with_the_slot_words_and_never_an_attempt() {
                use crate::git::endpoint::setup_retry::CapacityReport;
                // Even a count that rode along is not shown: the design omits it.
                use crate::git::endpoint::setup_retry::{EndNote, LocalBudget, LocalWait};
                let failure = capacity_failure(&CapacityReport::Local, None);
                let waited = |budget, waited_ms| ended(EndNote::LocalWait(LocalWait { budget, waited_ms }));
                let open = HttpsOpenFailure { ended: waited(LocalBudget::JobBudget, 30_000), attempts: Some((2, 4)), ..windows(failure.clone(), false, None) };
                let error = open.model_error().unwrap();
                assert_eq!(error.code, crate::model::ErrorCode::Capacity);
                assert_eq!(
                    error.message,
                    "gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out); retry later"
                );
                assert!(!error.message.contains("attempt"));
                // The wait that expired is the one named (Code P3-6): a reservation wait is no setup-slot wait.
                let reservation = HttpsOpenFailure { ended: waited(LocalBudget::Reservation, 7_000), ..windows(failure.clone(), false, None) };
                let text = reservation.model_error().unwrap().message;
                assert!(text.contains("in use for 7 s") && !text.contains("64") && !text.contains("stopped answering"), "{text}");
                let slots = HttpsOpenFailure { ended: waited(LocalBudget::SetupSlots, 30_000), ..windows(failure, false, None) };
                let text = slots.model_error().unwrap().message;
                assert!(text.contains("8 slots") && !text.contains("64") && !text.contains("stopped answering"), "{text}");
            }

            #[test]
            fn a_capacity_failure_without_a_detail_keeps_the_words_it_had() {
                // The pool's own refusal (`Error::Capacity`) is not a throttle and says nothing more.
                let open = windows(Failure { code: ErrorCode::Capacity, ..Default::default() }, false, None);
                assert!(open.model_error().is_none());
                assert_eq!(open.reason(), "HTTPS endpoint request failed: Capacity");
                // A detail that cannot be worded is no report either.
                let broken = Failure {
                    code: ErrorCode::Capacity,
                    detail: Some(Box::new(FailureDetail {
                        capacity: Some(CapacityDetail { cause: CapacityCause::ServerThrottle, ..Default::default() }),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                let open = windows(broken, false, None);
                assert!(open.model_error().is_none());
                assert_eq!(open.reason(), "HTTPS endpoint request failed: Capacity");
            }
        }
    }
}
