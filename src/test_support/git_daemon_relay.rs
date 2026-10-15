//! A relay that stands between a client and a `git-daemon`, so that a client sees what the daemon sent.
//!
//! Git for Windows' `git-daemon` ends every session with a reset (RST) instead of a close, right after the last
//! byte it sends. Windows throws away what a reset finds unread in the receiving socket, so a client that is busy
//! between two reads (libgit2 writes each object to disk before it reads the next packet) loses the end of the
//! pack, and its next read fails with "error receiving data from socket: An existing connection was forcibly
//! closed by the remote host". The loss is a matter of timing: about 1 clone in 85 with every core busy, and every
//! clone by a client that pauses 20 ms between reads. The daemon does this with and without `--verbose` and
//! `--reuseaddr`; the daemon on macOS closes the way a server should.
//!
//! The relay reads the daemon's side without pause, so the reset finds nothing unread there, and ends the
//! client's side with an ordinary close once everything has been delivered. It touches no bytes. It belongs to the
//! daemon's guard and stops with it: when the relay is dropped its port is closed and every connection it held is
//! ended.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::fixture_listener;

/// How long the accepting thread sleeps when no client is waiting.
const POLL: Duration = Duration::from_millis(2);

/// A listener on a loopback port of the system's choosing that forwards each connection to `target`.
pub(crate) struct Relay {
    port: u16,
    stop: Arc<AtomicBool>,
    /// A handle on each socket the relay opened or accepted, to end them when the relay is dropped.
    held: Arc<Mutex<Vec<TcpStream>>>,
    /// What each connection did, for a test that failed (see `report`).
    events: Arc<Mutex<Vec<String>>>,
    accepting: Option<JoinHandle<Vec<JoinHandle<()>>>>,
}

impl Relay {
    /// Listen on a port of its own (one of `fixture_port`'s range, which the system gives to no one when the relay
    /// lets it go) and forward each connection to `127.0.0.1:target`. The listener is bound before this returns, so
    /// the port is the relay's and nothing else's.
    pub(crate) fn start(target: u16) -> Self {
        let listener = fixture_listener();
        listener
            .set_nonblocking(true)
            .expect("the relay's listener is polled");
        let port = listener
            .local_addr()
            .expect("the relay has an address")
            .port();
        let stop = Arc::new(AtomicBool::new(false));
        let held = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let accepting = {
            let stop = stop.clone();
            let held = held.clone();
            let events = events.clone();
            thread::spawn(move || accept_until_stopped(&listener, target, &stop, &held, &events))
        };
        Self {
            port,
            stop,
            held,
            events,
            accepting: Some(accepting),
        }
    }

    /// How each connection ended so far: the bytes carried each way and whether the side ended with a close or an
    /// error, with the time since the relay started. A failed test prints it beside the daemon's log.
    fn report(&self) -> String {
        self.events.lock().unwrap().join("\n")
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Relay {
    /// Stop the relay, as dropping it does, and say how each connection ended.
    pub(crate) fn finish(mut self) -> String {
        self.shut();
        self.report()
    }

    fn shut(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accepting thread owns the listener: its end closes the port, and it hands back the threads to join.
        let pumps = self
            .accepting
            .take()
            .map(|accepting| accepting.join().unwrap_or_default())
            .unwrap_or_default();
        for socket in self.held.lock().unwrap().drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
        for pump in pumps {
            let _ = pump.join();
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shut();
    }
}

fn accept_until_stopped(
    listener: &TcpListener,
    target: u16,
    stop: &AtomicBool,
    held: &Mutex<Vec<TcpStream>>,
    events: &Arc<Mutex<Vec<String>>>,
) -> Vec<JoinHandle<()>> {
    let mut pumps = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                pumps.extend(forward(client, target, held, events));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(POLL);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => {
                break;
            }
        }
    }
    pumps
}

/// Connect `client` to the daemon and start the two threads that carry the bytes. None if the daemon cannot be
/// reached: the client's connection is then closed.
fn forward(
    client: TcpStream,
    target: u16,
    held: &Mutex<Vec<TcpStream>>,
    events: &Arc<Mutex<Vec<String>>>,
) -> Vec<JoinHandle<()>> {
    // A socket accepted from a non-blocking listener is non-blocking on some systems.
    if client.set_nonblocking(false).is_err() {
        return Vec::new();
    }
    let Ok(daemon) = TcpStream::connect(("127.0.0.1", target)) else {
        return Vec::new();
    };
    let (Ok(client_reader), Ok(daemon_writer), Ok(daemon_reader), Ok(client_writer)) = (
        client.try_clone(),
        daemon.try_clone(),
        daemon.try_clone(),
        client.try_clone(),
    ) else {
        return Vec::new();
    };
    held.lock().unwrap().extend([client, daemon]);
    let (to_daemon, to_client) = (events.clone(), events.clone());
    vec![
        thread::spawn(move || pump(client_reader, daemon_writer, "client to daemon", &to_daemon)),
        thread::spawn(move || pump(daemon_reader, client_writer, "daemon to client", &to_client)),
    ]
}

/// What the reading thread has taken that the writing thread has not yet written.
#[derive(Default)]
struct Queue {
    bytes: Vec<u8>,
    /// The reading side has ended.
    done: bool,
    /// The writing side has failed; nothing more is wanted.
    abandoned: bool,
}

/// How long the writing thread waits for bytes before it looks again.
const WRITER_WAIT: Duration = Duration::from_millis(1);

/// Carry bytes from `from` to `to` until `from` ends or fails (a reset ends it like a close does), then end `to`'s
/// sending side, which the other side sees as a close after all the bytes.
///
/// `from` is read by a thread of its own that never waits for `to`, and does nothing between two reads but append
/// to a buffer: a reset that finds bytes unread throws them away (Windows does), and the daemon sends its last
/// bytes a moment before it resets. A client that is slow to read fills its socket, and a relay that stopped
/// reading the daemon to wait for it would lose the same bytes. What the client has not taken yet waits in memory.
fn pump(mut from: TcpStream, mut to: TcpStream, name: &'static str, events: &Mutex<Vec<String>>) {
    let started = Instant::now();
    let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
    let reading = queue.clone();
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 64 * 1024];
        let (mut reads, mut bytes) = (0usize, 0usize);
        let ended = loop {
            match from.read(&mut buffer) {
                Ok(0) => {
                    break "closed".to_owned();
                }
                Ok(count) => {
                    reads += 1;
                    bytes += count;
                    let mut waiting = reading.0.lock().unwrap();
                    if waiting.abandoned {
                        break "the writing side had gone".to_owned();
                    }
                    waiting.bytes.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => {
                    break format!("failed: {error}");
                }
            }
        };
        reading.0.lock().unwrap().done = true;
        reading.1.notify_all();
        (reads, bytes, ended)
    });
    loop {
        let (taken, finished) = {
            let mut waiting = queue.0.lock().unwrap();
            if waiting.bytes.is_empty() && !waiting.done {
                waiting = queue.1.wait_timeout(waiting, WRITER_WAIT).unwrap().0;
            }
            (std::mem::take(&mut waiting.bytes), waiting.done)
        };
        if !taken.is_empty() && to.write_all(&taken).is_err() {
            queue.0.lock().unwrap().abandoned = true;
            break;
        }
        if finished && taken.is_empty() {
            break;
        }
    }
    let _ = to.shutdown(Shutdown::Write);
    if let Ok((reads, bytes, ended)) = reader.join() {
        events.lock().unwrap().push(format!(
            "{name}: {bytes} bytes in {reads} reads, then {ended}, after {:?}",
            started.elapsed()
        ));
    }
}
