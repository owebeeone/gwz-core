use crate::git::endpoint::setup_retry;
use crate::transport_host::helper_failure::{
    self, HELPER_ALLOCATION_PREFIX, HELPER_INTERACTION_PREFIX, MEMBER_URL,
};

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
}
impl HttpsOpenFailure {
    pub(crate) fn model_error(&self) -> Option<crate::model::ModelError> {
        use gwz_transport::protocol::{AuthMethod, ErrorCode};
        let facts = self.failure.facts.as_ref()?;
        let code = if self.helper_timeout() {
            crate::model::ErrorCode::CredentialHelperTimeout
        } else if self.failure.code == ErrorCode::Unavailable && facts.method == AuthMethod::Gh {
            crate::model::ErrorCode::ExternalToolMissing
        } else if self.failure.code == ErrorCode::Authentication {
            if facts.authenticated == Some(false)
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
    pub(crate) fn is_helper_timeout_message(message: &str) -> bool {
        message.starts_with(HELPER_INTERACTION_PREFIX)
            || message.starts_with(HELPER_ALLOCATION_PREFIX)
    }
    pub(crate) fn helper_timeout(&self) -> bool {
        helper_failure::timeout_budget(&self.failure).is_some()
    }
    /// The failure in words: its code, a timeout's origin (stall,
    /// aggregate, interaction or allocation, as on SSH), and the attempt it
    /// ended when that is known.
    pub(crate) fn reason(&self) -> String {
        use gwz_transport::protocol::{ErrorCode, SetupFailureCause};
        if let Some(reason) = helper_failure::timeout_reason(&self.failure) {
            return reason;
        }
        if self.failure.code == ErrorCode::Authentication
            && let Some(detail) = self.failure.detail.as_ref()
            && let Some(cause) = detail.helper_cause
        {
            return format!(
                "gwz could not use `git credential fill`'s answer: {}. No credential was sent. `git config --get-urlmatch credential.helper` with this member's URL {MEMBER_URL} can select a different helper through repository settings or conditional includes. Check the helper chain GWZ uses. See Troubleshooting: HTTPS Credential Failure.",
                helper_cause(cause, detail.pipe_kind.as_deref())
            );
        }
        use gwz_transport::protocol::{AuthMethod, GitService};
        let facts = self.failure.facts.as_ref();
        let helper = facts.is_some_and(|f| f.method == AuthMethod::Gh);
        let status = facts.and_then(|f| f.http_status);
        if helper && self.failure.code == ErrorCode::Unavailable {
            let what = if self.failure.setup_cause == Some(SetupFailureCause::NotFound) {
                "found no `git`"
            } else {
                "could not start the `git` it found"
            };
            let hint = if self.cli_hint {
                "run with --transport native to use libgit2's native transport, as gwz 1.0 did"
            } else {
                "set GWZ_TRANSPORT=native to use libgit2's native transport, as gwz 1.0 did"
            };
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
            return format!(
                "The server asked for an HTTPS credential, and no credential helper gave one. gwz asks only the helpers in your global and system git configuration, not those in a repository's own configuration or under any `includeIf`, whether `gitdir:` or `hasconfig:remote.*.url:`, and it does not prompt. With a sign-in tool, sign in with it, for example `gh auth login`. With a storing helper, run `git ls-remote` with this member's URL {MEMBER_URL}: git asks for the credential and stores it, or shows its own error. With no helper, set one up first. Then retry. See Troubleshooting: HTTPS Credential Failure."
            );
        }
        if helper && status == Some(403) {
            return format!(
                "The server refused this account (HTTP 403) after gwz sent your credential helper's credential. The account may lack access to the repository, a token scope, or single sign-on authorization; `git ls-remote` with this member's URL {MEMBER_URL} shows the server's own explanation. See Troubleshooting: HTTPS Credential Failure."
            );
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
                return format!(
                    "The server asks for HTTPS authentication by a scheme gwz does not answer here, and nor did gwz 1.0{named}: gwz answers Basic, and on Windows NTLM, Negotiate and Digest too. Use git itself for this member, or ask the server's administrator to accept Basic authentication with a token."
                );
            }
            if self.helpers_disabled {
                return "The server asked for an HTTPS credential, and credential helpers are off for this operation: the program running gwz built it with `Git2Backend::without_credential_helpers()`, in gwz-core's Rust API; the gwz CLI and gwz-py never do. Build it with `Git2Backend::new()` to use your helpers.".into();
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
        reason
    }
}

fn helper_cause(cause: gwz_transport::protocol::HelperFailureCause, pipe: Option<&str>) -> String {
    use gwz_transport::protocol::HelperFailureCause::*;
    match cause {
        PipeFailure => format!("the pipe to git failed ({})", pipe.unwrap_or("Other")),
        OutputLimit => "its answer passed 16 KiB".into(),
        ControlCharacter => "the credential holds a control character".into(),
        UsernameColon => "the username holds a colon".into(),
        NotUtf8 => "its answer is not UTF-8 text".into(),
        MissingNewline => "its answer has no final newline".into(),
        MissingField => "a field is missing".into(),
        MalformedOutput => "its answer has a malformed or duplicate username/password field".into(),
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
                let message = |cause, budget| HttpsOpenFailure {
                    failure: Failure {
                        code: ErrorCode::Timeout, effect: Effect::None,
                        setup_cause: Some(cause),
                        detail: Some(Box::new(FailureDetail { helper_budget_ms: Some(budget), ..Default::default() })),
                        ..Default::default()
                    }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true,
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
            fn missing_provenance_keeps_general_timeout_and_fixed_cause_hides_input() {
                let failure = HttpsOpenFailure { failure: Failure {
                    code: ErrorCode::Timeout, setup_cause: Some(SetupFailureCause::Allocation),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true };
                assert_eq!(failure.reason(), "HTTPS endpoint request failed: Timeout: allocation");
                let malformed = HttpsOpenFailure { failure: Failure {
                    code: ErrorCode::Authentication,
                    detail: Some(Box::new(FailureDetail { helper_cause: Some(HelperFailureCause::MalformedOutput), ..Default::default() })),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true }.reason();
                assert!(malformed.starts_with("gwz could not use `git credential fill`'s answer: its answer has a malformed or duplicate username/password field. No credential was sent."));
                assert!(malformed.contains("repository settings or conditional includes"));
                assert!(malformed.ends_with("See Troubleshooting: HTTPS Credential Failure."));
            }
        }
    }
}
