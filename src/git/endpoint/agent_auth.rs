//! Private signing bridge for prepared, explicitly trusted sessions. It signs through the endpoint's own agent
//! client on every platform; the signature buffer's allocator is the one libssh2 frees with, proved on Windows for
//! plan step 3.4 (`allocate`).
//!
//! It also holds the reuse proof of plan step 1.6 (contract §7.1; dev-docs/GwzConnectionReuseDesign.md §4): before an
//! idle connection another operation binding last used serves this one, the endpoint's agent signs OpenSSH SSHSIG
//! signed data (PROTOCOL.sshsig) for a fresh nonce with the key the connection authenticated with, and only a
//! signature that verifies against that key lets the lease proceed. The signed bytes are exactly
//! `"SSHSIG" || string(namespace) || string("") || string("sha512") || string(SHA-512(nonce))`, each `string` a
//! 32-bit big-endian length and its bytes, under the fixed namespace `gwz-connection-reuse-v1@owebeeone.github.io`
//! ([`PROOF_NAMESPACE`]); RSA keys are asked for `rsa-sha2-512`. The 32-byte nonce is drawn from the operating
//! system's random source inside the proof job, on that job's stack, and is never stored, logged, sent anywhere but
//! (as its digest) to the agent, or put in an error.

use super::{
    agent_client::{Agent, Channel},
    agent_job::{self, Control, Job, Place, Supervisor},
    agent_keys::{self, KeyType, Proof, Signed},
    agent_source::AgentSource,
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
    path::PathBuf,
    ptr, slice,
    sync::Arc,
    time::{Duration, Instant},
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
    connection: SshConnection,
    user: &str,
    trusted_key: &[u8],
    control: Arc<Control>,
    open: impl FnOnce() -> io::Result<Agent<C>>,
    offered: impl FnMut(),
    rejected: impl FnMut(),
) -> io::Result<SshConnection> {
    authenticate_recorded(
        connection,
        user,
        trusted_key,
        control,
        open,
        offered,
        rejected,
    )
    .map(|(connection, _)| connection)
}
/// [`authenticate_reporting`], with the public blob of the agent key the server accepted, which the connection keeps
/// for its reuse by another binding (dev-docs/GwzConnectionReuseDesign.md §4 and C8).
pub(crate) fn authenticate_recorded<C: Channel>(
    connection: SshConnection,
    user: &str,
    trusted_key: &[u8],
    control: Arc<Control>,
    open: impl FnOnce() -> io::Result<Agent<C>>,
    offered: impl FnMut(),
    mut rejected: impl FnMut(),
) -> io::Result<(SshConnection, Vec<u8>)> {
    authenticate_inner(
        connection,
        user,
        trusted_key,
        control,
        open,
        |_, rc| {
            if rc == LIBSSH2_ERROR_AUTHENTICATION_FAILED {
                rejected();
            }
        },
        offered,
    )
}
fn authenticate_inner<C: Channel>(
    mut connection: SshConnection,
    user: &str,
    trusted_key: &[u8],
    control: Arc<Control>,
    open: impl FnOnce() -> io::Result<Agent<C>>,
    mut observe: impl FnMut(&[u8], c_int),
    mut offered: impl FnMut(),
) -> io::Result<(SshConnection, Vec<u8>)> {
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
                return Ok((connection, key.clone()));
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
        mod tests;

        pub(crate) fn observed_authenticate<C: Channel>(
            connection: SshConnection, user: &str, trusted_key: &[u8], control: Arc<Control>,
            open: impl FnOnce() -> io::Result<Agent<C>>, observe: impl FnMut(&[u8], c_int),
        ) -> io::Result<SshConnection> {
            authenticate_inner(connection, user, trusted_key, control, open, observe, || {})
                .map(|(connection, _)| connection)
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
        let reply = signer
            .agent
            .sign(signer.key, bytes, agent_keys::flags(algorithm))?;
        signer.control.complete_wait()?;
        let Some(reply) = reply else {
            return Ok(None);
        };
        let signed = signer
            .kind
            .signature(signer.key, method, algorithm, &reply)?;
        signer.control.check()?;
        Ok(Some(signed))
    }))
    .unwrap_or_else(|_| Err(io::ErrorKind::Other.into()));
    match result {
        Ok(Some(Signed::Signature(signature))) => {
            // Never transfer a Rust Vec allocation into native ownership: the buffer comes from `allocate`.
            let memory = allocate(signature.len());
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
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A buffer of `len` bytes from the allocator that libssh2 frees the signature with. `Session::new` leaves
        /// libssh2 its default allocator (session.c), the C library's `malloc` and `free`, and a Unix process has one
        /// C library: `libc::malloc` is it.
        fn allocate(len: usize) -> *mut u8 {
            // SAFETY: `malloc` takes any size and returns a block or null.
            unsafe { libc::malloc(len) }.cast()
        }
    } else if #[cfg(windows)] {
        /// A buffer of `len` bytes from the allocator that libssh2 frees the signature with. `Session::new` leaves
        /// libssh2 its default allocator (session.c), the C library's `malloc` and `free`. A Windows process can hold
        /// several C runtimes, each with its own heap, and a block freed by another one is a heap corruption; so this
        /// call is the one that step 3.4 proves, for the runtime that libssh2 is built against and the one `libc`
        /// links, in the debug and the static-runtime builds (evidence run `2026-10-12-sshsmall`, label
        /// `sshsmall-20261012-crt`).
        fn allocate(len: usize) -> *mut u8 {
            // SAFETY: `malloc` takes any size and returns a block or null.
            unsafe { libc::malloc(len) }.cast()
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
    let method = std::str::from_utf8(field(&mut input)?).map_err(|_| io::ErrorKind::InvalidData)?;
    if field(&mut input)? != key || !input.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(method)
}

/// The SSHSIG namespace of gwz's reuse proof, in the `name@domain` form PROTOCOL.sshsig recommends for a private use.
/// It is fixed: a signature made for it cannot verify under another namespace, and none made for another can stand in
/// for this one.
pub(crate) const PROOF_NAMESPACE: &str = "gwz-connection-reuse-v1@owebeeone.github.io";

/// The data an agent signs for a reuse proof: OpenSSH's SSHSIG signed data (PROTOCOL.sshsig) for a message whose
/// SHA-512 is `digest`: the `SSHSIG` preamble, the namespace, an empty reserved string, `sha512` and the digest. It
/// begins with the six preamble bytes, where SSH user authentication data begins with the session identifier's length
/// (`method`), so neither signature can stand for the other.
pub(crate) fn proof_data(digest: &[u8; 64]) -> Vec<u8> {
    let mut data = b"SSHSIG".to_vec();
    for part in [PROOF_NAMESPACE.as_bytes(), b"", b"sha512", digest] {
        data.extend_from_slice(&(part.len() as u32).to_be_bytes());
        data.extend_from_slice(part);
    }
    data
}

/// What the agent showed of its possession of a key (dev-docs/GwzConnectionReuseDesign.md §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Possession {
    /// It signed a fresh nonce's proof data, and the signature verifies against the key.
    Proven,
    /// It refused, lacks the key, signed badly, or could not be reached.
    Refused,
    /// The transport cannot check a signature of this key: no other binding can prove it.
    Unprovable,
}

