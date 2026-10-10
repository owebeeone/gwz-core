//! The loopback listeners a fixture serves "localhost" on.
//!
//! "localhost" resolves to `::1` before `127.0.0.1` on Windows, where a connection to a
//! closed port takes about two seconds to be refused before the client falls back to IPv4. A
//! fixture reached by a `localhost` URL therefore listens on both loopbacks, on one port, as a
//! real server for the name does; with no IPv6 loopback it serves IPv4 alone and says so.
use std::{
    future::poll_fn,
    io,
    net::{Ipv6Addr, TcpListener},
    sync::atomic::{AtomicUsize, Ordering},
    task::Poll,
};

/// How many ports the pair is tried on before the bind gives up.
const ATTEMPTS: usize = 16;

/// A listener on `127.0.0.1` at a free port, and one on `::1` at the same port. The first is
/// always IPv4. A port taken on `::1` (another process holds it) is tried again on a fresh
/// port; only a host with no IPv6 loopback serves IPv4 alone, and says so.
pub(crate) fn bind() -> Vec<TcpListener> {
    bind_with(|| TcpListener::bind(("127.0.0.1", 0)))
}

/// `bind`, with the IPv4 listener (and so its port) chosen by `pick_v4`; a test's seam.
fn bind_with(mut pick_v4: impl FnMut() -> io::Result<TcpListener>) -> Vec<TcpListener> {
    for _ in 0..ATTEMPTS {
        let v4 = pick_v4().unwrap();
        let port = v4.local_addr().unwrap().port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, port)) {
            Ok(v6) => return vec![v4, v6],
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::AddrNotAvailable | io::ErrorKind::Unsupported
                ) =>
            {
                eprintln!("loopback fixture: no IPv6 loopback ({error}); serving IPv4 only");
                return vec![v4];
            }
            Err(error) => panic!("loopback fixture: binding [::1]:{port} failed: {error}"),
        }
    }
    panic!("loopback fixture: no port free on both loopbacks in {ATTEMPTS} attempts");
}

/// The listeners of `bind`, for a tokio fixture.
pub(crate) struct Loopback {
    listeners: Vec<tokio::net::TcpListener>,
    /// Where the next `accept` starts looking, so that no listener is served last every time.
    next: AtomicUsize,
    pub(crate) port: u16,
}

impl Loopback {
    /// Must run inside a tokio runtime.
    pub(crate) fn bind() -> Self {
        let listeners = bind()
            .into_iter()
            .map(|listener| {
                listener.set_nonblocking(true).unwrap();
                tokio::net::TcpListener::from_std(listener).unwrap()
            })
            .collect::<Vec<_>>();
        let port = listeners[0].local_addr().unwrap().port();
        Self {
            listeners,
            next: AtomicUsize::new(0),
            port,
        }
    }

    /// The next connection to either address.
    pub(crate) async fn accept(&self) -> io::Result<tokio::net::TcpStream> {
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        poll_fn(|cx| {
            for offset in 0..self.listeners.len() {
                let listener = &self.listeners[(start + offset) % self.listeners.len()];
                if let Poll::Ready(accepted) = listener.poll_accept(cx) {
                    return Poll::Ready(accepted.map(|(socket, _)| socket));
                }
            }
            Poll::Pending
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpStream};

    fn port_of(listener: &TcpListener) -> u16 {
        listener.local_addr().unwrap().port()
    }

    /// A free `::1` port is held only where IPv6 loopback exists; the tests below need it.
    fn ipv6_loopback_exists() -> bool {
        TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_ok()
    }

    #[test]
    fn a_port_taken_on_ipv6_makes_the_pair_retry_on_a_fresh_port() {
        if !ipv6_loopback_exists() {
            return;
        }
        let mut held = None;
        let mut picks = 0;
        let listeners = bind_with(|| {
            picks += 1;
            let v4 = TcpListener::bind(("127.0.0.1", 0))?;
            if held.is_none() {
                // Another process holds `::1` on the port the IPv4 listener was given.
                held = TcpListener::bind((Ipv6Addr::LOCALHOST, port_of(&v4))).ok();
            }
            Ok(v4)
        });
        let held = held.expect("the test held ::1 on the first port");
        assert!(picks >= 2, "the pair was retried");
        assert_eq!(listeners.len(), 2, "the fixture is dual-stack");
        assert_ne!(port_of(&listeners[0]), port_of(&held));
        assert_eq!(port_of(&listeners[0]), port_of(&listeners[1]));
    }

    #[test]
    fn accept_rotates_so_a_busy_ipv4_listener_cannot_starve_ipv6() {
        if !ipv6_loopback_exists() {
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let loopback = Loopback::bind();
                if loopback.listeners.len() != 2 {
                    return;
                }
                let v4 = SocketAddr::from(([127, 0, 0, 1], loopback.port));
                let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, loopback.port));
                // Two connections wait on IPv4 and one on IPv6.
                let _clients = [v4, v4, v6].map(|address| TcpStream::connect(address).unwrap());
                let first = loopback.accept().await.unwrap();
                let second = loopback.accept().await.unwrap();
                let families = [first, second].map(|socket| socket.local_addr().unwrap().is_ipv6());
                assert!(
                    families.contains(&true),
                    "IPv6 was served second, not starved"
                );
            });
    }
}
