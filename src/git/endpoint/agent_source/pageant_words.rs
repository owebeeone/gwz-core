//! What the user is told when Pageant, the agent a runtime selected, fails in the agent step (plan step 3.6; the
//! design's §14 item 13, text T6, and §5). The setup's failure crosses the wire as a code, a cause and the facts of the
//! step, with no text, so the host words it from the source its runtime chose, as for the pipes ([`super::words`]).
//!
//! None of these says that another agent was tried, because none is: a Pageant that fails never falls back to the
//! OpenSSH agent or to a key file.
use super::AgentSource;
use crate::git::endpoint::setup_retry::{seconds_unit, stall_seconds};
use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, Failure, SetupFailureCause};

/// The text T6 with `{seconds}` and `{unit}` filled in: the attempt ended at the stall limit while Pageant's request
/// was out, which is `--ssh-timeout`'s bound.
fn stall_words(seconds: &str) -> String {
    let unit = seconds_unit(seconds);
    format!(
        "Pageant did not answer within {seconds} {unit} (the SSH stall limit, set by --ssh-timeout). Pageant may \
         still be waiting for you to confirm the key or to enter its passphrase. Answer it and run the command \
         again, raise --ssh-timeout (up to 30 seconds; 0 removes the limit), or use --transport native, which waits \
         for Pageant as gwz 1.0 did."
    )
}

/// The attempt ended at the 30-second setup budget, not the stall limit. Raising `--ssh-timeout` cannot move that
/// budget; only `--ssh-timeout 0` removes it.
const BUDGET_WORDS: &str = "Pageant did not answer within 30 seconds, the longest GWZ spends setting up one connection, \
                            and raising --ssh-timeout does not lengthen that. Pageant may still be waiting for you to \
                            confirm the key or to enter its passphrase. Answer it sooner and run the command again, \
                            use --ssh-timeout 0 (which removes the limit), or use --transport native, which waits for \
                            Pageant as gwz 1.0 did.";

/// The request ended at the longest GWZ waits for one Pageant request, which `--ssh-timeout 0` leaves in place.
fn cap_words() -> String {
    let seconds = super::super::agent_pageant::REQUEST_CAP.as_secs();
    format!(
        "Pageant did not answer within {seconds} seconds, the longest GWZ waits for it even with --ssh-timeout 0. \
         Pageant may still be waiting for you to confirm the key or to enter its passphrase. Answer it and run the \
         command again, or use --transport native, which waits for Pageant as gwz 1.0 did."
    )
}

/// Pageant was there when the runtime started and is not the same program now. GWZ asks no other agent.
pub(super) const GONE_WORDS: &str = "Pageant was running when GWZ started, but its window is gone or now belongs to another \
                          program, and GWZ does not ask another SSH agent in its place. Start Pageant and load your \
                          key, then run the command again.";

/// Pageant answered, and holds no key this transport can use (none, or only types it does not offer).
const NO_KEY_WORDS: &str = "Pageant has no key this transport can use. Load an RSA, ECDSA or ed25519 key into \
                            Pageant, or close Pageant to use the OpenSSH agent.";

/// Pageant would not serve the request: a Pageant of another user, or one at a higher privilege level (Windows blocks
/// the message across integrity levels), as opposed to one that is gone. `consulted` is the clause that notes
/// `SSH_AUTH_SOCK` was set and not consulted, or nothing.
fn denied_words(sock_ignored: bool) -> String {
    let consulted = if sock_ignored {
        " SSH_AUTH_SOCK was not consulted."
    } else {
        ""
    };
    format!(
        "Pageant refused the request. Pageant is the SSH agent GWZ selected, because a Pageant window is \
         running.{consulted} It may be running as another user, or at a higher privilege level than this command (for \
         example as administrator), so it does not serve this account. GWZ does not ask another SSH agent in its \
         place. Run Pageant as the same user and at the same privilege level as this command, or close Pageant to use \
         the OpenSSH agent."
    )
}

impl AgentSource {
    /// The words for a Pageant that refused the request before a key was offered, or `None` for any other source.
    pub(super) fn denied_reason(&self) -> Option<String> {
        match self {
            Self::Pageant { sock_ignored, .. } => Some(denied_words(*sock_ignored)),
            _ => None,
        }
    }

    /// The words for an agent step that failed with `failure` when the source is Pageant, or `None` when the failure
    /// is not one of Pageant's (another source, or another step) and the generic words stand. `stall_ms` is the open's
    /// `--ssh-timeout`, in milliseconds; `attempts` the attempt the failure ended, if it is known.
    pub(crate) fn pageant_reason(
        &self,
        failure: &Failure,
        stall_ms: u64,
        attempts: Option<(u32, u32)>,
    ) -> Option<String> {
        let Self::Pageant { .. } = self else {
            return None;
        };
        let facts = failure.facts.as_ref()?;
        // The endpoint marks the agent step by the method its facts name (`ssh_local`).
        if failure.effect != Effect::None || facts.method != AuthMethod::SshAgent {
            return None;
        }
        let mut words = match (failure.code, failure.setup_cause) {
            (ErrorCode::Timeout, Some(SetupFailureCause::Stall)) => {
                stall_words(&stall_seconds(stall_ms)?)
            }
            (ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)) => BUDGET_WORDS.to_owned(),
            (ErrorCode::Timeout, None) => cap_words(),
            (ErrorCode::Unavailable, Some(SetupFailureCause::NotFound)) => GONE_WORDS.to_owned(),
            // The setup reports a Pageant that refused the request as unavailable with no cause (`AgentFault::Denied`).
            (ErrorCode::Unavailable, None) if !facts.credential_offered => self.denied_reason()?,
            (ErrorCode::Authentication, _) if !facts.credential_offered => NO_KEY_WORDS.to_owned(),
            _ => return None,
        };
        if let Some((attempt, total)) = attempts {
            words.push_str(&format!(" (attempt {attempt} of {total})"));
        }
        Some(words)
    }
}
