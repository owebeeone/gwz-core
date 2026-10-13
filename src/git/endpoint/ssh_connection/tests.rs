//! `terminate` on sockets whose peer has already ended the connection.
use super::*;
use std::net::TcpListener;

/// A connected pair on loopback: the client wrapped, the server's end returned.
fn connection() -> (SshConnection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    (SshConnection::new(client).unwrap(), server)
}

fn failing_with(kind: io::ErrorKind) -> fn(&TcpStream) -> io::Result<()> {
    match kind {
        io::ErrorKind::NotConnected => |_| Err(io::ErrorKind::NotConnected.into()),
        io::ErrorKind::ConnectionReset => |_| Err(io::ErrorKind::ConnectionReset.into()),
        io::ErrorKind::ConnectionAborted => |_| Err(io::ErrorKind::ConnectionAborted.into()),
        _ => |_| Err(io::ErrorKind::PermissionDenied.into()),
    }
}

#[test]
fn a_socket_the_peer_already_ended_is_terminated() {
    for kind in [
        io::ErrorKind::NotConnected,
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::ConnectionAborted,
    ] {
        let (connection, _server) = connection();
        let connection = connection.with_shutdown(failing_with(kind));
        assert!(connection.terminate().is_ok(), "{kind:?}");
    }
}

#[test]
fn any_other_failure_of_the_shutdown_is_reported() {
    let (connection, _server) = connection();
    let connection = connection.with_shutdown(failing_with(io::ErrorKind::PermissionDenied));
    assert_eq!(
        connection.terminate().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn a_live_socket_is_shut_down_and_a_second_shutdown_is_not_an_error() {
    let (connection, mut server) = connection();
    connection.terminate().unwrap();
    // The peer reads the end of the stream.
    let mut buffer = [0; 1];
    assert_eq!(io::Read::read(&mut server, &mut buffer).unwrap(), 0);
    // Whatever the platform reports for a socket shut down twice, the termination stands.
    connection.terminate().unwrap();
}

/// A pair whose server end is dropped with a linger of zero, which sends an RST: the client's socket is reset.
fn reset_connection() -> SshConnection {
    let (connection, server) = connection();
    socket2::SockRef::from(&server)
        .set_linger(Some(std::time::Duration::ZERO))
        .unwrap();
    drop(server);
    connection
}

/// State review P2-5 on real sockets: `terminate` of a connection the peer has reset is `Ok` in each state the reset
/// leaves it in. (Unix reports not connected for some of them; the Windows host used to qualify the transport reports
/// success for all four, so this row does not tell the fix from its absence there: the injected rows above do.)
#[test]
fn a_socket_the_peer_reset_is_terminated_in_every_state_the_reset_leaves_it_in() {
    let pause = || std::thread::sleep(std::time::Duration::from_millis(100));
    // Unread: nothing has looked at the reset yet.
    let unread = reset_connection();
    pause();
    unread.terminate().unwrap();
    // Read: the read has reported the reset (or the end of the stream, where the platform shows that).
    let read = reset_connection();
    pause();
    let mut buffer = [0; 1];
    let mut socket = read.watch_socket().unwrap();
    let outcome = io::Read::read(&mut socket, &mut buffer);
    assert!(
        matches!(&outcome, Ok(0))
            || outcome
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::ConnectionReset),
        "{outcome:?}"
    );
    read.terminate().unwrap();
    read.terminate().unwrap();
    // Written: a write to the reset connection has failed (the first may be accepted, a later one is not).
    let written = reset_connection();
    pause();
    let mut socket = written.watch_socket().unwrap();
    let failure = (0..20).find_map(|_| {
        pause();
        io::Write::write(&mut socket, b"x").err()
    });
    assert!(failure.is_some(), "a write to a reset connection must fail");
    written.terminate().unwrap();
    // Taken: the pending socket error has been read, as a `take_error` poll does.
    let taken = reset_connection();
    pause();
    let _ = taken.watch_socket().unwrap().take_error();
    taken.terminate().unwrap();
}
