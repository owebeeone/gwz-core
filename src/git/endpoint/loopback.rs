//! The loopback listeners a fixture serves "localhost" on.
//!
//! "localhost" resolves to `::1` before `127.0.0.1` on Windows, where a connection to a
//! closed port takes about two seconds to be refused before the client falls back to IPv4. A
//! fixture reached by a `localhost` URL therefore listens on both loopbacks, on one port, as a
//! real server for the name does; with IPv6 unavailable it serves IPv4 alone.
use std::{
    future::poll_fn,
    io,
    net::{Ipv6Addr, TcpListener},
    task::Poll,
};

/// A listener on `127.0.0.1` at a free port, and one on `::1` at the same port when that port
/// is free there too. The first is always IPv4.
pub(crate) fn bind() -> Vec<TcpListener> {
    let v4 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = v4.local_addr().unwrap().port();
    let mut listeners = vec![v4];
    if let Ok(v6) = TcpListener::bind((Ipv6Addr::LOCALHOST, port)) {
        listeners.push(v6);
    }
    listeners
}

/// The listeners of `bind`, for a tokio fixture.
pub(crate) struct Loopback {
    listeners: Vec<tokio::net::TcpListener>,
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
        Self { listeners, port }
    }

    /// The next connection to either address.
    pub(crate) async fn accept(&self) -> io::Result<tokio::net::TcpStream> {
        poll_fn(|cx| {
            for listener in &self.listeners {
                if let Poll::Ready(accepted) = listener.poll_accept(cx) {
                    return Poll::Ready(accepted.map(|(socket, _)| socket));
                }
            }
            Poll::Pending
        })
        .await
    }
}
