//! The fixture's dialer: an authenticated, nonblocking SSH session through a port that may be a proxy in front of the
//! fixture's server, retried where libssh2 on Windows (or the server) fails by itself, with the retried connections
//! counted so that a test that counts connections can tell them from sessions.
use super::{HOST_KEY_MISMATCH, SshConnection};
use ssh2::{CheckResult, KnownHostFileKind};
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// An authenticated, nonblocking session through `port`, which may be a proxy in front of the server that listens
/// on `server_port`: the host key checked is that server's own, from `known_hosts`, and the client authenticates
/// as `user` with the key file `key`.
pub(crate) fn dialer(
    port: u16,
    server_port: u16,
    user: String,
    known_hosts: PathBuf,
    key: PathBuf,
) -> impl Fn() -> io::Result<SshConnection> + Send + Sync + 'static {
    dialer_counting(
        port,
        server_port,
        user,
        known_hosts,
        key,
        DialRetries::default(),
    )
}

/// [`dialer`], whose retried connections are counted (and, if `retries` says so, injected) as [`DialRetries`]
/// describes.
pub(crate) fn dialer_counting(
    port: u16,
    server_port: u16,
    user: String,
    known_hosts: PathBuf,
    key: PathBuf,
    retries: DialRetries,
) -> impl Fn() -> io::Result<SshConnection> + Send + Sync + 'static {
    move || {
        let mut attempt = 1;
        loop {
            let dialed = dial_once(
                port,
                server_port,
                &user,
                &known_hosts,
                &key,
                &retries,
                attempt,
            );
            match &dialed {
                Err(error) if attempt < DIAL_ATTEMPTS && key_rejected(error) => {
                    retries.abandon();
                    attempt += 1;
                }
                Err(error) => {
                    // A dial that fails is reported to the row that dialed it by a setup failure that says nothing
                    // of why.
                    eprintln!(
                        "fixture dial through port {port} failed: {error} ({:?})",
                        error.kind()
                    );
                    return dialed;
                }
                Ok(_) => {
                    return dialed;
                }
            }
        }
    }
}

/// How many connections a dial makes before it gives up on a key the server refuses.
const DIAL_ATTEMPTS: usize = 4;

/// The server accepted the fixture's key and then refused the signature made with it (libssh2's
/// `PUBLICKEY_UNVERIFIED`; a key the server does not authorize is `AUTHENTICATION_FAILED` instead, and final). The
/// fixture signs with a key that is authorized, so a signature that fails is not the answer to a wrong key. Windows
/// hosts have shown it about once in a thousand dials, under load (`sshd.exe` 9.8 and libssh2 on CNG, hosted-like
/// runs of the `idle_loss` rows).
#[derive(Debug)]
struct KeyRejected(ssh2::Error);

impl std::fmt::Display for KeyRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the server refused the fixture's key: {}",
            self.0
        )
    }
}

impl std::error::Error for KeyRejected {}

/// Whether `error` is a [`KeyRejected`].
fn key_rejected(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|source| source.is::<KeyRejected>())
}

/// One dial of [`dialer_counting`], its `attempt`th: a handshaken connection whose host key is the server's own and
/// which has authenticated as `user`, nonblocking. `retries` may fail the first connection's key exchange or refuse its
/// key, so that the dial connects again.
fn dial_once(
    port: u16,
    server_port: u16,
    user: &str,
    known_hosts: &Path,
    key: &Path,
    retries: &DialRetries,
    attempt: usize,
) -> io::Result<SshConnection> {
    let (mut connection, _) = handshaken_counting(
        port,
        5_000,
        retries,
        retries.fail_key_exchanges && attempt == 1,
    )?;
    {
        let session = connection.session();
        let (host_key, _) = session
            .host_key()
            .ok_or_else(|| io::Error::other("sshd did not provide a host key"))?;
        let host_key = host_key.to_owned();
        let mut known = session.known_hosts()?;
        known.read_file(known_hosts, KnownHostFileKind::OpenSSH)?;
        if !matches!(
            known.check_port("127.0.0.1", server_port, &host_key),
            CheckResult::Match
        ) {
            return Err(io::Error::other(HOST_KEY_MISMATCH));
        }
        let refusal = ssh2::ErrorCode::Session(libssh2_sys::LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED);
        let outcome = if retries.refuse_keys && attempt == 1 {
            Err(ssh2::Error::from_errno(refusal))
        } else {
            session.userauth_pubkey_file(user, None, key, None)
        };
        match outcome {
            Ok(()) => {}
            Err(error) if error.code() == refusal => {
                return Err(io::Error::other(KeyRejected(error)));
            }
            Err(error) => {
                return Err(error.into());
            }
        }
        if !session.authenticated() {
            return Err(io::Error::other("temporary SSH authentication failed"));
        }
    }
    connection.set_nonblocking()?;
    Ok(connection)
}

