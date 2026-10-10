//! In-memory explicit-key authentication; no agent, path reopen or fallback.
use super::{
    agent_job::Control, ssh_connection::SshConnection, ssh_key_snapshot::Entry, ssh_network,
};
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
}
pub(crate) fn authenticate_reporting(
    connection: SshConnection,
    trusted: &[u8],
    entry: Arc<Entry>,
    control: Arc<Control>,
    mut offered: impl FnMut(),
    mut rejected: impl FnMut(),
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
        let result = userauth_from_memory(owner.connection.session(), user, owner.entry.text());
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
                // A wait on the server's reply: it ends when the socket is
                // ready, within the control's quantum, stall and aggregate bounds.
                ssh_network::wait_session(&mut owner.connection, &control)?;
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

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// Public-key authentication with a PEM key held in memory, which `ssh2` offers where libssh2 is built on OpenSSL.
        fn userauth_from_memory(session: &ssh2::Session, user: &str, key: &str) -> Result<(), ssh2::Error> {
            session.userauth_pubkey_memory(user, None, key, None)
        }
    } else {
        /// The header of the one key form libssh2's CNG backend reads.
        const CNG_KEY_HEADER: &str = "-----BEGIN RSA PRIVATE KEY-----";

        /// The same call on Windows. `ssh2` offers `userauth_pubkey_memory` only with OpenSSL, but libssh2's CNG
        /// backend implements `libssh2_userauth_publickey_frommemory` for RSA keys in PEM form, so this calls it
        /// through the `-sys` crate, as `ssh2` does on Unix. A key in memory never touches a file.
        ///
        /// CNG reads only the traditional PEM form, and given an OpenSSH-format (or any other) key it does not
        /// return: the call blocks the setup thread beyond any cancellation (found on dabeest, step 1.4). So
        /// the key is refused here, before the call, unless it has the `RSA PRIVATE KEY` header CNG reads.
        fn userauth_from_memory(session: &ssh2::Session, user: &str, key: &str) -> Result<(), ssh2::Error> {
            if !key.trim_start().starts_with(CNG_KEY_HEADER) {
                return Err(ssh2::Error::new(
                    ssh2::ErrorCode::Session(libssh2_sys::LIBSSH2_ERROR_METHOD_NOT_SUPPORTED),
                    "Windows reads only RSA private keys in the traditional PEM form",
                ));
            }
            let user = std::ffi::CString::new(user)?;
            let key = std::ffi::CString::new(key)?;
            let mut raw = session.raw();
            // SAFETY: the exclusive session guard, NUL-terminated user and key with their lengths, no public-key
            // text and no passphrase; libssh2 keeps none of the pointers after the call returns.
            let rc = unsafe {
                libssh2_sys::libssh2_userauth_publickey_frommemory(
                    &mut *raw,
                    user.as_ptr(),
                    user.as_bytes().len(),
                    std::ptr::null(),
                    0,
                    key.as_ptr(),
                    key.as_bytes().len(),
                    std::ptr::null(),
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(rc)))
            }
        }
    }
}
