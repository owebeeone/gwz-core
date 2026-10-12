//! The SSH agent a runtime uses: chosen once, before any connection, and opened when a setup needs an agent
//! (GwzTransportWindowsParityPlan.md, steps 3.2a and 3.2b; amendment 2 §3.5; TR1.8 §4).
//!
//! The source is an owned value, not a path that sometimes means something else. A runtime captures it from its
//! environment snapshot when it is built and hands every endpoint of that runtime the same value, so a worker never
//! rereads the environment and a failed agent never falls back to another one.
//!
//! Unix has the socket path `SSH_AUTH_SOCK` names, or no agent. Windows has three agents and one rule ([`select`]): a
//! visible Pageant first, else the local pipe `SSH_AUTH_SOCK` names, else the OpenSSH agent's default pipe; a value of
//! that variable that is not a local pipe is a refusal that names the variable. The rule is pure and every platform's
//! test run covers it; only Windows applies it. The channels are separate steps: the OpenSSH pipe is 3.3, Pageant 3.6.
use super::{
    agent_client::{Agent, Channel},
    agent_job::Control,
    agent_pageant::PageantPin,
};
use std::{ffi::OsStr, io, path::PathBuf, sync::Arc};

// Unix builds none of the Windows forms, and Windows none of the Unix one; both are tested everywhere.
#[allow(dead_code)]
mod fault;
mod pageant_words;
#[allow(dead_code)]
mod pipe_name;
#[allow(dead_code)]
mod select;
mod words;

pub(crate) use fault::AgentFault;
pub(crate) use pipe_name::{AgentRefusal, PipeName};
// Windows applies the rule; Unix builds only test it.
#[allow(unused_imports)]
pub(crate) use select::select;

/// The agent a runtime's setups use.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AgentSource {
    /// No agent: a setup that needs one is refused with the no-agent refusal (S4.3), before any credential is
    /// offered.
    #[default]
    None,
    /// The Unix socket `SSH_AUTH_SOCK` names.
    Path(PathBuf),
    /// A pipe on this computer (Windows): the one `SSH_AUTH_SOCK` names, or the OpenSSH agent's default.
    NamedPipe(PipeName),
    /// The Pageant window found when the runtime was built (Windows), with the process that owned it: a request goes
    /// to that window only while that process still owns it, and a Pageant that is gone or replaced is a refusal, never
    /// another agent. `sock_ignored` is true when `SSH_AUTH_SOCK` was set and so was not consulted.
    Pageant { sock_ignored: bool, pin: PageantPin },
    /// `SSH_AUTH_SOCK` holds something Windows will not connect to. The runtime is built; a setup that needs an
    /// agent is refused with the text that names the variable.
    Refused(AgentRefusal),
}

impl AgentSource {
    /// The source for a configuration that holds an optional socket path.
    pub(crate) fn from_path(path: Option<PathBuf>) -> Self {
        path.map_or(Self::None, Self::Path)
    }

    /// The source a runtime uses given the `SSH_AUTH_SOCK` of its environment snapshot: an unset or empty variable
    /// is none.
    pub(crate) fn capture(sock: Option<&OsStr>) -> Self {
        capture_platform(sock.filter(|value| !value.is_empty()))
    }

    /// Opens the agent's channel under `control`, which bounds the connect and every exchange after it. A source
    /// that names no agent is the no-agent refusal; one that cannot be used says so as an [`AgentFault`].
    pub(crate) fn open(&self, control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
        match self {
            Self::None => Err(io::ErrorKind::NotFound.into()),
            Self::Path(path) => open_path(path, control),
            Self::NamedPipe(name) => open_pipe(name, control),
            Self::Pageant { pin, .. } => open_pageant(pin, control),
            Self::Refused(_) => Err(AgentFault::Refused.into_error()),
        }
    }
}

impl Channel for Box<dyn Channel> {
    fn wait(&mut self, writing: bool, control: &Control) -> io::Result<()> {
        (**self).wait(writing, control)
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Unix takes the socket path `SSH_AUTH_SOCK` names.
        fn capture_platform(sock: Option<&OsStr>) -> AgentSource {
            AgentSource::from_path(sock.map(PathBuf::from))
        }

        /// The agent on the Unix socket at `path`.
        fn open_path(path: &std::path::Path, control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            Ok(super::agent_socket::connect(path, control)?.boxed())
        }

        /// Unix has no named-pipe agent: the source is never chosen there.
        fn open_pipe(_name: &PipeName, _control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            Err(AgentFault::NoChannel.into_error())
        }

        /// Unix has no Pageant: the source is never chosen there.
        fn open_pageant(_pin: &PageantPin, _control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            Err(AgentFault::NoChannel.into_error())
        }
    } else {
        /// Windows chooses between Pageant and the pipes (TR1.8 §4); the probe looks for Pageant's window as Pageant's
        /// own protocol does (`FindWindowW`), once.
        fn capture_platform(sock: Option<&OsStr>) -> AgentSource {
            select(sock, super::agent_pageant::find)
        }

        /// Windows takes no socket path: its agents are a pipe and Pageant (steps 3.3 and 3.6).
        fn open_path(_path: &std::path::Path, _control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            Err(io::ErrorKind::NotFound.into())
        }

        /// The agent on the local pipe `name` (the OpenSSH agent).
        fn open_pipe(name: &PipeName, control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            let pipe = super::agent_pipe::connect(name, &control)?;
            Ok(Agent::new(pipe, control).boxed())
        }

        /// The agent in the Pageant window `pin` names. A Pageant that is already gone, or whose window another
        /// program now owns, is the agent not running (words from the source and the cause), and no other agent is
        /// tried in its place.
        fn open_pageant(pin: &PageantPin, control: Arc<Control>) -> io::Result<Agent<Box<dyn Channel>>> {
            use super::agent_pageant::{PageantChannel, PageantWindow};
            let window = super::agent_pageant::open(pin)?;
            if !window.owner_pinned() {
                return Err(AgentFault::NotRunning.into_error());
            }
            Ok(Agent::new(PageantChannel::new(Arc::new(window)), control).boxed())
        }
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod pageant_words_tests; mod select_tests; mod tests; mod words_tests; } }
