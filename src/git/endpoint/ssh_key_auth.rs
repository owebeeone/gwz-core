//! In-memory explicit-key authentication; no agent, path reopen or fallback.
use super::{agent_job::Control, ssh_connection::SshConnection, ssh_key_snapshot::Entry};
use std::{io, sync::Arc};
/// The connection is destroyed before its snapshot pin on every rejected handoff.
pub(crate) struct Verified {
    connection: SshConnection,
    entry: Arc<Entry>,
}
impl Verified {
    /// Transfers unpromoted proof into a supervised setup result. Its owner must
    /// keep connection-before-pin drop order and promote only after joined handoff.
    pub(crate) fn into_parts(self) -> (SshConnection, Arc<Entry>) {
        (self.connection, self.entry)
    }

    /// Only call after the authentication Job has joined. Check the original
    /// request, not the consumed helper Control, immediately before promotion.
    pub(crate) fn publish(
        mut self,
        live: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<(SshConnection, Arc<Entry>)> {
        live()?;
        if !self.connection.session().authenticated() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.entry.promote();
        Ok((self.connection, self.entry))
    }
}
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub(crate) fn authenticate(
            connection: SshConnection,
            trusted: &[u8],
            entry: Arc<Entry>,
            control: Arc<Control>,
        ) -> io::Result<Verified> {
            authenticate_reporting(connection, trusted, entry, control, || {}, || {})
        }
        pub(crate) fn authenticate_reporting(
            connection: SshConnection, trusted: &[u8], entry: Arc<Entry>, control: Arc<Control>,
            mut offered: impl FnMut(), mut rejected: impl FnMut(),
        ) -> io::Result<Verified> {
            let mut owner = Verified { connection, entry };
            control.check()?;
            let user = owner
                .entry
                .key()
                .username
                .as_deref()
                .ok_or(io::ErrorKind::InvalidInput)?;
            if owner.entry.key().scheme != gwz_transport::protocol::Scheme::Ssh
                || user.is_empty()
                || user.len() > 128
                || user.chars().any(char::is_control)
            {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            if trusted.is_empty()
                || owner.connection.session().host_key().map(|(key, _)| key) != Some(trusted)
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            owner.connection.set_nonblocking()?;
            loop {
                control.check()?;
                offered();
                let result =
                    owner
                        .connection
                        .session()
                        .userauth_pubkey_memory(user, None, owner.entry.text(), None);
                control.check()?;
                match result {
                    Ok(()) => {
                        if !owner.connection.session().authenticated() {
                            return Err(io::ErrorKind::PermissionDenied.into());
                        }
                        return Ok(owner);
                    }
                    Err(error)
                        if error.code() == ssh2::ErrorCode::Session(libssh2_sys::LIBSSH2_ERROR_EAGAIN) =>
                    {
                        std::thread::sleep(control.quantum()?);
                    }
                    Err(error)
                        if error.code()
                            == ssh2::ErrorCode::Session(
                                libssh2_sys::LIBSSH2_ERROR_AUTHENTICATION_FAILED,
                            ) =>
                    {
                        rejected();
                        return Err(io::ErrorKind::PermissionDenied.into());
                    }
                    // PUBLICKEY_UNVERIFIED also hides response/socket failures.
                    Err(_) => {
                        return Err(io::ErrorKind::Other.into());
                    }
                }
            }
        }
    }
}
