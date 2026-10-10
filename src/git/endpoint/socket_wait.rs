//! Bounded socket readiness, one call per supervised slice (GwzTransportWindowsParityPlan.md, step 1.2).
//!
//! Every wait in the SSH setup path is a closure that `Control::wait_step` hands at most one slice (20 ms) and
//! that answers "ready" or "not yet". The closures need one OS call: ask whether a socket can be read or written,
//! or whether a non-blocking connect has finished or failed. That call is `poll` on Unix and `select` on
//! Windows; nothing else in the setup path names the operating system.
//!
//! A wait never lasts longer than [`MAX_WAIT`], whatever timeout it is given, so a caller that passes a long
//! timeout still returns to its `Control` within a slice.
use cfg_if::cfg_if;
use std::{io, time::Duration};

/// The longest one wait lasts: the supervised slice.
pub(crate) const MAX_WAIT: Duration = Duration::from_millis(20);

/// What a wait is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Interest {
    pub(crate) read: bool,
    pub(crate) write: bool,
}

impl Interest {
    pub(crate) const READ: Self = Self {
        read: true,
        write: false,
    };
    pub(crate) const WRITE: Self = Self {
        read: false,
        write: true,
    };
    pub(crate) const BOTH: Self = Self {
        read: true,
        write: true,
    };
}

/// The two questions a socket is asked: whether it can be read or written, and whether a pending connect has an
/// outcome. A failed connect is reported as an error condition on some platforms and as writability on others,
/// so the connect question is its own.
#[derive(Clone, Copy)]
enum Question {
    Ready(Interest),
    Connected,
}

/// Waits up to `timeout` (at most [`MAX_WAIT`]) for `socket` to be readable: data, EOF or an error to report.
pub(crate) fn wait_readable(socket: &impl SocketHandle, timeout: Duration) -> io::Result<bool> {
    wait_for(socket, Interest::READ, timeout)
}

/// Waits up to `timeout` (at most [`MAX_WAIT`]) for `socket` to accept a write.
pub(crate) fn wait_writable(socket: &impl SocketHandle, timeout: Duration) -> io::Result<bool> {
    wait_for(socket, Interest::WRITE, timeout)
}

/// Waits for `interest`, up to `timeout` (at most [`MAX_WAIT`]).
pub(crate) fn wait_for(
    socket: &impl SocketHandle,
    interest: Interest,
    timeout: Duration,
) -> io::Result<bool> {
    ask(socket, Question::Ready(interest), timeout)
}

/// Waits up to `timeout` (at most [`MAX_WAIT`]) for a socket whose non-blocking connect is pending to finish
/// or fail. `true` means the connect has an outcome, which the caller reads with `take_error` and `peer_addr`.
pub(crate) fn connect_wait(socket: &impl SocketHandle, timeout: Duration) -> io::Result<bool> {
    ask(socket, Question::Connected, timeout)
}

fn ask(socket: &impl SocketHandle, question: Question, timeout: Duration) -> io::Result<bool> {
    sys::poll(socket.raw_handle(), question, timeout.min(MAX_WAIT))
}

/// Whether `error` from a non-blocking `connect` means the connect is under way.
pub(crate) fn connect_pending(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    ) || error.raw_os_error().is_some_and(sys::is_pending_code)
}

