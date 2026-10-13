//! What the user is told when the agent step cannot start. The setup's failure carries no text across the wire, so
//! the host words it from the source its runtime chose.
use super::AgentSource;
use gwz_transport::protocol::SetupFailureCause;

impl AgentSource {
    /// The words for an agent step that failed before a key was offered, or `None` when the source has none of its
    /// own and the generic ones stand. `cause` is the setup failure's cause: a pipe that is not there is told from one
    /// that refused the caller by it.
    pub(crate) fn unavailable_reason(&self, cause: Option<SetupFailureCause>) -> Option<String> {
        match self {
            Self::None | Self::Path(_) => None,
            Self::Refused(refusal) => Some(refusal.to_string()),
            // A Pageant that is gone is told apart from one that is merely selected by the same cause as the pipe's.
            Self::Pageant { .. } if cause == Some(SetupFailureCause::NotFound) => {
                Some(super::pageant_words::GONE_WORDS.to_owned())
            }
            // A Pageant that refused the request (`AgentFault::Denied`, which carries no cause) says so; the selected
            // message below stands for a cause that has no words of its own.
            Self::Pageant { .. } if cause.is_none() => self.denied_reason(),
            Self::Pageant { sock_ignored, .. } => Some(pageant_selected(*sock_ignored)),
            Self::NamedPipe(pipe) => Some(match cause {
                // The pipe is not there. The default pipe is the OpenSSH Authentication Agent service's, which is
                // disabled on many machines (U19); a pipe the variable names is told by its name.
                Some(SetupFailureCause::NotFound) if pipe.is_default() => STOPPED_AGENT.to_owned(),
                Some(SetupFailureCause::NotFound) => format!(
                    "The SSH agent pipe SSH_AUTH_SOCK names ({}) does not exist, so no agent is serving it. Start the \
                     agent that creates it, or unset SSH_AUTH_SOCK to use the OpenSSH agent's default pipe.",
                    pipe.as_str()
                ),
                _ => format!(
                    "GWZ could not open the SSH agent pipe {}. The agent may be running as another user, or it may \
                     refuse this account.",
                    pipe.as_str()
                ),
            }),
        }
    }
}

/// The message for a stopped OpenSSH agent service (the design's §14 item 13, owed to step 3.3; the user guide quotes
/// the same text): it names the service and the commands, which only an administrator can run.
const STOPPED_AGENT: &str = "The OpenSSH Authentication Agent service (ssh-agent) is not running. As administrator \
                             run: Set-Service ssh-agent -StartupType Automatic; Start-Service ssh-agent. Then add a \
                             key with ssh-add. To use Pageant instead, load the key into Pageant.";

/// The message that names Pageant as the selected source (the design's §14 item 13, U6). Pageant wins even when the
/// OpenSSH agent is running (gwz 1.0.17 does the same); the way back to the OpenSSH agent is to close Pageant.
fn pageant_selected(sock_ignored: bool) -> String {
    let consulted = if sock_ignored {
        "; SSH_AUTH_SOCK was not consulted"
    } else {
        ""
    };
    format!(
        "Pageant is the SSH agent GWZ selected, because a Pageant window is running{consulted}. To use the OpenSSH \
         agent instead, close Pageant."
    )
}
