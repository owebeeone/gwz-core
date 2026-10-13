//! A loopback TCP proxy in front of a fixture server that can end its
//! connections from the server's side: at once, or when the client next sends
//! bytes on them, or by resetting them. It plays a server that closes an idle
//! connection, one that closes it just as the client starts an exchange on it,
//! and a peer or middlebox that sends a reset in the middle of one.
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

struct Link {
    client: TcpStream,
    server: TcpStream,
    /// Ends the link, unforwarded, when the client next sends bytes.
    armed: Arc<AtomicBool>,
    /// Drops the client's bytes and keeps the link open: a server that stalls.
    frozen: Arc<AtomicBool>,
    /// The link is being reset: its forwarding threads end without a shutdown, so that the last handle closed ends
    /// the connection with a reset and not a FIN.
    reset: Arc<AtomicBool>,
}
impl Link {
    fn cut(&self) {
        let _ = self.client.shutdown(Shutdown::Both);
        let _ = self.server.shutdown(Shutdown::Both);
    }

    /// Resets the link: both sockets close with a linger of zero once every handle to them is closed, which sends a
    /// reset (an RST, which the client reports as a reset connection) and no FIN.
    fn reset(self) {
        self.reset.store(true, Ordering::Release);
        for socket in [&self.client, &self.server] {
            let _ = socket2::SockRef::from(socket).set_linger(Some(Duration::ZERO));
        }
    }
}

pub(crate) struct CutProxy {
    pub(crate) port: u16,
    /// The listeners' addresses, which `drop` connects to so that each accept loop sees the stop.
    wake: Vec<SocketAddr>,
    links: Arc<Mutex<Vec<Link>>>,
    accepted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl CutProxy {
    pub(crate) fn start(target: u16) -> Self {
        let listeners = super::loopback::bind();
        let port = listeners[0].local_addr().unwrap().port();
        let wake = listeners
            .iter()
            .map(|listener| listener.local_addr().unwrap())
            .collect();
        let links = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        for listener in listeners {
            let (shared, count, stopping) = (links.clone(), accepted.clone(), stop.clone());
            thread::spawn(move || {
                for client in listener.incoming() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(client) = client else {
                        continue;
                    };
                    let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else {
                        continue;
                    };
                    count.fetch_add(1, Ordering::AcqRel);
                    let armed = Arc::new(AtomicBool::new(false));
                    let frozen = Arc::new(AtomicBool::new(false));
                    let reset = Arc::new(AtomicBool::new(false));
                    let link = Link {
                        client: client.try_clone().unwrap(),
                        server: server.try_clone().unwrap(),
                        armed: armed.clone(),
                        frozen: frozen.clone(),
                        reset: reset.clone(),
                    };
                    let (upstream, downstream) = (
                        (client.try_clone().unwrap(), server.try_clone().unwrap()),
                        (server, client),
                    );
                    shared.lock().unwrap().push(link);
                    let also = reset.clone();
                    thread::spawn(move || {
                        forward(upstream.0, upstream.1, Some((armed, frozen)), &also);
                    });
                    thread::spawn(move || forward(downstream.0, downstream.1, None, &reset));
                }
            });
        }
        Self {
            port,
            wake,
            links,
            accepted,
            stop,
        }
    }

    /// Connections accepted so far.
    pub(crate) fn connections(&self) -> usize {
        self.accepted.load(Ordering::Acquire)
    }

    /// Ends every current connection now: the client reads EOF.
    pub(crate) fn cut_all(&self) {
        for link in self.links.lock().unwrap().iter() {
            link.cut();
        }
    }

    /// Resets every current connection now, as a peer that crashes, or a middlebox that drops its state, does: the
    /// client reads a reset connection and no end of stream (State review P2-5).
    pub(crate) fn reset_all(&self) {
        let links = std::mem::take(&mut *self.links.lock().unwrap());
        for link in links {
            link.reset();
        }
    }

    /// Ends the oldest connection now, and no other.
    pub(crate) fn cut_oldest(&self) {
        if let Some(link) = self.links.lock().unwrap().first() {
            link.cut();
        }
    }

    /// Every current connection ends, unforwarded, when its client next sends
    /// bytes. Later connections are not affected.
    pub(crate) fn arm_existing(&self) {
        for link in self.links.lock().unwrap().iter() {
            link.armed.store(true, Ordering::Release);
        }
    }

    /// Every current connection stays open but forwards nothing more from its
    /// client. Later connections are not affected.
    pub(crate) fn freeze_existing(&self) {
        for link in self.links.lock().unwrap().iter() {
            link.frozen.store(true, Ordering::Release);
        }
    }
}

impl Drop for CutProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cut_all();
        // Wake the accept loops so that they see the stop.
        for address in &self.wake {
            let _ = TcpStream::connect(address);
        }
    }
}

/// Copies `from` to `to`. Upstream, `client` holds the link's armed and frozen
/// flags. A reset link ends at once, closing its sockets without a shutdown.
fn forward(
    mut from: TcpStream,
    mut to: TcpStream,
    client: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
    reset: &AtomicBool,
) {
    let mut buffer = [0; 16 * 1024];
    // A read that waits ends after a moment to see whether the link has been reset.
    let _ = from.set_read_timeout(Some(Duration::from_millis(20)));
    loop {
        let count = match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if reset.load(Ordering::Acquire) {
                    return;
                }
                continue;
            }
            Err(_) => break,
        };
        if reset.load(Ordering::Acquire) {
            return;
        }
        if let Some((armed, frozen)) = &client {
            if armed.load(Ordering::Acquire) {
                break;
            }
            if frozen.load(Ordering::Acquire) {
                continue;
            }
        }
        if to.write_all(&buffer[..count]).is_err() {
            break;
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}
