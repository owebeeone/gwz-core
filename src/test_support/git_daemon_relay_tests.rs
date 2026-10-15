//! What the relay promises: a client sees everything the daemon sent, and a graceful close, even when the daemon
//! ends the connection with a reset and the client reads late.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::git_daemon_relay::Relay;

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// A server that sends `payload` to its one client and, a moment later, closes with the client's byte unread, which
/// ends the connection with a reset, as Git for Windows' `git-daemon` ends every session. The moment is what a
/// reader that waits in `read` needs to take the payload before the reset comes; a reset that finds the payload
/// unread loses it (and on macOS, when the payload is larger than the socket buffers, even the reset).
fn aborting_server(payload: Vec<u8>) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        // Wait until the client's byte is here, and leave it unread.
        client.peek(&mut [0u8; 1]).unwrap();
        client.write_all(&payload).unwrap();
        thread::sleep(Duration::from_millis(100));
    });
    (port, server)
}

#[test]
fn a_late_reader_gets_all_that_an_aborting_server_sent_and_a_clean_end() {
    let sent = payload(100_000);
    let (server_port, server) = aborting_server(sent.clone());
    let relay = Relay::start(server_port);

    let mut client = TcpStream::connect(("127.0.0.1", relay.port())).unwrap();
    client.write_all(b"x").unwrap();
    // The server has sent everything and reset by now; a reader that was on the server's socket would have lost
    // what it had not read.
    thread::sleep(Duration::from_millis(300));
    let mut received = Vec::new();
    client
        .read_to_end(&mut received)
        .expect("the relay ends the stream cleanly");

    assert_eq!(received.len(), sent.len());
    assert!(received == sent, "the bytes differ");
    server.join().unwrap();
}

#[test]
fn what_the_client_sends_reaches_the_server_and_its_end_ends_the_server_side() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut received = Vec::new();
        client.read_to_end(&mut received).unwrap();
        client.write_all(b"reply").unwrap();
        received
    });
    let relay = Relay::start(server_port);

    let mut client = TcpStream::connect(("127.0.0.1", relay.port())).unwrap();
    client.write_all(&payload(100_000)).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).unwrap();

    assert_eq!(reply, b"reply");
    assert!(server.join().unwrap() == payload(100_000));
}

#[test]
fn a_relay_serves_one_connection_after_another() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut client, _) = listener.accept().unwrap();
            client.write_all(b"hello").unwrap();
        }
    });
    let relay = Relay::start(server_port);

    for _ in 0..2 {
        let mut text = String::new();
        TcpStream::connect(("127.0.0.1", relay.port()))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "hello");
    }
    server.join().unwrap();
}

// The relay's port is not checked for refusing connections: on macOS a socket is made close-on-exec after it is
// created, and a test that starts a process at that moment (the daemon tests do) hands it the listener, which then
// stays open until that process ends.
#[test]
fn dropping_the_relay_ends_a_connection_held_open() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let relay = Relay::start(server_port);

    let mut held = TcpStream::connect(("127.0.0.1", relay.port())).unwrap();
    let (_accepted, _) = listener.accept().unwrap();
    drop(relay);

    held.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let ended = held.read_to_end(&mut Vec::new());
    // An end of stream or a reset, not a timeout: the held connection did not outlive its relay.
    assert!(
        ended.is_ok()
            || ended.as_ref().is_err_and(|error| {
                error.kind() != ErrorKind::WouldBlock && error.kind() != ErrorKind::TimedOut
            }),
        "{ended:?}"
    );
}
