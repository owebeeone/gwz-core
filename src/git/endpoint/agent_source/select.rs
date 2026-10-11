//! The Windows agent-source rule (TR1.8 §4; plan step 3.2b): a visible Pageant first, else the local pipe
//! `SSH_AUTH_SOCK` names, else `\\.\pipe\openssh-ssh-agent`. Both agents running selects Pageant, as libssh2 does.
//! A failure of the chosen agent never falls back to another one.
use super::{AgentSource, PipeName};
use std::ffi::OsStr;

/// The source for a Windows runtime: `sock` is the snapshot's `SSH_AUTH_SOCK`, and `pageant_visible` the probe for a
/// Pageant window discoverable by Pageant's own protocol (`FindWindowW`, which is step 3.6's; a hidden window counts).
/// The probe runs once, before the variable is looked at: when Pageant wins the variable is not consulted, so a value
/// that would be refused is not.
pub(crate) fn select(sock: Option<&OsStr>, pageant_visible: impl FnOnce() -> bool) -> AgentSource {
    let sock = sock.filter(|value| !value.is_empty());
    if pageant_visible() {
        return AgentSource::Pageant {
            sock_ignored: sock.is_some(),
        };
    }
    match sock {
        Some(value) => {
            PipeName::parse(value).map_or_else(AgentSource::Refused, AgentSource::NamedPipe)
        }
        None => AgentSource::NamedPipe(PipeName::default_pipe()),
    }
}
