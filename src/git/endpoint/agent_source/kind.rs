//! Which kind of agent a source reaches, for the words that tell the user how to add a key to it. `ssh-add` reaches a
//! Unix agent and the Windows OpenSSH agent and cannot add a key to Pageant, so a remedy is chosen by the kind.
//! Decision D38's selection table (step 1.3) uses the same kind for its K1 and K6 remedies.
use super::AgentSource;

/// What an agent that answered is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentKind {
    /// An OpenSSH-protocol agent on a Unix socket.
    Unix,
    /// The OpenSSH agent on a Windows named pipe.
    OpenSsh,
    /// Pageant.
    Pageant,
}

impl AgentSource {
    /// The kind of agent this source reaches, or `None` for a source that reaches none (no agent, or a value of
    /// `SSH_AUTH_SOCK` that was refused).
    pub(crate) fn kind(&self) -> Option<AgentKind> {
        match self {
            Self::None | Self::Refused(_) => None,
            Self::Path(_) => Some(AgentKind::Unix),
            Self::NamedPipe(_) => Some(AgentKind::OpenSsh),
            Self::Pageant { .. } => Some(AgentKind::Pageant),
        }
    }
}
