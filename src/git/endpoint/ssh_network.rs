//! Network/trust setup, owned by an existing supervised Job.
use super::{
    agent_job::Control,
    socket_wait::{self, Interest},
    ssh_connection::SshConnection,
    ssh_host_key::HostKeyRefused,
    ssh_limits::SshLimit,
};
use crate::git::regular_file;
use cfg_if::cfg_if;
use gwz_transport::pool::Key;
pub(crate) use kex_retry::Network;
use kex_retry::{KEX_FAILURE, Steps};
use socket2::{Domain, SockAddr, Socket, Type};
use ssh2::{BlockDirections, CheckResult, KnownHostFileKind, MethodType};
use std::{
    borrow::Cow,
    io::{self, Read},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::Path,
    time::Instant,
};

mod kex_retry;

const FILE_CAP: usize = 4 * 1024 * 1024;
const LINE_CAP: usize = 16 * 1024;
const ADDRESS_CAP: usize = 32;
const HOSTKEYS: &[(&str, &str)] = &[
    ("ssh-ed25519", "ssh-ed25519"),
    ("ecdsa-sha2-nistp256", "ecdsa-sha2-nistp256"),
    ("ecdsa-sha2-nistp384", "ecdsa-sha2-nistp384"),
    ("ecdsa-sha2-nistp521", "ecdsa-sha2-nistp521"),
    ("ssh-rsa", "rsa-sha2-512,rsa-sha2-256,ssh-rsa"),
];

/// Connects to `key`'s server and establishes trust in it, for
/// an open whose URL wrote the host as `written`, one of the key's
/// host's ASCII case variants (TR2.18). `credential_offered` says whether the
/// open's facts show a credential could already have been offered, which
/// forbids the one retry of a failed key exchange (D14).
pub(crate) fn establish_written(
    key: &Key,
    written: &str,
    known_hosts: &Path,
    control: &Control,
    credential_offered: &dyn Fn() -> bool,
) -> io::Result<(SshConnection, Vec<u8>)> {
    establish_inner(
        key,
        written,
        known_hosts,
        control,
        read_regular,
        resolve,
        &mut Network::production(credential_offered),
    )
}

pub(crate) fn establish_inner<L, R, S>(
    key: &Key,
    written: &str,
    known_path: &Path,
    control: &Control,
    load: L,
    resolve_addresses: R,
    network: &mut Network<'_, S>,
) -> io::Result<(SshConnection, Vec<u8>)>
where
    S: Steps,
    L: FnOnce(&Path, &Control) -> io::Result<String>,
    R: FnOnce(&str, u16, &Control) -> io::Result<Vec<SocketAddr>>,
{
    validate_key(key)?;
    // Trust is looked up under the key's host lowercased, as the
    // pool keys it, and under the host as the open's URL wrote it,
    // which a hashed known_hosts name may hash (TR2.18).
    let lowered = key.host.to_ascii_lowercase();
    if !written.eq_ignore_ascii_case(&lowered) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let both = [lowered.as_str(), written];
    let names = if written == lowered {
        &both[..1]
    } else {
        &both[..]
    };
    control.check()?;
    let text = match load(known_path, control) {
        // A missing `known_hosts` lists no host: it is the refusal of an unknown one (U8).
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(HostKeyRefused::NotListed.into_error());
        }
        loaded => loaded?,
    };
    control.check()?;
    validate_lines(&text, control)?;
    control.check()?;
    // A limit of the SSH library is met before any connection is made: it would only waste the server's start.
    let prefs = host_key_preferences(&text, names, key.port, control)?;
    control.check()?;
    let mut addresses = resolve_addresses(&key.host, key.port, control)?;
    control.check()?;
    addresses.truncate(ADDRESS_CAP);
    if addresses.is_empty() {
        return Err(io::ErrorKind::NotFound.into());
    }
    // What the setup has spent when a try began: a retry needs more budget left than a try took (D14).
    let started = control.mark();
    let mut retried = false;
    loop {
        let socket = connect_addresses(addresses.clone(), control, |address, control| {
            network.steps.connect(address, control)
        })?;
        match handshake(
            socket,
            names,
            key.port,
            &text,
            &prefs,
            control,
            &mut network.steps,
        ) {
            Ok(established) => return Ok(established),
            Err(Ended::KexFailure(error)) => {
                // A new TCP connection and a new session: the failed one was dropped with the handshake. The
                // handshake comes before every authentication step, so none has begun.
                if !network.may_retry(retried, control, started) {
                    return Err(error);
                }
                retried = true;
            }
            Err(Ended::Other(error)) => return Err(error),
        }
    }
}

