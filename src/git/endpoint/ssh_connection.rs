//! Socket ownership needed to terminate SSH before nonblocking native destruction.
use ssh2::Session;
use std::io;
use std::net::{Shutdown, TcpStream};

/// One physical connection. The host authenticates and verifies trust through
/// `session`, then transfers this owner to a channel. Never retain Session or
/// Channel clones: their native lifetimes must end before this owner is dropped.
pub struct SshConnection {
    session: Session,
    socket: TcpStream,
    /// Ends the socket's traffic in both directions: [`shutdown_both`] outside tests.
    shutdown: fn(&TcpStream) -> io::Result<()>,
}

/// The socket's own shutdown of both directions.
fn shutdown_both(socket: &TcpStream) -> io::Result<()> {
    socket.shutdown(Shutdown::Both)
}

impl SshConnection {
    pub fn new(socket: TcpStream) -> io::Result<Self> {
        let mut session = Session::new()?;
        session.set_tcp_stream(socket.try_clone()?);
        Ok(Self {
            session,
            socket,
            shutdown: shutdown_both,
        })
    }

    /// Setup access only; do not clone the session or open additional channels.
    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn set_nonblocking(&mut self) -> io::Result<()> {
        self.socket.set_nonblocking(true)?;
        self.session.set_blocking(false);
        Ok(())
    }

    pub(super) fn native(&self) -> &Session {
        &self.session
    }

    /// A duplicate of the socket, for watching it while the session is idle.
    /// Shutdown through either handle ends both.
    pub(super) fn watch_socket(&self) -> io::Result<TcpStream> {
        self.socket.try_clone()
    }

    /// Shutdown affects both socket handles. Afterward native EOF/close attempts
    /// see termination instead of a peer that can indefinitely return EAGAIN.
    ///
    /// A socket whose peer already ended it is terminated: Unix reports `NotConnected`, and Microsoft documents
    /// `ConnectionReset` and `ConnectionAborted` (WSAECONNRESET, WSAECONNABORTED) for `shutdown` on a connection
    /// that the peer reset or the stack aborted. (The Windows host the transport is qualified on reported success for
    /// a peer's reset in every state tried; the documented errors are accepted all the same.)
    pub(super) fn terminate(&self) -> io::Result<()> {
        match (self.shutdown)(&self.socket) {
            Ok(()) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotConnected
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for SshConnection {
    fn drop(&mut self) {
        // Session's destructor runs only AFTER this body. This also protects idle
        // eviction and partially opened native channels owned by the session.
        let _ = self.terminate();
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl SshConnection {
            /// This connection with `shutdown` as its socket's shutdown, to play a socket that fails it.
            pub(super) fn with_shutdown(mut self, shutdown: fn(&TcpStream) -> io::Result<()>) -> Self {
                self.shutdown = shutdown;
                self
            }
        }

        mod tests;
    }
}
