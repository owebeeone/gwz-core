//! A password beside the user in an SSH URL, used as 1.0.17's libgit2 1.9.7
//! uses it (TR2.18; `transports/ssh_libssh2.c`).
//!
//! | libgit2 | here |
//! | --- | --- |
//! | `_git_ssh_setup_conn` makes a user and password credential when the URL has both (lines 843-846), after it checks the host key (line 830) | the setup has trust before [`authenticate`] runs, which checks the trusted key again |
//! | `list_auth_methods` sends libssh2's `none` request and reads the server's methods, by prefix, after an optional comma (lines 1019-1066) | `list` and [`methods`] |
//! | the password is offered first, and only when the server offers `password` (lines 851-854) | [`authenticate`] |
//! | a refusal, an expired password or an unverified key is `GIT_EAUTH` (lines 371-374), and then the credential callback is asked, with the methods first listed (lines 856-863) | [`Password::Declined`]: the setup goes on to its key or agent, which gwz's callback gives libgit2 only when `publickey` is listed (`transport_support.rs:244-280`) |
//! | any other failure ends the setup (lines 376-380) | an error |
//!
//! A server that lists no `publickey` uses the configured helper on the accepted
//! ambient password-only route. A server that takes
//! the `none` request fails on 1.0.17, whose callback has no type to answer;
//! here it has authenticated the user.
use super::{
    agent_job::Control, ssh_connection::SshConnection, ssh_handoff::UrlPassword, ssh_network,
};
use libssh2_sys::{
    LIBSSH2_ERROR_AUTHENTICATION_FAILED, LIBSSH2_ERROR_EAGAIN, LIBSSH2_ERROR_PASSWORD_EXPIRED,
    LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED,
};
use std::{
    ffi::{CStr, CString, c_uint},
    io,
};

/// What the server made of the URL's password.
pub(crate) enum Password {
    /// It took the password, or authenticated the user without one.
    Accepted(SshConnection),
    /// It did not offer password authentication, or refused the password,
    /// and it offers keys: the setup's key or agent comes next.
    Declined(SshConnection),
}

/// The methods `list_auth_methods` reads from the server's list.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Methods {
    pub(crate) publickey: bool,
    pub(crate) password: bool,
}

/// `list_auth_methods`'s reading of a method list: at each name's start, or
/// just after a comma, a name that starts with `publickey`, `password` or
/// `keyboard-interactive` counts, and its rest is read on as the next name's
/// start; any other name is skipped to the next comma.
pub(crate) fn methods(list: &[u8]) -> Methods {
    let mut found = Methods::default();
    let mut rest = Some(list);
    while let Some(mut at) = rest {
        if let Some(after) = at.strip_prefix(b",") {
            at = after;
        }
        rest = if let Some(after) = at.strip_prefix(b"publickey") {
            found.publickey = true;
            Some(after)
        } else if let Some(after) = at.strip_prefix(b"password") {
            found.password = true;
            Some(after)
        } else if let Some(after) = at.strip_prefix(b"keyboard-interactive") {
            Some(after)
        } else {
            at.iter()
                .position(|byte| *byte == b',')
                .map(|comma| &at[comma..])
        };
    }
    found
}

