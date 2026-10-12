//! The network setup's steps that a test can replace, and the one retry of a key exchange that libssh2's WinCNG
//! backend ends on its own side (operator decision D14).
//!
//! libssh2 on Windows CNG fails about one key exchange in a hundred with `LIBSSH2_ERROR_KEX_FAILURE` (-8), whatever
//! the server. On Windows the setup connects afresh after that failure, once, inside the same setup attempt: the
//! attempt's deadlines bound both tries and the retry machine sees one attempt. Elsewhere the failure ends the
//! attempt as it always did.
use super::super::agent_job::{Control, Mark};
use cfg_if::cfg_if;
use ssh2::{ErrorCode, Session};
use std::{
    io,
    net::{SocketAddr, TcpStream},
};

/// The code of libssh2's key exchange failure, "Unable to exchange encryption keys".
pub(crate) const KEX_FAILURE: ErrorCode = ErrorCode::Session(-8);

/// Whether the setup connects afresh after a failed key exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KexRetry {
    /// The failure ends the attempt.
    Never,
    /// The setup connects afresh once, if the rest of the rules allow it.
    Once,
}

cfg_if! {
    if #[cfg(windows)] {
        impl KexRetry {
            /// The platform's policy: WinCNG's failure is rare and is the library's own.
            pub(crate) const PLATFORM: Self = Self::Once;
        }
    } else {
        impl KexRetry {
            /// The platform's policy: the failure ends the attempt, as it always did.
            pub(crate) const PLATFORM: Self = Self::Never;
        }
    }
}

/// The two calls of the setup that reach the network, which a test replaces to make a connection or a key exchange
/// fail on cue.
pub(crate) trait Steps {
    /// One TCP connect to `address`, within `control`'s bounds.
    fn connect(&mut self, address: SocketAddr, control: &Control) -> io::Result<TcpStream>;
    /// One try at the session's handshake (a nonblocking session answers "would block" until it is done).
    fn handshake(&mut self, session: &mut Session) -> Result<(), ssh2::Error>;
}

/// The real steps.
pub(crate) struct Native;

impl Steps for Native {
    fn connect(&mut self, address: SocketAddr, control: &Control) -> io::Result<TcpStream> {
        super::connect(address, control)
    }

    fn handshake(&mut self, session: &mut Session) -> Result<(), ssh2::Error> {
        session.handshake()
    }
}

/// What one network setup connects with, and may retry on.
pub(crate) struct Network<'a, S: Steps = Native> {
    pub(crate) steps: S,
    pub(crate) kex: KexRetry,
    /// Whether the open's facts say a credential could already have been offered.
    pub(crate) credential_offered: &'a dyn Fn() -> bool,
}

impl<'a> Network<'a, Native> {
    /// The production setup: the real steps and the platform's policy.
    pub(crate) fn production(credential_offered: &'a dyn Fn() -> bool) -> Self {
        Self {
            steps: Native,
            kex: KexRetry::PLATFORM,
            credential_offered,
        }
    }
}

impl<S: Steps> Network<'_, S> {
    /// Whether a handshake that failed with a key exchange failure (-8), the one failure retried, may be tried
    /// again on a new connection. Only once; never when a credential could have been offered (the handshake comes
    /// before any authentication step, so the open's facts must say none was); and never when the budget left is
    /// no more than the failed try used since `started`, so that a repeat could only run the deadline out and hide
    /// the failure under a timeout.
    pub(crate) fn may_retry(
        &self,
        already_retried: bool,
        control: &Control,
        started: Mark,
    ) -> bool {
        self.kex == KexRetry::Once
            && !already_retried
            && !(self.credential_offered)()
            && control.has_room_since(started)
    }
}