pub(crate) fn connect_addresses<C>(
    addresses: Vec<SocketAddr>,
    control: &Control,
    mut connect_address: C,
) -> io::Result<TcpStream>
where
    C: FnMut(SocketAddr, &Control) -> io::Result<TcpStream>,
{
    let mut last = None;
    for address in addresses {
        control.check()?;
        let started = Instant::now();
        let socket = match connect_address(address, control) {
            Ok(socket) => {
                // The settle time follows this: the TCP connect to the
                // address that answered, not the setup that comes after.
                control.record_tcp_connect(started.elapsed());
                socket
            }
            Err(error) => {
                control.check()?;
                last = Some(error);
                continue;
            }
        };
        return Ok(socket);
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::NotFound.into()))
}

fn validate_key(key: &Key) -> io::Result<()> {
    if key.scheme != gwz_transport::protocol::Scheme::Ssh
        || key.port == 0
        || key.host.is_empty()
        || key.host.len() > 255
        || key
            .host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "@/?#\\".contains(c))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// `establish_written` for a URL that wrote the host as the key has it.
        pub(crate) fn establish(
            key: &Key,
            known_hosts: &Path,
            control: &Control,
        ) -> io::Result<(SshConnection, Vec<u8>)> {
            establish_written(key, &key.host, known_hosts, control, &|| false)
        }
    }
}