/// Offers `password` for `user` as libgit2 does, on a connection whose host
/// key is `trusted`. `offered` runs as the password goes, and `rejected` as
/// the server refuses it.
pub(crate) fn authenticate(
    mut connection: SshConnection,
    user: &str,
    password: &UrlPassword,
    trusted: &[u8],
    control: &Control,
    mut offered: impl FnMut(),
    mut rejected: impl FnMut(),
) -> io::Result<Password> {
    control.check()?;
    let user = CString::new(user).map_err(|_| io::ErrorKind::InvalidInput)?;
    if user.as_bytes().is_empty() || user.as_bytes().len() > 1024 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if trusted.is_empty() || connection.session().host_key().map(|(key, _)| key) != Some(trusted) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    connection.set_nonblocking()?;
    let Some(methods) = list(&mut connection, &user, control)? else {
        return Ok(Password::Accepted(connection));
    };
    if methods.password {
        offered();
        if attempt(&mut connection, &user, password.bytes(), control)? {
            return Ok(Password::Accepted(connection));
        }
        rejected();
    }
    if !methods.publickey {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(Password::Declined(connection))
}

pub(crate) fn password_only(
    connection: &mut SshConnection,
    user: &str,
    control: &Control,
) -> io::Result<bool> {
    let user = CString::new(user).map_err(|_| io::ErrorKind::InvalidInput)?;
    connection.set_nonblocking()?;
    Ok(list(connection, &user, control)?
        .is_some_and(|methods| methods.password && !methods.publickey))
}

pub(crate) fn authenticate_helper(
    mut connection: SshConnection,
    secret: &mut super::https_auth::Secret,
    trusted: &[u8],
    control: &Control,
    mut offered: impl FnMut(),
    mut rejected: impl FnMut(),
) -> io::Result<SshConnection> {
    control.check()?;
    if trusted.is_empty() || connection.session().host_key().map(|(key, _)| key) != Some(trusted) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let (user, password) = secret.ssh_parts();
    offered();
    if attempt(&mut connection, user, password, control)? {
        Ok(connection)
    } else {
        rejected();
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

/// libssh2's `none` request: the server's methods, or None when it took the
/// request and authenticated the user.
fn list(
    connection: &mut SshConnection,
    user: &CStr,
    control: &Control,
) -> io::Result<Option<Methods>> {
    let length = user.to_bytes().len() as c_uint;
    loop {
        control.begin_wait()?;
        let (list, errno, authenticated) = {
            let mut session = connection.session().raw();
            let session = &mut *session;
            // SAFETY: an exclusive session guard and a NUL-terminated user.
            // libssh2 owns the list it returns until its next call on this
            // session, and it ends the list with a NUL.
            unsafe {
                let list = libssh2_sys::libssh2_userauth_list(session, user.as_ptr(), length);
                if list.is_null() {
                    (
                        None,
                        libssh2_sys::libssh2_session_last_errno(session),
                        libssh2_sys::libssh2_userauth_authenticated(session) != 0,
                    )
                } else {
                    (Some(methods(CStr::from_ptr(list).to_bytes())), 0, false)
                }
            }
        };
        control.check()?;
        match (list, errno) {
            (Some(methods), _) => {
                control.complete_wait()?;
                return Ok(Some(methods));
            }
            (None, LIBSSH2_ERROR_EAGAIN) => ssh_network::wait_session(connection, control)?,
            (None, _) if authenticated => {
                control.complete_wait()?;
                return Ok(None);
            }
            // The request or its answer failed in transit.
            (None, _) => return Err(io::ErrorKind::Other.into()),
        }
    }
}

/// libssh2's password request: whether the server took the password.
fn attempt(
    connection: &mut SshConnection,
    user: &CStr,
    password: &[u8],
    control: &Control,
) -> io::Result<bool> {
    let secret = password;
    let length = c_uint::try_from(secret.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    loop {
        control.begin_wait()?;
        let rc = {
            let mut session = connection.session().raw();
            // SAFETY: an exclusive session guard, a NUL-terminated user, and
            // the password's bytes, which outlive the call. libssh2 copies
            // them into the packet it encrypts in place, and keeps no pointer.
            unsafe {
                libssh2_sys::libssh2_userauth_password_ex(
                    &mut *session,
                    user.as_ptr(),
                    user.to_bytes().len() as c_uint,
                    secret.as_ptr().cast(),
                    length,
                    None,
                )
            }
        };
        control.check()?;
        match rc {
            0 if connection.session().authenticated() => {
                control.complete_wait()?;
                return Ok(true);
            }
            0 => return Err(io::ErrorKind::PermissionDenied.into()),
            LIBSSH2_ERROR_EAGAIN => ssh_network::wait_session(connection, control)?,
            LIBSSH2_ERROR_AUTHENTICATION_FAILED
            | LIBSSH2_ERROR_PASSWORD_EXPIRED
            | LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED => {
                control.complete_wait()?;
                return Ok(false);
            }
            _ => return Err(io::ErrorKind::Other.into()),
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn methods_are_read_as_list_auth_methods_reads_them() {
            let both = Methods { publickey: true, password: true };
            for (list, found) in [
                (&b"publickey,password,keyboard-interactive"[..], both),
                (b"password", Methods { password: true, ..Methods::default() }),
                (b"publickey", Methods { publickey: true, ..Methods::default() }),
                (b"keyboard-interactive", Methods::default()),
                (b"", Methods::default()),
                (b"gssapi-with-mic,hostbased,password", Methods { password: true, ..Methods::default() }),
                // By prefix, as libgit2 reads them, including a name's rest.
                (b"passwordless,publickey", both),
                (b"publickeypassword", both),
                (b"xpassword,publickey-x", Methods { publickey: true, ..Methods::default() }),
                // A comma the name skip lands on is passed over as a leading one.
                (b",,password", Methods { password: true, ..Methods::default() }),
                (b"x,,password", Methods { password: true, ..Methods::default() }),
            ] {
                assert_eq!(methods(list), found, "{}", String::from_utf8_lossy(list));
            }
        }
    }
}