/// Asks `agent` to sign the proof data of a fresh 32-byte nonce with `key`, and checks the signature against `key`.
/// The nonce and the data live on this call's stack only: they are never stored, logged or put in an error. An error
/// is the control's (a timeout or a cancellation) or the agent channel's; the caller decides which it was.
pub(crate) fn prove_possession<C: Channel>(
    agent: &mut Agent<C>,
    key: &[u8],
    control: &Control,
) -> io::Result<Possession> {
    use sha2::{Digest, Sha512};
    let Some(kind) = KeyType::of(key) else {
        return Ok(Possession::Unprovable);
    };
    if !kind.provable(key) {
        return Ok(Possession::Unprovable);
    }
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::from(io::ErrorKind::Other))?;
    let digest: [u8; 64] = Sha512::digest(nonce).into();
    let data = proof_data(&digest);
    control.begin_wait()?;
    let reply = agent.sign(key, &data, agent_keys::flags(kind.proof_algorithm()))?;
    control.complete_wait()?;
    control.check()?;
    Ok(match reply {
        None => Possession::Refused,
        Some(reply) => match kind.verify_proof(key, &data, &reply) {
            Proof::Valid => Possession::Proven,
            Proof::Invalid => Possession::Refused,
            Proof::Unsupported => Possession::Unprovable,
        },
    })
}