cfg_if! {
    if #[cfg(unix)] {
        use std::os::fd::{AsRawFd, RawFd};

        /// A socket the readiness calls can wait on, by its platform handle.
        pub(crate) trait SocketHandle {
            fn raw_handle(&self) -> RawFd;
        }

        impl<T: AsRawFd> SocketHandle for T {
            fn raw_handle(&self) -> RawFd {
                self.as_raw_fd()
            }
        }

        mod sys {
            use super::{Question, io};
            use std::{os::fd::RawFd, time::Duration};

            pub(super) fn is_pending_code(code: i32) -> bool {
                code == libc::EINPROGRESS || code == libc::EALREADY
            }

            pub(super) fn poll(fd: RawFd, question: Question, timeout: Duration) -> io::Result<bool> {
                let events = match question {
                    Question::Ready(interest) => {
                        let read = if interest.read { libc::POLLIN } else { 0 };
                        let write = if interest.write { libc::POLLOUT } else { 0 };
                        read | write
                    }
                    Question::Connected => libc::POLLOUT | libc::POLLERR | libc::POLLHUP,
                };
                let mut poll = libc::pollfd {
                    fd,
                    events,
                    revents: 0,
                };
                let result = unsafe { libc::poll(&mut poll, 1, timeout.as_millis() as i32) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                    return Ok(false);
                }
                Ok(result > 0 && (poll.revents & events) != 0)
            }
        }
    } else if #[cfg(windows)] {
        use std::os::windows::io::AsRawSocket;

        /// A socket the readiness calls can wait on, by its platform handle.
        pub(crate) trait SocketHandle {
            fn raw_handle(&self) -> usize;
        }

        impl<T: AsRawSocket> SocketHandle for T {
            fn raw_handle(&self) -> usize {
                self.as_raw_socket() as usize
            }
        }

        mod sys {
            use super::{Question, io};
            use std::time::Duration;
            use windows_sys::Win32::Networking::WinSock::{
                FD_SET, SOCKET_ERROR, TIMEVAL, WSAEALREADY, WSAEINPROGRESS, WSAEWOULDBLOCK,
                WSAGetLastError, select,
            };

            pub(super) fn is_pending_code(code: i32) -> bool {
                code == WSAEWOULDBLOCK || code == WSAEINPROGRESS || code == WSAEALREADY
            }

            fn one(socket: usize) -> FD_SET {
                let mut set = FD_SET {
                    fd_count: 1,
                    fd_array: [0; 64],
                };
                set.fd_array[0] = socket;
                set
            }

            /// `select` reports a failed non-blocking connect in its exception set, not its write set, and does
            /// so on every Windows version; `WSAPoll` is not used because it did not report it before Windows 10
            /// version 2004.
            pub(super) fn poll(socket: usize, question: Question, timeout: Duration) -> io::Result<bool> {
                let (read, write, except) = match question {
                    Question::Ready(interest) => (interest.read, interest.write, false),
                    Question::Connected => (false, true, true),
                };
                let (mut read_set, mut write_set, mut except_set) = (one(socket), one(socket), one(socket));
                let pick = |wanted: bool, set: &mut FD_SET| -> *mut FD_SET {
                    if wanted { set } else { std::ptr::null_mut() }
                };
                let wait = TIMEVAL {
                    tv_sec: timeout.as_secs() as i32,
                    tv_usec: timeout.subsec_micros() as i32,
                };
                let result = unsafe {
                    select(
                        0,
                        pick(read, &mut read_set),
                        pick(write, &mut write_set),
                        pick(except, &mut except_set),
                        &wait,
                    )
                };
                if result == SOCKET_ERROR {
                    return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
                }
                Ok(result > 0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use socket2::{Domain, SockAddr, Socket, Type};
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        time::Instant,
    };

    /// A connected loopback pair, client first.
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    /// Repeats a one-slice wait until it reports ready or `bound` passes.
    fn until(bound: Duration, mut wait: impl FnMut() -> io::Result<bool>) -> bool {
        let deadline = Instant::now() + bound;
        while Instant::now() < deadline {
            if wait().unwrap() {
                return true;
            }
        }
        false
    }

    /// A non-blocking socket with its connect to `address` started.
    fn connecting(address: SocketAddr) -> Socket {
        let socket = Socket::new(Domain::for_address(address), Type::STREAM, None).unwrap();
        socket.set_nonblocking(true).unwrap();
        match socket.connect(&SockAddr::from(address)) {
            Ok(()) => {}
            Err(error) => assert!(connect_pending(&error), "connect failed outright: {error}"),
        }
        socket
    }

    #[test]
    fn a_socket_is_readable_after_a_write_and_not_before() {
        let (client, mut server) = pair();
        let started = Instant::now();
        assert!(!wait_readable(&client, MAX_WAIT).unwrap());
        assert!(started.elapsed() >= Duration::from_millis(10));
        server.write_all(b"x").unwrap();
        assert!(until(Duration::from_secs(5), || wait_readable(
            &client, MAX_WAIT
        )));
    }

    #[test]
    fn a_socket_with_room_is_writable() {
        let (client, _server) = pair();
        assert!(until(Duration::from_secs(5), || wait_writable(
            &client, MAX_WAIT
        )));
    }

    #[test]
    fn a_closed_peer_is_readable_as_end_of_file() {
        let (mut client, server) = pair();
        drop(server);
        assert!(until(Duration::from_secs(5), || wait_readable(
            &client, MAX_WAIT
        )));
        let mut buffer = [0u8; 8];
        assert_eq!(client.read(&mut buffer).unwrap(), 0);
    }

    #[test]
    fn a_long_timeout_still_returns_within_one_slice() {
        let (client, _server) = pair();
        let started = Instant::now();
        assert!(!wait_readable(&client, Duration::from_secs(30)).unwrap());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_zero_timeout_asks_without_waiting() {
        let (client, _server) = pair();
        let started = Instant::now();
        assert!(!wait_readable(&client, Duration::ZERO).unwrap());
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn interest_in_both_directions_reports_either() {
        let (client, mut server) = pair();
        assert!(until(Duration::from_secs(5), || wait_for(
            &client,
            Interest::BOTH,
            MAX_WAIT
        )));
        server.write_all(b"x").unwrap();
        assert!(until(Duration::from_secs(5), || wait_for(
            &client,
            Interest::READ,
            MAX_WAIT
        )));
    }

    #[test]
    fn a_connect_to_a_listener_finishes_successfully() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let socket = connecting(listener.local_addr().unwrap());
        assert!(until(Duration::from_secs(5), || connect_wait(
            &socket, MAX_WAIT
        )));
        assert!(socket.take_error().unwrap().is_none());
        assert!(socket.peer_addr().is_ok());
    }

    #[test]
    fn a_connect_to_a_closed_port_ends_with_the_refusal() {
        let closed = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap()
        };
        let socket = connecting(closed);
        let started = Instant::now();
        // Windows retries a refused loopback connect for about two seconds before it reports it.
        assert!(
            until(Duration::from_secs(10), || connect_wait(&socket, MAX_WAIT)),
            "the failed connect was never reported"
        );
        let error = socket.take_error().unwrap().expect("the connect failed");
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_pending_connect_is_recognised_and_a_refusal_is_not() {
        assert!(connect_pending(&io::ErrorKind::WouldBlock.into()));
        assert!(connect_pending(&io::ErrorKind::Interrupted.into()));
        assert!(!connect_pending(&io::ErrorKind::ConnectionRefused.into()));
        assert!(!connect_pending(&io::ErrorKind::TimedOut.into()));
    }

    cfg_if! {
        if #[cfg(unix)] {
            #[test]
            fn the_unix_in_progress_codes_are_pending() {
                assert!(connect_pending(&io::Error::from_raw_os_error(libc::EINPROGRESS)));
                assert!(connect_pending(&io::Error::from_raw_os_error(libc::EALREADY)));
                assert!(!connect_pending(&io::Error::from_raw_os_error(libc::ECONNREFUSED)));
            }
        } else if #[cfg(windows)] {
            #[test]
            fn the_winsock_in_progress_codes_are_pending() {
                // WSAEWOULDBLOCK, WSAEINPROGRESS, WSAEALREADY; WSAECONNREFUSED is the refusal.
                for code in [10035, 10036, 10037] {
                    assert!(connect_pending(&io::Error::from_raw_os_error(code)));
                }
                assert!(!connect_pending(&io::Error::from_raw_os_error(10061)));
            }
        }
    }
}
