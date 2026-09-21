//! Unix socket adapter; platform qualification and production activation remain deferred.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::{agent_client::{Agent, Channel}, agent_job::Control};
            use socket2::{Domain, SockAddr, Socket, Type};
            use std::{io::{self, Read, Write}, os::fd::AsRawFd, path::Path, sync::Arc};
            pub(crate) struct AgentSocket(Socket);
            impl Read for AgentSocket { fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> { (&self.0).read(bytes) } }
            impl Write for AgentSocket {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { (&self.0).write(bytes) }
                fn flush(&mut self) -> io::Result<()> { Ok(()) }
            }
            impl Channel for AgentSocket {
                fn wait(&mut self, writing: bool, control: &Control) -> io::Result<()> {
                    let duration = control.quantum()?;
                    let timeout = duration.as_millis().min(20) as i32;
                    let mut fd = libc::pollfd { fd: self.0.as_raw_fd(), events: if writing { libc::POLLOUT } else { libc::POLLIN }, revents: 0 };
                    // SAFETY: one initialized pollfd lives throughout this bounded call.
                    let result = unsafe { libc::poll(&mut fd, 1, timeout) };
                    control.check()?;
                    if result < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::Interrupted { return Err(error.kind().into()); }
                    }
                    Ok(())
                }
            }
            pub(crate) fn connect(path: &Path, control: Arc<Control>) -> io::Result<Agent<AgentSocket>> {
                control.check()?;
                let address = SockAddr::unix(path).map_err(|e| io::Error::from(e.kind()))?;
                let mut socket = AgentSocket(Socket::new(Domain::UNIX, Type::STREAM, None)?);
                socket.0.set_nonblocking(true)?;
                loop {
                    control.check()?;
                    match socket.0.connect(&address) {
                        Ok(()) => break,
                        Err(e) if e.raw_os_error() == Some(libc::EISCONN) => break,
                        Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) || e.raw_os_error() == Some(libc::EALREADY) => {
                            socket.wait(true, &control)?;
                            if let Some(e) = socket.0.take_error()? { return Err(e.kind().into()); }
                            if socket.0.peer_addr().is_ok() { break; }
                        }
                        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {
                            // Unix backlog exhaustion may not initiate a connection.
                            // Avoid a writable socket producing a tight connect retry.
                            std::thread::sleep(control.quantum()?);
                        }
                        Err(e) => return Err(e.kind().into()),
                    }
                }
                control.check()?; Ok(Agent::new(socket, control))
            }
        }
        pub(crate) use unix::connect;
    }
}
