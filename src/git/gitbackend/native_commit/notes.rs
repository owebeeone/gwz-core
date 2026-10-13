//! Telling a person why a commit or tag ran the Git CLI.
//!
//! Every refusal of the route rule names its reason ([`CliReason`]); this turns
//! it into one line for a reader the caller gives the backend. The CLI gives
//! one only for `--verbose` in human output, so no other output mode carries it.

use std::path::Path;
use std::sync::Arc;

use super::ident::IdentRefusal;
use super::route::CliReason;

/// A reader of the lines that say why a commit or tag ran the Git CLI, given
/// to a backend by its caller. Cloned with the backend; there is no global.
#[derive(Clone)]
pub struct RouteNotes {
    read: Arc<dyn Fn(&str) + Send + Sync>,
}

impl RouteNotes {
    pub fn new(read: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
        }
    }

    /// Tell the reader that `operation` in the repository at `path` ran Git.
    pub(super) fn cli_used(&self, operation: &str, path: &Path, reason: &CliReason) {
        (self.read)(&format!(
            "note: {operation} in {}: ran the Git CLI ({reason})",
            path.display()
        ));
    }
}

impl std::fmt::Display for CliReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forced => write!(f, "the Git CLI was asked for"),
            Self::Environment(name) => write!(f, "environment: {name}"),
            Self::CannotOpen => write!(f, "libgit2 did not open the repository"),
            Self::Bare => write!(f, "the repository is bare"),
            Self::ObjectFormat => write!(f, "the repository does not use SHA-1"),
            Self::Config(what) => write!(f, "configuration native does not reproduce: {what}"),
            Self::SystemConfig(what) => write!(f, "system configuration: {what}"),
            Self::SystemFile { why, path } => {
                write!(f, "system configuration file {}: {why}", path.display())
            }
            Self::Hook(name) => write!(f, "a hook would run: {name}"),
            Self::Signing(what) => write!(f, "signing is configured: {what}"),
            Self::InProgress(what) => write!(f, "an operation is in progress: {what}"),
            Self::IndexState(what) => write!(f, "index state native does not reproduce: {what}"),
            Self::Identity(IdentRefusal::NotExplicit(what)) => {
                write!(f, "the {what} is not given explicitly")
            }
            Self::Identity(IdentRefusal::Unusual(what)) => {
                write!(f, "the {what} has a form native does not reproduce")
            }
            Self::Identity(IdentRefusal::Date) => {
                write!(f, "the date has a form native does not parse")
            }
            Self::Message(what) => write!(f, "message: {what}"),
            Self::AllCommit(what) => write!(f, "commit -a: {what}"),
            Self::TagName => write!(
                f,
                "the tag name is one Git and libgit2 may judge differently"
            ),
            Self::HeadShape => write!(f, "HEAD is not a branch tip or commit native can use"),
        }
    }
}
