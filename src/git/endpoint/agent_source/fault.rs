//! Why the agent step could not start, as the setup's error carries it. The setup's failure crosses the wire as a
//! code with no text, so the step only has to say *that* it was the agent: the host words it from the source it chose
//! ([`super::AgentSource::unavailable_reason`]). Nothing here holds a path or a name from the user's environment.
use std::{fmt, io};

/// A reason the agent step could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentFault {
    /// `SSH_AUTH_SOCK` holds a value that was refused when the runtime was built.
    Refused,
    /// This build has no channel for the selected agent.
    NoChannel,
    /// The agent's pipe is not there: the service is not running, or nothing serves the name.
    NotRunning,
    /// The agent's pipe refused this account.
    Denied,
}

impl fmt::Display for AgentFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Refused => "the SSH agent named by SSH_AUTH_SOCK is not one gwz uses",
            Self::NoChannel => "gwz has no channel for the selected SSH agent",
            Self::NotRunning => "the SSH agent is not running",
            Self::Denied => "the SSH agent refused this account",
        })
    }
}

impl std::error::Error for AgentFault {}

impl AgentFault {
    /// The setup's error. Its kind is `Unsupported`, which the setup reports as `Unavailable` with no cause that a
    /// retry would act on; an agent that is not running is `NotFound`, which the setup reports as `Unavailable` with
    /// the cause `NotFound` (no retry acts on that either). Never `PermissionDenied`: that is an authentication
    /// failure, and no key was offered.
    pub(crate) fn into_error(self) -> io::Error {
        let kind = match self {
            Self::NotRunning => io::ErrorKind::NotFound,
            Self::Refused | Self::NoChannel | Self::Denied => io::ErrorKind::Unsupported,
        };
        io::Error::new(kind, self)
    }

    /// The fault `error` stands for, if it was made by [`Self::into_error`].
    pub(crate) fn of_error(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }
}
