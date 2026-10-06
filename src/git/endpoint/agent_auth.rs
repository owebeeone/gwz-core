//! Private signing bridge for prepared, explicitly trusted sessions. Unix only
//! until allocator and handle primitives are qualified in the deferred batch.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::{
                agent_client::{Agent, Channel},
                agent_job::Control,
                agent_keys::{self, KeyType, Signed},
                ssh_connection::SshConnection,
                ssh_network,
            };
            use libssh2_sys::{
                LIBSSH2_ERROR_ALGO_UNSUPPORTED, LIBSSH2_ERROR_AUTHENTICATION_FAILED, LIBSSH2_ERROR_EAGAIN,
                LIBSSH2_ERROR_METHOD_NONE, LIBSSH2_SESSION,
            };
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
            /// `offered` runs as each key is offered, `rejected` as the server refuses one.
            pub(crate) fn authenticate_reporting<C: Channel>(
                connection: SshConnection, user: &str, trusted_key: &[u8], control: Arc<Control>,
                open: impl FnOnce() -> io::Result<Agent<C>>, offered: impl FnMut(),
                mut rejected: impl FnMut(),
            ) -> io::Result<SshConnection> {
                authenticate_inner(connection, user, trusted_key, control, open,
                    |_, rc| { if rc == LIBSSH2_ERROR_AUTHENTICATION_FAILED { rejected(); } }, offered)
            }
            fn authenticate_inner<C: Channel>(
                mut connection: SshConnection,
                user: &str,
                trusted_key: &[u8],
                control: Arc<Control>,
                open: impl FnOnce() -> io::Result<Agent<C>>,
                mut observe: impl FnMut(&[u8], c_int),
                mut offered: impl FnMut(),
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
                control.begin_wait()?;
                let keys = agent.identities()?;
                control.complete_wait()?;
                control.check()?;
                for key in keys {
                    control.check()?;
                    // A key of a type outside TR2.8's list is never offered.
                    let Some(kind) = KeyType::of(&key) else {
                        continue;
                    };
                    let mut signer = Signer {
                        agent: &mut agent,
                        key: &key,
                        kind,
                        user: user.as_bytes(),
                        control: &control,
                        invoked: 0,
                        downgraded: false,
                        refused: false,
                        error: None,
                    };
                    // This stack owner and its key remain stable across every native EAGAIN.
                    let mut context = (&mut signer as *mut Signer<'_, C>).cast::<c_void>();
                    loop {
                        control.begin_wait()?;
                        offered();
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
                        if let Some(error) = signer.error.take() {
                            return Err(error);
                        }
                        if rc == 0 {
                            if signer.invoked == 0 || !connection.session().authenticated() {
                                return Err(io::ErrorKind::PermissionDenied.into());
                            }
                            drop(agent); // Agent handle and callback state cannot cross the handoff.
                            control.complete_wait()?;
                            control.check()?;
                            return Ok(connection);
                        }
                        if rc == LIBSSH2_ERROR_EAGAIN {
                            // A wait on the server's reply, which ends when the
                            // session's socket is ready, within the control's quantum,
                            // stall and aggregate bounds. The agent's own waits are in `sign`.
                            ssh_network::wait_session(&mut connection, &control)?;
                        } else if rc == LIBSSH2_ERROR_AUTHENTICATION_FAILED {
                            break; // Server rejected this key; attempt the next listed identity once.
                        } else if signer.refused || rc == LIBSSH2_ERROR_METHOD_NONE {
                            // The agent declined to sign after the server
                            // accepted the query, or libssh2 found no algorithm
                            // the server lists for this key and sent nothing.
                            // Either way the session awaits a new request: this
                            // key fails and the next is tried, as in 1.0.17.
                            // After METHOD_NONE libssh2 keeps this key's method,
                            // so later keys fail too
                            // (dev-docs/GwzTransportSshKeyTypes.md §3).
                            break;
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
                        authenticate_inner(connection, user, trusted_key, control, open, observe, || {})
                    }
                }
            }
            struct Signer<'a, C> {
                agent: &'a mut Agent<C>,
                key: &'a [u8],
                kind: KeyType,
                user: &'a [u8],
                control: &'a Control,
                /// Signatures libssh2 asked for: one per offer of this key.
                invoked: u8,
                /// The agent answered a `rsa-sha2-*` request with `ssh-rsa`,
                /// which allows libssh2 one more offer, as `ssh-rsa`.
                downgraded: bool,
                /// The agent declined to sign.
                refused: bool,
                error: Option<io::Error>,
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
                let result = catch_unwind(AssertUnwindSafe(|| -> io::Result<Option<Signed>> {
                    signer.control.begin_wait()?;
                    if signer.invoked > u8::from(signer.downgraded) || len == 0 || len > 65536 {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    signer.invoked += 1;
                    // SAFETY: native buffer lives for this callback; length bounded above.
                    let bytes = unsafe { slice::from_raw_parts(data, len) };
                    let method = method(bytes, signer.user, signer.key)?;
                    let algorithm = signer.kind.algorithm(method)?;
                    let reply = signer.agent.sign(signer.key, bytes, agent_keys::flags(algorithm))?;
                    signer.control.complete_wait()?;
                    let Some(reply) = reply else {
                        return Ok(None);
                    };
                    let signed = signer.kind.signature(signer.key, method, algorithm, &reply)?;
                    signer.control.check()?;
                    Ok(Some(signed))
                }))
                .unwrap_or_else(|_| Err(io::ErrorKind::Other.into()));
                match result {
                    Ok(Some(Signed::Signature(signature))) => {
                        // Session::new uses libssh2's default malloc/free (session.c).
                        // Unix libc is the same allocator; Windows is deliberately unadmitted.
                        // Never transfer a Rust Vec allocation into native ownership.
                        let memory = unsafe { libc::malloc(signature.len()) }.cast::<u8>();
                        if memory.is_null() {
                            signer.error = Some(io::Error::from(io::ErrorKind::OutOfMemory));
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
                    // RFC 8332's fallback, once per key, exactly as 1.0.17's
                    // libssh2 takes it (dev-docs/GwzTransportSshKeyTypes.md
                    // §3, case 3): libssh2 offers this key again as ssh-rsa.
                    // No other answer returns ALGO_UNSUPPORTED, which would
                    // retry under the key's default algorithm.
                    Ok(Some(Signed::Downgraded)) if !signer.downgraded => {
                        signer.downgraded = true;
                        LIBSSH2_ERROR_ALGO_UNSUPPORTED
                    }
                    Ok(Some(Signed::Downgraded)) => {
                        signer.error = Some(io::ErrorKind::InvalidData.into());
                        -1
                    }
                    Ok(None) => {
                        signer.refused = true;
                        -1
                    }
                    Err(error) => {
                        signer.error = Some(error);
                        -1
                    }
                }
            }
            fn method<'a>(mut input: &'a [u8], user: &[u8], key: &[u8]) -> io::Result<&'a str> {
                use agent_keys::field;
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
        pub(crate) use unix::authenticate_reporting;
        cfg_if::cfg_if! {
            if #[cfg(test)] { pub(crate) use unix::observed_authenticate; }
        }
    }
}
