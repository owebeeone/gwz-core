//! The proxy's `arm_existing` and `freeze_existing` mark the bytes the client sends from then on, and not the bytes
//! it had already sent, which a loaded machine's forwarding thread may not have read yet (the
//! `idle_loss::the_retry_after_a_dead_lease...` flake: a session's last packet, sent before the test armed the links,
//! was read after it and ended the "untouched" session).
use super::CutProxy;
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

/// How many times a row repeats the send-then-mark sequence: the forwarding thread loses the race often, not always.
const ROUNDS: usize = 20;

/// A client and the server end of its link through a proxy that has marked nothing.
struct Link {
    proxy: CutProxy,
    client: TcpStream,
    server: TcpStream,
    _listener: TcpListener,
}

impl Link {
    fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let proxy = CutProxy::start(listener.local_addr().unwrap().port());
        let client = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        let (server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // The proxy has connected to the server; the link is registered a moment later.
        while proxy.connections() == 0 {
            thread::sleep(Duration::from_millis(1));
        }
        Self {
            proxy,
            client,
            server,
            _listener: listener,
        }
    }

    /// What the server end reads next, up to `want` bytes, and whether it saw the link end.
    fn read(&mut self, want: usize) -> (Vec<u8>, bool) {
        let mut bytes = Vec::new();
        let mut buffer = [0; 64];
        while bytes.len() < want {
            match self.server.read(&mut buffer) {
                Ok(0) => {
                    return (bytes, true);
                }
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                }
                Err(_) => {
                    return (bytes, false);
                }
            }
        }
        (bytes, false)
    }
}

#[test]
fn bytes_sent_before_arm_existing_reach_the_server_and_the_next_bytes_end_the_link() {
    for round in 0..ROUNDS {
        let mut link = Link::new();
        link.client.write_all(b"before").unwrap();
        link.proxy.arm_existing();
        let (bytes, ended) = link.read(6);
        assert_eq!(
            (bytes.as_slice(), ended),
            (&b"before"[..], false),
            "round {round}: what the client sent before the arm"
        );
        link.client.write_all(b"after").unwrap();
        let (bytes, ended) = link.read(1);
        assert_eq!(
            (bytes.as_slice(), ended),
            (&b""[..], true),
            "round {round}: the first bytes after the arm end the link, unforwarded"
        );
    }
}

#[test]
fn bytes_sent_before_freeze_existing_reach_the_server_and_the_next_bytes_are_dropped() {
    for round in 0..ROUNDS {
        let mut link = Link::new();
        link.client.write_all(b"before").unwrap();
        link.proxy.freeze_existing();
        let (bytes, ended) = link.read(6);
        assert_eq!(
            (bytes.as_slice(), ended),
            (&b"before"[..], false),
            "round {round}: what the client sent before the freeze"
        );
        link.client.write_all(b"after").unwrap();
        link.server
            .set_read_timeout(Some(Duration::from_millis(30)))
            .unwrap();
        let (bytes, ended) = link.read(1);
        assert_eq!(
            (bytes.as_slice(), ended),
            (&b""[..], false),
            "round {round}: the bytes after the freeze are dropped and the link stays open"
        );
    }
}

#[test]
fn marking_links_that_have_ended_does_not_wait_for_them() {
    let mut link = Link::new();
    link.proxy.cut_all();
    let (_, ended) = link.read(1);
    assert!(ended, "the cut ended the link");
    let started = Instant::now();
    link.proxy.arm_existing();
    link.proxy.freeze_existing();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "marking an ended link returns at once"
    );
}

/// Waits until every link's client read has taken the fault it was given.
fn wait_for_the_fault_to_be_taken(proxy: &CutProxy) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while proxy
        .links
        .lock()
        .unwrap()
        .iter()
        .any(|link| link.gate.fault.lock().unwrap().is_some())
    {
        assert!(Instant::now() < deadline, "the read never took its fault");
        thread::sleep(Duration::from_millis(1));
    }
    // The read that took it has been answered; the link either goes on reading or has ended.
    thread::sleep(Duration::from_millis(50));
}

/// A client read that fails and is tried again, as these do not end a link; the bytes the client then sends arrive.
fn the_link_goes_on_after(error: impl Fn() -> io::Error) {
    for round in 0..ROUNDS {
        let mut link = Link::new();
        link.proxy.fail_next_client_read(&error);
        wait_for_the_fault_to_be_taken(&link.proxy);
        link.client.write_all(b"still here").unwrap();
        let (bytes, ended) = link.read(10);
        assert_eq!(
            (bytes.as_slice(), ended),
            (&b"still here"[..], false),
            "round {round}: the link after the failed read"
        );
    }
}

#[test]
fn an_interrupted_read_does_not_end_the_link() {
    the_link_goes_on_after(|| io::Error::from(io::ErrorKind::Interrupted));
}

#[test]
fn a_reset_connection_ends_the_link() {
    let mut link = Link::new();
    link.proxy
        .fail_next_client_read(|| io::Error::from(io::ErrorKind::ConnectionReset));
    let (bytes, ended) = link.read(1);
    assert_eq!(
        (bytes.as_slice(), ended),
        (&b""[..], true),
        "a reset client ends the link, and the server sees it end"
    );
}

/// A segment the client has sent can still be on its way when the link is marked (loopback delivers through a queue of
/// its own), so a mark holds only after the socket has stayed empty for more than one read of 20 ms.
#[test]
fn a_mark_waits_for_the_socket_to_stay_empty_and_not_for_one_empty_read() {
    for round in 0..ROUNDS {
        let link = Link::new();
        let started = Instant::now();
        link.proxy.arm_existing();
        assert!(
            started.elapsed() >= Duration::from_millis(40),
            "round {round}: the arm held after {:?}",
            started.elapsed()
        );
    }
}