pub(crate) fn read_regular(path: &Path, control: &Control) -> io::Result<String> {
    control.check()?;
    // Refuses a FIFO, a device or a pipe without blocking, on every platform.
    let file = regular_file::open(path).map_err(clean)?;
    control.check()?;
    let mut bytes = Vec::new();
    file.take((FILE_CAP + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(clean)?;
    control.check()?;
    if bytes.len() > FILE_CAP {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // libssh2 reads `known_hosts` as text, which on Windows is not the file's bytes (`crt_text`).
    String::from_utf8(libssh2_text(bytes)).map_err(|_| io::ErrorKind::InvalidInput.into())
}

cfg_if! {
    if #[cfg(windows)] {
        /// A `known_hosts` file's bytes as libssh2 reads them: in text mode, through the C runtime.
        fn libssh2_text(bytes: Vec<u8>) -> Vec<u8> {
            super::ssh_limits::crt_text(&bytes).into_owned()
        }
    } else {
        /// A `known_hosts` file's bytes as libssh2 reads them.
        fn libssh2_text(bytes: Vec<u8>) -> Vec<u8> {
            bytes
        }
    }
}

fn validate_lines(text: &str, control: &Control) -> io::Result<()> {
    if text.len() > FILE_CAP {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if text.contains('\0') {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for line in text.split('\n') {
        control.check()?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.len() > LINE_CAP {
            return Err(io::ErrorKind::InvalidInput.into());
        }
    }
    Ok(())
}

fn resolve(host: &str, port: u16, control: &Control) -> io::Result<Vec<SocketAddr>> {
    let result = timed_resolution(control, || (host, port).to_socket_addrs().map_err(clean))?;
    let addresses: Vec<_> = result.take(ADDRESS_CAP).collect();
    control.check()?;
    Ok(addresses)
}

pub(crate) fn timed_resolution<T>(
    control: &Control,
    resolve: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    control.begin_wait()?;
    let result = resolve();
    control.check()?;
    let result = result?;
    control.complete_wait()?;
    control.check()?;
    Ok(result)
}

fn connect(address: SocketAddr, control: &Control) -> io::Result<TcpStream> {
    let domain = match address {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::STREAM, None).map_err(clean)?;
    control.check()?;
    socket.set_nonblocking(true).map_err(clean)?;
    control.check()?;
    control.begin_wait()?;
    let attempt = socket.connect(&SockAddr::from(address));
    control.check()?;
    match attempt {
        Ok(()) => {
            control.complete_wait()?;
        }
        Err(error) if socket_wait::connect_pending(&error) => loop {
            // The connect is over when the wait says so: `getpeername` answers for a socket that is still
            // connecting on Windows, so the peer address does not say it.
            if wait_socket(&socket, control)? {
                if let Some(error) = socket.take_error().map_err(clean)? {
                    return Err(clean(error));
                }
                control.complete_wait()?;
                break;
            }
        },
        Err(error) => return Err(clean(error)),
    }
    control.check()?;
    Ok(socket.into())
}

/// One slice of waiting for a connect's outcome: whether it came.
fn wait_socket(socket: &Socket, control: &Control) -> io::Result<bool> {
    let mut finished = false;
    wait_step(control, |duration| {
        finished = socket_wait::connect_wait(socket, duration).map_err(clean)?;
        Ok(finished)
    })?;
    Ok(finished)
}

pub(crate) fn wait_step(
    control: &Control,
    poll: impl FnOnce(std::time::Duration) -> io::Result<bool>,
) -> io::Result<()> {
    control.wait_step(poll)
}

/// How a handshake ended without a trusted connection.
enum Ended {
    /// libssh2's key exchange failure (-8), the one failure that D14 retries.
    KexFailure(io::Error),
    /// Any other end.
    Other(io::Error),
}

impl From<io::Error> for Ended {
    fn from(error: io::Error) -> Self {
        Self::Other(error)
    }
}

fn handshake(
    socket: TcpStream,
    names: &[&str],
    port: u16,
    text: &str,
    prefs: &str,
    control: &Control,
    steps: &mut impl Steps,
) -> Result<(SshConnection, Vec<u8>), Ended> {
    let mut connection = SshConnection::new(socket).map_err(clean)?;
    control.check()?;
    connection.set_nonblocking().map_err(clean)?;
    control.check()?;
    let mut known = connection.session().known_hosts().map_err(ssh)?;
    load_known(&mut known, text, None, control)?;
    if !prefs.is_empty() {
        control.check()?;
        connection
            .session()
            .method_pref(MethodType::HostKey, prefs)
            .map_err(ssh)?;
        control.check()?;
    }
    loop {
        control.begin_wait()?;
        let result = steps.handshake(connection.session());
        control.check()?;
        match result {
            Ok(()) => {
                control.complete_wait()?;
                control.check()?;
                break;
            }
            Err(error) if error.code() == ssh2::ErrorCode::Session(-37) => {
                wait_session(&mut connection, control)?;
            }
            Err(error) if error.code() == KEX_FAILURE => return Err(Ended::KexFailure(ssh(error))),
            Err(error) => return Err(ssh(error).into()),
        }
    }
    control.check()?;
    let host_key = connection
        .session()
        .host_key()
        .map(|(key, _)| key.to_vec())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    control.check()?;
    let mut changed = false;
    for name in names {
        match known.check_port(name, port, &host_key) {
            CheckResult::Match => return Ok((connection, host_key)),
            CheckResult::Mismatch => changed = true,
            CheckResult::NotFound | CheckResult::Failure => {}
        }
    }
    // Not listed, or listed with a different key (libssh2 tells them apart): the host words it with the file it read
    // (U8, decision D12).
    if !changed {
        return Err(HostKeyRefused::NotListed.into_error().into());
    }
    let line = changed_line(&mut connection, text, names, port, &host_key, control)?;
    Err(HostKeyRefused::Changed { line }.into_error().into())
}

/// The first line (from 1) of `text` that lists one of `names` with a key other than `host_key`: each line is read
/// into a set of its own and asked, as the whole file was. `None` when no line is found alone, as a line that only
/// matches with its neighbours would be.
fn changed_line(
    connection: &mut SshConnection,
    text: &str,
    names: &[&str],
    port: u16,
    host_key: &[u8],
    control: &Control,
) -> io::Result<Option<usize>> {
    for (index, line) in text.split('\n').enumerate() {
        control.check()?;
        let mut alone = connection.session().known_hosts().map_err(ssh)?;
        if !load_known(&mut alone, line, None, control)? {
            continue;
        }
        if names.iter().any(|name| {
            matches!(
                alone.check_port(name, port, host_key),
                CheckResult::Mismatch
            )
        }) {
            return Ok(Some(index + 1));
        }
    }
    Ok(None)
}

/// Waits within `control`'s bounds for the session's socket to be
/// ready in the directions libssh2 last blocked on.
pub(crate) fn wait_session(connection: &mut SshConnection, control: &Control) -> io::Result<()> {
    let directions = connection.session().block_directions();
    let session = &*connection.session();
    wait_step(control, |duration| {
        match directions {
            BlockDirections::Inbound => socket_wait::wait_readable(session, duration),
            BlockDirections::Outbound => socket_wait::wait_writable(session, duration),
            BlockDirections::Both | BlockDirections::None => {
                socket_wait::wait_for(session, Interest::BOTH, duration)
            }
        }
        .map_err(clean)
    })
}

/// The host-key preference for a connection to `names`: the algorithms of the kinds `known_hosts` has entries for,
/// as the session's library supports them. A session is made for the answer and never connected.
fn host_key_preferences(
    text: &str,
    names: &[&str],
    port: u16,
    control: &Control,
) -> io::Result<String> {
    let session = ssh2::Session::new().map_err(ssh)?;
    let supported = session.supported_algs(MethodType::HostKey).map_err(ssh)?;
    let mut present = Vec::new();
    for entry @ (kind, _) in HOSTKEYS {
        control.check()?;
        let mut set = session.known_hosts().map_err(ssh)?;
        if load_known(&mut set, text, Some(kind), control)?
            && names
                .iter()
                .any(|name| matches!(set.check_port(name, port, &[0]), CheckResult::Mismatch))
        {
            present.push(*entry);
        }
    }
    host_key_choice(&present, &supported)
}

/// The preference list for the kinds `known_hosts` holds entries for (`present`, each with its algorithms), as the
/// library `supported` supports them. Algorithms it lacks are left out, as libssh2 would strip them; when the kinds
/// present are all unsupported the open is refused (TD5), where 1.0.17 failed with `failed to set hostkey preference`.
fn host_key_choice(present: &[(&str, &str)], supported: &[&str]) -> io::Result<String> {
    let mut prefs = Vec::new();
    let mut refused = false;
    for (_, algorithms) in present {
        let kept: Vec<_> = algorithms
            .split(',')
            .filter(|algorithm| supported.contains(algorithm))
            .collect();
        if kept.is_empty() {
            refused = true;
        } else {
            prefs.extend(kept);
        }
    }
    if prefs.is_empty() && refused {
        return Err(SshLimit::HostKeys.into_error());
    }
    Ok(prefs.join(","))
}

fn load_known(
    known: &mut ssh2::KnownHosts,
    text: &str,
    kind: Option<&str>,
    control: &Control,
) -> io::Result<bool> {
    let mut loaded = false;
    // CR/LF exclusion measures admission; preserve every native token
    // byte but a plain host field's ASCII case, which `folded` drops.
    for line in text.split('\n') {
        control.check()?;
        if line.trim_matches([' ', '\t']).is_empty()
            || line.trim_start_matches([' ', '\t']).starts_with('#')
        {
            continue;
        }
        if kind.is_some_and(|wanted| line_kind(line) != Some(wanted)) {
            continue;
        }
        known
            .read_str(&folded(line), KnownHostFileKind::OpenSSH)
            .map_err(ssh)?;
        loaded = true;
        control.check()?;
    }
    Ok(loaded)
}

/// libssh2 compares a plain known_hosts name byte for byte, where
/// the transport matches it ignoring ASCII case (TR2.18). So a
/// plain line's host field, which libssh2's readline takes up to
/// the first space or tab after leading blanks, is lowercased
/// before libssh2 reads the line, and the host is looked up
/// lowercased. libssh2's hostline takes a field of more than two
/// bytes that does not start with `|1|` as plain names. Any other
/// field, a hashed name among them, stays as written: its hash is
/// of the name as it was written.
fn folded(line: &str) -> Cow<'_, str> {
    let start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let end = line[start..]
        .find([' ', '\t'])
        .map_or(line.len(), |at| start + at);
    let field = &line[start..end];
    if field.len() <= 2
        || field.starts_with("|1|")
        || !field.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Cow::Borrowed(line);
    }
    let mut folded = String::with_capacity(line.len());
    folded.push_str(&line[..start]);
    folded.push_str(&field.to_ascii_lowercase());
    folded.push_str(&line[end..]);
    Cow::Owned(folded)
}

fn line_kind(line: &str) -> Option<&str> {
    // Match pinned hostline() tokenization and prefix recognition.
    let key_type = line.split([' ', '\t']).filter(|s| !s.is_empty()).nth(1)?;
    HOSTKEYS
        .iter()
        .find_map(|(kind, _)| kind.starts_with(key_type).then_some(*kind))
}

fn clean(error: io::Error) -> io::Error {
    if super::agent_job::timeout_reason(&error).is_some() {
        return error;
    }
    // An abort the network stack reports (it carries an OS error number; the setup's own cancellation does not) is a
    // connection lost before authentication, which a retry may cure. Windows reports it where Unix reports a reset.
    if error.kind() == io::ErrorKind::ConnectionAborted && error.raw_os_error().is_some() {
        return io::Error::from(io::ErrorKind::ConnectionReset);
    }
    io::Error::from(error.kind())
}

fn ssh(error: ssh2::Error) -> io::Error {
    clean(io::Error::from(error))
}

cfg_if! {
    if #[cfg(test)] {
        mod tests;
        mod kex_retry_tests;

        /// `establish_inner` for a URL that wrote the host as the key has it.
        pub(crate) fn establish_with<L, R>(
            key: &Key,
            known_path: &Path,
            control: &Control,
            load: L,
            resolve_addresses: R,
        ) -> io::Result<(SshConnection, Vec<u8>)>
        where
            L: FnOnce(&Path, &Control) -> io::Result<String>,
            R: FnOnce(&str, u16, &Control) -> io::Result<Vec<SocketAddr>>,
        {
            establish_inner(
                key,
                &key.host,
                known_path,
                control,
                load,
                resolve_addresses,
                &mut Network::production(&|| false),
            )
        }
    }
}
