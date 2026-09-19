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
}

impl SshConnection {
    pub fn new(socket: TcpStream) -> io::Result<Self> {
        let mut session = Session::new()?;
        session.set_tcp_stream(socket.try_clone()?);
        Ok(Self { session, socket })
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

    /// Shutdown affects both socket handles. Afterward native EOF/close attempts
    /// see termination instead of a peer that can indefinitely return EAGAIN.
    pub(super) fn terminate(&self) -> io::Result<()> {
        match self.socket.shutdown(Shutdown::Both) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotConnected => Ok(()),
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
