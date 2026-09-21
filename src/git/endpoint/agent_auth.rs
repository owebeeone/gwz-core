//! Private signing bridge for prepared, explicitly trusted sessions. Unix only
//! until allocator and handle primitives are qualified in the deferred batch.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::{
                agent_client::{Agent, Channel},
                agent_job::Control,
                ssh_connection::SshConnection,
            };
            use libssh2_sys::{LIBSSH2_ERROR_AUTHENTICATION_FAILED, LIBSSH2_ERROR_EAGAIN, LIBSSH2_SESSION};
            use std::{
                ffi::{CString, c_char, c_int, c_void},
                io,
                panic::{AssertUnwindSafe, catch_unwind},
                ptr, slice,
                sync::Arc,
            };
            type Sign = unsafe extern "C" fn(
                *mut LIBSSH2_SESSION,
                *mut *mut u8,
                *mut usize,
                *const u8,
                usize,
                *mut *mut c_void,
            ) -> c_int;
            unsafe extern "C" {
                fn libssh2_userauth_publickey(
                    session: *mut LIBSSH2_SESSION,
                    user: *const c_char,
                    key: *const u8,
                    key_len: usize,
                    sign: Sign,
                    context: *mut *mut c_void,
                ) -> c_int;
            }
            /// Takes sole ownership of a handshaken session created by SshConnection.
            /// `trusted_key` is endpoint policy's independently approved host key.
            /// `open` must use owned bounded agent I/O (normally agent_socket::connect).
            pub(crate) fn authenticate<C: Channel>(
                connection: SshConnection,
                user: &str,
                trusted_key: &[u8],
                control: Arc<Control>,
                open: impl FnOnce() -> io::Result<Agent<C>>,
            ) -> io::Result<SshConnection> {
                authenticate_inner(connection, user, trusted_key, control, open, |_, _| {})
            }
            fn authenticate_inner<C: Channel>(
                mut connection: SshConnection,
                user: &str,
                trusted_key: &[u8],
                control: Arc<Control>,
                open: impl FnOnce() -> io::Result<Agent<C>>,
                mut observe: impl FnMut(&[u8], c_int),
            ) -> io::Result<SshConnection> {
                control.check()?;
                let user = CString::new(user).map_err(|_| io::ErrorKind::InvalidInput)?;
                if user.as_bytes().is_empty() || user.as_bytes().len() > 1024 {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                if trusted_key.is_empty()
                    || connection.session().host_key().map(|(key, _)| key) != Some(trusted_key)
                {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                connection.set_nonblocking()?;
                let mut agent = open()?;
                let keys = agent.identities()?;
                for key in keys {
                    control.check()?;
                    let mut signer = Signer {
                        agent: &mut agent,
                        key: &key,
                        user: user.as_bytes(),
                        control: &control,
                        invoked: false,
                        error: None,
                    };
                    // This stack owner and its key remain stable across every native EAGAIN.
                    let mut context = (&mut signer as *mut Signer<'_, C>).cast::<c_void>();
                    loop {
                        control.check()?;
                        let rc = {
                            let mut session = connection.session().raw();
                            // SAFETY: exclusive session guard, NUL-terminated user, bounded key,
                            // live callback state. Native API retains no callback after return.
                            unsafe {
                                libssh2_userauth_publickey(
                                    &mut *session,
                                    user.as_ptr(),
                                    key.as_ptr(),
                                    key.len(),
                                    sign::<C>,
                                    &mut context,
                                )
                            }
                        };
                        observe(&key, rc);
                        control.check()?;
                        if let Some(error) = signer.error {
                            return Err(error.into());
                        }
                        if rc == 0 {
                            if !signer.invoked || !connection.session().authenticated() {
                                return Err(io::ErrorKind::PermissionDenied.into());
                            }
                            drop(agent); // Agent handle and callback state cannot cross the handoff.
                            control.check()?;
                            return Ok(connection);
                        }
                        if rc == LIBSSH2_ERROR_EAGAIN {
                            // Bounded polling fallback: no hidden blocking native agent calls.
                            std::thread::sleep(control.quantum()?);
                        } else if rc == LIBSSH2_ERROR_AUTHENTICATION_FAILED {
                            break; // Server rejected this key; attempt the next listed identity once.
                        } else {
                            // PUBLICKEY_UNVERIFIED also hides packet/transport failures.
                            // Its origin is lost: terminate, never offer another identity.
                            return Err(io::ErrorKind::Other.into());
                        }
                    }
                }
                Err(io::ErrorKind::PermissionDenied.into())
            }
            cfg_if::cfg_if! {
                if #[cfg(test)] {
                    pub(crate) fn observed_authenticate<C: Channel>(
                        connection: SshConnection, user: &str, trusted_key: &[u8], control: Arc<Control>,
                        open: impl FnOnce() -> io::Result<Agent<C>>, observe: impl FnMut(&[u8], c_int),
                    ) -> io::Result<SshConnection> {
                        authenticate_inner(connection, user, trusted_key, control, open, observe)
                    }
                }
            }
            struct Signer<'a, C> {
                agent: &'a mut Agent<C>,
                key: &'a [u8],
                user: &'a [u8],
                control: &'a Control,
                invoked: bool,
                error: Option<io::ErrorKind>,
            }
            unsafe extern "C" fn sign<C: Channel>(
                _: *mut LIBSSH2_SESSION,
                output: *mut *mut u8,
                output_len: *mut usize,
                data: *const u8,
                len: usize,
                context: *mut *mut c_void,
            ) -> c_int {
                // SAFETY: pointers supplied by the pinned native API and our live Signer.
                let signer = unsafe { &mut *((*context).cast::<Signer<'_, C>>()) };
                let result = catch_unwind(AssertUnwindSafe(|| -> io::Result<Vec<u8>> {
                    signer.control.check()?;
                    if signer.invoked || len == 0 || len > 65536 {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    signer.invoked = true;
                    // SAFETY: native buffer lives for this callback; length bounded above.
                    let bytes = unsafe { slice::from_raw_parts(data, len) };
                    let method = method(bytes, signer.user, signer.key)?;
                    if !matches!(method, "ssh-ed25519" | "rsa-sha2-256" | "rsa-sha2-512") {
                        return Err(io::ErrorKind::Unsupported.into());
                    }
                    let signature = signer.agent.sign(signer.key, bytes, method)?;
                    shape(method, signer.key, &signature)?;
                    signer.control.check()?;
                    Ok(signature)
                }))
                .unwrap_or_else(|_| Err(io::ErrorKind::Other.into()));
                match result {
                    Ok(signature) => {
                        // Session::new uses libssh2's default malloc/free (session.c).
                        // Unix libc is the same allocator; Windows is deliberately unadmitted.
                        // Never transfer a Rust Vec allocation into native ownership.
                        let memory = unsafe { libc::malloc(signature.len()) }.cast::<u8>();
                        if memory.is_null() {
                            signer.error = Some(io::ErrorKind::OutOfMemory);
                            return -1;
                        }
                        // SAFETY: allocation has exact capacity; output slots are native-owned.
                        unsafe {
                            ptr::copy_nonoverlapping(signature.as_ptr(), memory, signature.len());
                            *output = memory;
                            *output_len = signature.len();
                        }
                        0 // libssh2 frees this signature on its success and failure paths.
                    }
                    Err(error) => {
                        signer.error = Some(error.kind());
                        -1
                    }
                }
                // Never return ALGO_UNSUPPORTED: libssh2 would retry with the key's
                // default algorithm, potentially downgrading RSA to SHA-1.
            }
            fn shape(method: &str, mut key: &[u8], signature: &[u8]) -> io::Result<()> {
                let kind = field(&mut key)?;
                let valid = match method {
                    "ssh-ed25519" => {
                        kind == b"ssh-ed25519"
                            && field(&mut key)?.len() == 32
                            && key.is_empty()
                            && signature.len() == 64
                    }
                    "rsa-sha2-256" | "rsa-sha2-512" => {
                        if kind != b"ssh-rsa" || field(&mut key)?.is_empty() {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                        let modulus = field(&mut key)?;
                        let modulus = modulus.strip_prefix(&[0]).unwrap_or(modulus);
                        key.is_empty() && !modulus.is_empty() && signature.len() == modulus.len()
                    }
                    _ => return Err(io::ErrorKind::Unsupported.into()),
                };
                if valid {
                    Ok(())
                } else {
                    Err(io::ErrorKind::InvalidData.into())
                }
            }
            fn field<'a>(input: &mut &'a [u8]) -> io::Result<&'a [u8]> {
                if input.len() < 4 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let len = u32::from_be_bytes(
                    input[..4]
                        .try_into()
                        .map_err(|_| io::ErrorKind::InvalidData)?,
                ) as usize;
                *input = &input[4..];
                if len > input.len() {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let (value, rest) = input.split_at(len);
                *input = rest;
                Ok(value)
            }
            fn method<'a>(mut input: &'a [u8], user: &[u8], key: &[u8]) -> io::Result<&'a str> {
                let _session_id = field(&mut input)?;
                if input.first() != Some(&50) {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                input = &input[1..];
                if field(&mut input)? != user
                    || field(&mut input)? != b"ssh-connection"
                    || field(&mut input)? != b"publickey"
                    || input.first() != Some(&1)
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                input = &input[1..];
                let method =
                    std::str::from_utf8(field(&mut input)?).map_err(|_| io::ErrorKind::InvalidData)?;
                if field(&mut input)? != key || !input.is_empty() {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Ok(method)
            }
        }
        pub(crate) use unix::authenticate;
        cfg_if::cfg_if! {
            if #[cfg(test)] { pub(crate) use unix::observed_authenticate; }
        }
    }
}