/// What authenticated a connection, which it keeps for its reuse by another binding (contract §7.1; reuse design §4,
/// C8). Public data only: a key's public blob, or the registry token of a selected key's exact bytes. A connection a
/// password authenticated (a URL's or a helper's) keeps none: no other binding can prove it, so it serves only the
/// binding that last used it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Authenticator {
    /// The agent key the server accepted.
    Agent(Vec<u8>),
    /// A selected key file, by the token of the snapshot that authenticated it.
    KeyFile(String),
}

/// What a connection keeps for its reuse by another binding: what authenticated it and the host key its trust check
/// accepted, and the endpoint's own agent and `known_hosts` to prove them against.
pub(crate) struct Evidence {
    pub(crate) authenticator: Authenticator,
    pub(crate) host_key: Vec<u8>,
    pub(crate) authority: Arc<ReuseAuthority>,
}

/// The agent and `known_hosts` of one endpoint, which every binding it serves shares by configuration equality
/// (reuse design §3), and the job budget its proofs run on.
pub(crate) struct ReuseAuthority {
    agent: AgentSource,
    known_hosts: PathBuf,
    supervisor: Supervisor,
    origin: Instant,
    cleanup: Duration,
}

/// The checks a lease by another binding needs: possession of the agent key `possession`, and trust of `host_key`
/// for the host's `names` at `port`.
pub(crate) struct Check {
    pub(crate) possession: Option<Vec<u8>>,
    pub(crate) trust: Option<(Vec<String>, u16, Vec<u8>)>,
}

/// The outcome of a [`Check`]: each part asked, as it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub(crate) possession: Option<Possession>,
    pub(crate) trusted: Option<bool>,
}

impl ReuseAuthority {
    pub(crate) fn new(
        agent: AgentSource,
        known_hosts: PathBuf,
        supervisor: Supervisor,
        origin: Instant,
        cleanup: Duration,
    ) -> Self {
        Self {
            agent,
            known_hosts,
            supervisor,
            origin,
            cleanup,
        }
    }

    /// Starts `check` as a supervised job on the endpoint's job budget, under `deadline` (milliseconds on the
    /// endpoint's clock) and the stall `stall_ms`, as an authentication sign wait runs (reuse design §4). A full
    /// budget is `WouldBlock`.
    pub(crate) fn start(
        self: &Arc<Self>,
        check: Check,
        deadline: Option<u64>,
        stall_ms: u64,
    ) -> io::Result<Job<Verdict>> {
        let authority = self.clone();
        let aggregate = deadline.map(|at| self.origin + Duration::from_millis(at));
        Job::start_timed(
            Place::Take(&self.supervisor),
            aggregate,
            Duration::from_millis(stall_ms),
            self.cleanup,
            agent_job::wall_clock(),
            move |control| authority.run(check, &control),
        )
    }

    fn run(&self, check: Check, control: &Arc<Control>) -> io::Result<Verdict> {
        // A failure of the control (a deadline, a stall, a cancellation) ends the job as that; any other failure of
        // a part is that part's negative answer: an agent that cannot be reached holds no key for this binding.
        let settle = |result: io::Result<()>| control.check().and(Ok(result.is_ok()));
        let possession = match check.possession {
            None => None,
            Some(key) => Some(match self.agent.open(control.clone()) {
                Ok(mut agent) => match prove_possession(&mut agent, &key, control) {
                    Ok(possession) => possession,
                    Err(error) => {
                        settle(Err(error))?;
                        Possession::Refused
                    }
                },
                Err(error) => {
                    settle(Err(error))?;
                    Possession::Refused
                }
            }),
        };
        let trusted = match check.trust {
            None => None,
            Some((names, port, host_key)) => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                Some(settle(ssh_network::recheck_trust(
                    &self.known_hosts,
                    &names,
                    port,
                    &host_key,
                    control,
                ))?)
            }
        };
        Ok(Verdict {
            possession,
            trusted,
        })
    }
}
