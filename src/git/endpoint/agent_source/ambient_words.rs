//! What the user is told when the ambient (all-keys) agent route reached an agent and still did not authenticate: the
//! diagnosability parity fix for finding F4 of run `2026-10-18-gapsb`, where the transport printed only
//! `ssh setup failed: Authentication` for what gwz 1.0.17 worded as "no usable identity in the ssh-agent".
//!
//! The words say what was tried and give the next action for the kind of agent that answered ([`AgentKind`]: `ssh-add`
//! reaches a Unix agent and the Windows OpenSSH agent, and cannot add a key to Pageant). They claim nothing the failure
//! does not show: the wire carries no count of the keys offered, so none is named, and a last key that the server did
//! not reject (the agent declined to sign, or no algorithm matched) is not called a rejection. The exact route's
//! `--identity` words (decision D38, the K texts) are a separate selection, for an open that named a key file.
use super::{AgentKind, AgentSource};
use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, Failure};

/// How the ambient step ended, as the facts show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    /// No key was offered: the agent holds none, holds none of a type offered, or would not list its keys.
    NothingOffered,
    /// Keys were offered and the server rejected the last one.
    Rejected,
    /// Keys were offered and the last was not rejected by the server (the agent did not sign, or no algorithm matched).
    Unfinished,
}

/// The words for an agent that holds no key to offer: a Unix agent, or the Windows OpenSSH agent (Pageant's are
/// `NO_KEY_WORDS`).
fn nothing_offered(unix: bool, host: &str) -> String {
    let (agent, remedy) = if unix {
        (
            "The SSH agent",
            "(ssh-add -l lists what the agent holds, and ssh-add -X unlocks a locked agent)",
        )
    } else {
        (
            "The OpenSSH agent",
            "(ssh-add -l lists what the agent holds)",
        )
    };
    format!(
        "{agent} has no key GWZ can offer {host}: it holds no key, holds only key types GWZ does not use, or would not \
         list its keys, so GWZ offered {host} nothing. Add a key with ssh-add <the private key file> {remedy}, then \
         run the command again."
    )
}

/// The words for keys that the server rejected.
fn rejected(kind: AgentKind, sock_ignored: bool, host: &str) -> String {
    let check = format!("Check that the key you mean to use is added to your account on {host}.");
    match kind {
        AgentKind::Unix | AgentKind::OpenSsh => {
            let agent = if kind == AgentKind::Unix {
                "SSH"
            } else {
                "OpenSSH"
            };
            format!(
                "GWZ offered {host} every key the {agent} agent holds that it can use, and {host} accepted none of \
                 them. {check} ssh-add -l lists the keys the agent holds, and ssh-add <the private key file> adds \
                 another; then run the command again."
            )
        }
        AgentKind::Pageant => format!(
            "GWZ offered {host} every key Pageant holds that it can use, and {host} accepted none of them. {check} \
             Pageant's View Keys lists what it holds and Add Key loads another (ssh-add cannot add a key to \
             Pageant); then run the command again. Pageant is the agent GWZ uses because a Pageant window is \
             running{consulted}, so GWZ did not ask the OpenSSH agent; close Pageant to use it.",
            consulted = if sock_ignored {
                "; SSH_AUTH_SOCK was not consulted"
            } else {
                ""
            }
        ),
    }
}

/// The words for keys offered where the last one was not rejected by the server.
fn unfinished(kind: AgentKind, host: &str) -> String {
    let (agent, declined) = match kind {
        AgentKind::Unix => (
            "the SSH agent",
            "a confirmation asked for with ssh-add -c was declined, or a security key was not touched in time",
        ),
        AgentKind::OpenSsh => (
            "the OpenSSH agent",
            "a confirmation was declined, or a security key was not touched in time",
        ),
        AgentKind::Pageant => ("Pageant", "a Pageant prompt was declined"),
    };
    format!(
        "GWZ offered {host} the keys {agent} holds, but none completed authentication: the agent may have declined to \
         sign ({declined}), or {host} lists no signature algorithm GWZ can use for the key. Fix that and run the \
         command again."
    )
}

impl AgentSource {
    /// The words for an ambient agent step that reached its agent and ended in `Authentication`, or `None` when the
    /// failure is not that (another credential's step or code, a step with effects, a source that reaches no agent,
    /// or Pageant with no key, which `pageant_reason` words) and the generic words stand. `host` is the destination;
    /// `attempts` the attempt the failure ended, if it is known. The caller leaves an open that selected a key file to
    /// the exact route.
    pub(crate) fn ambient_reason(
        &self,
        failure: &Failure,
        host: &str,
        attempts: Option<(u32, u32)>,
    ) -> Option<String> {
        let kind = self.kind()?;
        let facts = failure.facts.as_ref()?;
        // The endpoint marks the agent step by the method its facts name (`ssh_local`).
        if failure.code != ErrorCode::Authentication
            || failure.effect != Effect::None
            || facts.method != AuthMethod::SshAgent
        {
            return None;
        }
        let ending = match (facts.credential_offered, facts.authenticated) {
            (false, _) => Ending::NothingOffered,
            (true, Some(false)) => Ending::Rejected,
            (true, _) => Ending::Unfinished,
        };
        let sock_ignored = matches!(
            self,
            Self::Pageant {
                sock_ignored: true,
                ..
            }
        );
        let mut words = match (ending, kind) {
            // Pageant that listed no key keeps its own words.
            (Ending::NothingOffered, AgentKind::Pageant) => return None,
            (Ending::NothingOffered, kind) => nothing_offered(kind == AgentKind::Unix, host),
            (Ending::Rejected, kind) => rejected(kind, sock_ignored, host),
            (Ending::Unfinished, kind) => unfinished(kind, host),
        };
        if let Some((attempt, total)) = attempts {
            words.push_str(&format!(" (attempt {attempt} of {total})"));
        }
        Some(words)
    }
}