/// How many times [`handshaken`] connects before it gives up.
const HANDSHAKE_ATTEMPTS: usize = 6;

/// The connections a dialer abandoned and replaced, which a test that counts the connections its server (or a proxy in
/// front of it) accepted cannot tell from the sessions it asked for. libssh2 on Windows CNG ends about one key
/// exchange in a hundred on its own side (see [`handshaken`]), and the dial connects afresh; a server has refused the
/// fixture's own key about one dial in a thousand ([`KeyRejected`]), and the dial connects afresh again. A proxy then
/// counts more connections than the sessions dialed. Clones share one count.
#[derive(Clone, Default)]
pub(crate) struct DialRetries {
    abandoned: Arc<AtomicUsize>,
    /// The first connection of each dial fails its key exchange, as libssh2's can, which a Unix host never does by
    /// itself: a seam for the rows that count connections.
    fail_key_exchanges: bool,
    /// The first connection of each dial has its key refused after its key exchange, as a server's can, which a
    /// Unix host never does by itself either.
    refuse_keys: bool,
}

impl DialRetries {
    /// Counts the retries that happen, and makes the first connection of each dial fail its key exchange, so that each
    /// session takes two connections.
    pub(crate) fn failing_key_exchanges() -> Self {
        Self {
            fail_key_exchanges: true,
            ..Self::default()
        }
    }

    /// Counts the retries that happen, and makes the first connection of each dial have its key refused, so that each
    /// session takes two connections.
    pub(crate) fn refusing_keys() -> Self {
        Self {
            refuse_keys: true,
            ..Self::default()
        }
    }

    /// These retries, and the first connection of each dial has its key refused as well: with failing key
    /// exchanges, each session takes three connections.
    pub(crate) fn and_refusing_keys(self) -> Self {
        Self {
            refuse_keys: true,
            ..self
        }
    }

    /// The sessions among `accepted` connections a server or proxy has counted: all but the ones abandoned. Call it
    /// when no dial is in flight: a dial that has connected and not yet given up on its connection is a connection
    /// that is not abandoned here yet.
    pub(crate) fn sessions_in(&self, accepted: usize) -> usize {
        accepted.saturating_sub(self.abandoned.load(Ordering::Acquire))
    }

    /// The connections the dialer has abandoned and replaced so far: the ones a test forced and any key exchange
    /// libssh2 on Windows CNG ended by itself.
    pub(crate) fn abandoned(&self) -> usize {
        self.abandoned.load(Ordering::Acquire)
    }

    fn abandon(&self) {
        self.abandoned.fetch_add(1, Ordering::AcqRel);
    }
}

/// Connects to `port` and completes the SSH handshake within `timeout_ms`, with a duplicate of the socket for
/// the caller to watch or break. libssh2 on Windows CNG sometimes ends a key exchange on its own side, about one
/// connection in a hundred, whatever the server, key exchange method or host key algorithm (the server's log shows
/// it finished its half and then saw the client close; `Session(-8)`, "Unable to exchange encryption keys"), so a
/// failure with that code connects afresh; any other failure, and the last attempt's, is returned.
pub(crate) fn handshaken(port: u16, timeout_ms: u32) -> io::Result<(SshConnection, TcpStream)> {
    handshaken_counting(port, timeout_ms, &DialRetries::default(), false)
}

/// [`handshaken`], counting each connection it abandons for a key exchange failure in `retries`. `fail_first` ends
/// the first key exchange (the connection is closed before the handshake, as the server sees a client that gave up).
pub(crate) fn handshaken_counting(
    port: u16,
    timeout_ms: u32,
    retries: &DialRetries,
    fail_first: bool,
) -> io::Result<(SshConnection, TcpStream)> {
    let mut attempt = 1;
    loop {
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        let watch = stream.try_clone()?;
        let mut connection = SshConnection::new(stream)?;
        connection.session().set_timeout(timeout_ms);
        let outcome = if attempt == 1 && fail_first {
            Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(-8)))
        } else {
            connection.session().handshake()
        };
        match outcome {
            Ok(()) => return Ok((connection, watch)),
            Err(error)
                if attempt < HANDSHAKE_ATTEMPTS && error.code() == ssh2::ErrorCode::Session(-8) =>
            {
                retries.abandon();
                attempt += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
