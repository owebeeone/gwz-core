//! Unix network/trust setup, owned by an existing supervised Job.
use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::{agent_job::Control, ssh_connection::SshConnection};
            use gwz_transport::pool::Key;
            use socket2::{Domain, SockAddr, Socket, Type};
            use ssh2::{BlockDirections, CheckResult, KnownHostFileKind, MethodType};
            use std::{
                fs::OpenOptions,
                io::{self, Read},
                net::{SocketAddr, TcpStream, ToSocketAddrs},
                os::fd::AsRawFd,
                os::unix::fs::OpenOptionsExt,
                path::Path,
            };

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

            pub(crate) fn establish(
                key: &Key,
                known_hosts: &Path,
                control: &Control,
            ) -> io::Result<(SshConnection, Vec<u8>)> {
                establish_inner(key, known_hosts, control, read_regular, resolve)
            }

            pub(crate) fn establish_inner<L, R>(
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
                validate_key(key)?;
                control.check()?;
                let text = load(known_path, control)?;
                control.check()?;
                validate_lines(&text, control)?;
                control.check()?;
                let mut addresses = resolve_addresses(&key.host, key.port, control)?;
                control.check()?;
                addresses.truncate(ADDRESS_CAP);
                if addresses.is_empty() {
                    return Err(io::ErrorKind::NotFound.into());
                }
                let mut last = None;
                for address in addresses {
                    control.check()?;
                    let socket = match connect(address, control) {
                        Ok(socket) => socket,
                        Err(error) if terminal(&error) => return Err(error),
                        Err(error) => {
                            last = Some(error);
                            continue;
                        }
                    };
                    return handshake(socket, &key.host, key.port, &text, control);
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

            pub(crate) fn read_regular(path: &Path, control: &Control) -> io::Result<String> {
                control.check()?;
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)
                    .map_err(clean)?;
                control.check()?;
                let metadata = file.metadata().map_err(clean)?;
                control.check()?;
                if !metadata.file_type().is_file() {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                control.check()?;
                let mut bytes = Vec::new();
                file.take((FILE_CAP + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(clean)?;
                control.check()?;
                if bytes.len() > FILE_CAP {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidInput.into())
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
                control.check()?;
                let result = (host, port).to_socket_addrs().map_err(clean)?;
                let addresses: Vec<_> = result.take(ADDRESS_CAP).collect();
                control.check()?;
                Ok(addresses)
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
                match socket.connect(&SockAddr::from(address)) {
                    Ok(()) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) || error.raw_os_error() == Some(libc::EINPROGRESS)
                            || error.raw_os_error() == Some(libc::EALREADY) =>
                    {
                        loop {
                            wait_socket(&socket, control)?;
                            if let Some(error) = socket.take_error().map_err(clean)? {
                                return Err(clean(error));
                            }
                            if socket.peer_addr().is_ok() {
                                break;
                            }
                        }
                    }
                    Err(error) => return Err(clean(error)),
                }
                control.check()?;
                Ok(socket.into())
            }

            fn wait_socket(socket: &Socket, control: &Control) -> io::Result<()> {
                control.check()?;
                let timeout = control.quantum()?.as_millis().min(20) as i32;
                let mut poll = libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLOUT | libc::POLLERR | libc::POLLHUP,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut poll, 1, timeout) };
                control.check()?;
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(clean(error));
                    }
                }
                Ok(())
            }

            fn handshake(
                socket: TcpStream,
                host: &str,
                port: u16,
                text: &str,
                control: &Control,
            ) -> io::Result<(SshConnection, Vec<u8>)> {
                let mut connection = SshConnection::new(socket).map_err(clean)?;
                control.check()?;
                connection.set_nonblocking().map_err(clean)?;
                control.check()?;
                let mut known = connection.session().known_hosts().map_err(ssh)?;
                load_known(&mut known, text, None, control)?;
                let prefs = preferences(&mut connection, text, host, port, control)?;
                if !prefs.is_empty() {
                    control.check()?;
                    connection
                        .session()
                        .method_pref(MethodType::HostKey, &prefs)
                        .map_err(ssh)?;
                    control.check()?;
                }
                loop {
                    control.check()?;
                    match connection.session().handshake() {
                        Ok(()) => break,
                        Err(error) if error.code() == ssh2::ErrorCode::Session(-37) => {
                            wait_session(&mut connection, control)?;
                        }
                        Err(error) => return Err(ssh(error)),
                    }
                }
                control.check()?;
                let host_key = connection
                    .session()
                    .host_key()
                    .map(|(key, _)| key.to_vec())
                    .ok_or(io::ErrorKind::InvalidData)?;
                control.check()?;
                if !matches!(known.check_port(host, port, &host_key), CheckResult::Match) {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                Ok((connection, host_key))
            }

            fn wait_session(connection: &mut SshConnection, control: &Control) -> io::Result<()> {
                let events = match connection.session().block_directions() {
                    BlockDirections::Inbound => libc::POLLIN,
                    BlockDirections::Outbound => libc::POLLOUT,
                    BlockDirections::Both => libc::POLLIN | libc::POLLOUT,
                    BlockDirections::None => libc::POLLIN | libc::POLLOUT,
                };
                let timeout = control.quantum()?.as_millis().min(20) as i32;
                let mut poll = libc::pollfd {
                    fd: connection.session().as_raw_fd(),
                    events,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut poll, 1, timeout) };
                control.check()?;
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(clean(error));
                    }
                }
                Ok(())
            }

            fn preferences(
                connection: &mut SshConnection,
                text: &str,
                host: &str,
                port: u16,
                control: &Control,
            ) -> io::Result<String> {
                let mut prefs = String::new();
                for (kind, names) in HOSTKEYS {
                    control.check()?;
                    let mut set = connection.session().known_hosts().map_err(ssh)?;
                    if load_known(&mut set, text, Some(kind), control)?
                        && matches!(set.check_port(host, port, &[0]), CheckResult::Mismatch)
                    {
                        if !prefs.is_empty() {
                            prefs.push(',');
                        }
                        prefs.push_str(names);
                    }
                }
                Ok(prefs)
            }

            fn load_known(
                known: &mut ssh2::KnownHosts,
                text: &str,
                kind: Option<&str>,
                control: &Control,
            ) -> io::Result<bool> {
                let mut loaded = false;
                for raw in text.split('\n') {
                    control.check()?;
                    let line = raw.trim_end_matches('\r');
                    if line.trim_matches([' ', '\t']).is_empty()
                        || line.trim_start_matches([' ', '\t']).starts_with('#')
                    {
                        continue;
                    }
                    if line.len() > LINE_CAP {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                    if kind.is_some_and(|wanted| line_kind(line) != Some(wanted)) {
                        continue;
                    }
                    known
                        .read_str(line, KnownHostFileKind::OpenSSH)
                        .map_err(ssh)?;
                    loaded = true;
                    control.check()?;
                }
                Ok(loaded)
            }

            fn line_kind(line: &str) -> Option<&str> {
                // Match pinned hostline() tokenization and prefix recognition.
                let key_type = line.split([' ', '\t']).filter(|s| !s.is_empty()).nth(1)?;
                HOSTKEYS
                    .iter()
                    .find_map(|(kind, _)| kind.starts_with(key_type).then_some(*kind))
            }

            fn terminal(error: &io::Error) -> bool {
                matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted
                )
            }

            fn clean(error: io::Error) -> io::Error {
                io::Error::from(error.kind())
            }

            fn ssh(error: ssh2::Error) -> io::Error {
                clean(io::Error::from(error))
            }
        }
        pub(crate) use unix::establish;
        cfg_if! {
            if #[cfg(test)] {
                pub(crate) use unix::{establish_inner as establish_with, read_regular};
            }
        }
    }
}
