use crate::git::endpoint::setup_retry;
use crate::transport_host::helper_failure::{
    self, HELPER_ALLOCATION_PREFIX, HELPER_INTERACTION_PREFIX, MEMBER_URL,
};

/// The machine proxy asked for a sign-in (HTTP 407 to the CONNECT). Only the Windows machine proxy produces this
/// failure, so the text names it; it points to no native option, because the native route refuses a 407 the same way
/// (design §6, "407").
const PROXY_SIGN_IN_UNSUPPORTED: &str = "The proxy in the Windows machine proxy setting (netsh winhttp show proxy) asked for a sign-in before it would connect to the server (HTTP 407), and GWZ does not support proxy sign-in. No credential was sent to the proxy. Ask your administrator for access through the proxy that does not need a sign-in for this server.";

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
        if self.failure.code == ErrorCode::Authentication && status == Some(407) {
            return PROXY_SIGN_IN_UNSUPPORTED.into();
        }
        if helper && self.failure.code == ErrorCode::Unavailable {
            let what = if self.failure.setup_cause == Some(SetupFailureCause::NotFound) {
                "found no `git`"
            } else {
                "could not start the `git` it found"
            };
            let hint = if self.cli_hint {
                "run with --transport native to use libgit2's native transport, as gwz 1.0 did"
            } else if self.windows {
                // Two shells set a variable two ways, and the message cannot know which one the caller is in.
                "set GWZ_TRANSPORT=native (cmd) or $env:GWZ_TRANSPORT = \"native\" (PowerShell) to use libgit2's native transport, as gwz 1.0 did"
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
            if self.windows {
                return "The server asks for a user name and password (Basic) and no git credential helper is configured. Configure one (git config --global credential.helper manager; manager is Git Credential Manager, which comes with Git for Windows) and retry.".into();
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
        reason
    }
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
                    }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None,
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
                let failure = HttpsOpenFailure { failure: Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { http_status: Some(407), ..Default::default() }),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true };
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
                let failure = HttpsOpenFailure { failure: Failure {
                    code: ErrorCode::Timeout, setup_cause: Some(SetupFailureCause::Allocation),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None };
                assert_eq!(failure.reason(), "HTTPS endpoint request failed: Timeout: allocation");
                let malformed = HttpsOpenFailure { failure: Failure {
                    code: ErrorCode::Authentication,
                    detail: Some(Box::new(FailureDetail { helper_cause: Some(HelperFailureCause::MalformedOutput), ..Default::default() })),
                    ..Default::default()
                }, anonymous: None, attempts: None, service: None, helpers_disabled: false, cli_hint: true, windows: false, host: None }.reason();
                assert!(malformed.starts_with("gwz could not use `git credential fill`'s answer: its answer has a malformed or duplicate username/password field. No credential was sent."));
                assert!(malformed.contains("repository settings or conditional includes"));
                assert!(malformed.ends_with("See Troubleshooting: HTTPS Credential Failure."));
            }

            /// A failed open as the Windows policies end it.
            fn windows(failure: Failure, helpers_disabled: bool, host: Option<&str>) -> HttpsOpenFailure {
                HttpsOpenFailure {
                    failure, anonymous: None, attempts: None, service: Some(GitService::UploadPackAdvertisement),
                    helpers_disabled, cli_hint: false, windows: true, host: host.map(str::to_owned),
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

            #[test]
            fn basic_only_with_no_helper_says_how_to_configure_one_t4() {
                let no_helper = Failure {
                    code: ErrorCode::Authentication,
                    facts: Some(Facts { method: AuthMethod::Gh, http_status: Some(401), ..Default::default() }),
                    ..Default::default()
                };
                assert_eq!(
                    windows(no_helper.clone(), false, Some("git.example.test")).reason(),
                    "The server asks for a user name and password (Basic) and no git credential helper is configured. Configure one (git config --global credential.helper manager; manager is Git Credential Manager, which comes with Git for Windows) and retry."
                );
                // Off Windows the existing message stands.
                let mut elsewhere = windows(no_helper, false, None);
                elsewhere.windows = false;
                assert!(elsewhere.reason().starts_with("The server asked for an HTTPS credential, and no credential helper gave one."));
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
        }
    }
}
