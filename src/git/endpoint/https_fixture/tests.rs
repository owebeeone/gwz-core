//! The fixture's own TLS servers, judged by the product's TLS client
//! (`https_backend`: schannel on Windows, Security.framework on macOS, OpenSSL on
//! Linux). The servers are rustls so that they load no identity from the platform's
//! key store, which a key-based OpenSSH logon on Windows denies; the client verifies
//! against the fixture's CA exactly as the product's tests do, and trusts nothing else.
use super::*;
use crate::git::endpoint::{
    ca_bundle,
    https_backend::{Connector, Refused, RootsLoader},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const REQUEST: &[u8] = b"GET /repo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

fn port(url: &str) -> u16 {
    url.rsplit(':')
        .next()
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

cfg_if::cfg_if! {
    if #[cfg(any(windows, target_vendor = "apple"))] {
        /// The platform's own roots, which do not include the fixture CA.
        fn platform_roots() -> Option<RootsLoader> {
            None
        }
    } else {
        /// The roots the endpoint's loader gives on the OpenSSL stack, which has no roots of
        /// its own: none, so only what a connector is given beside them is trusted.
        fn platform_roots() -> Option<RootsLoader> {
            Some(std::sync::Arc::new(Vec::new))
        }
    }
}

fn trusting(ca: &[u8]) -> Connector {
    Connector::build(
        platform_roots().as_ref(),
        &ca_bundle::certificates(ca).unwrap(),
    )
    .unwrap()
}

/// Why an exchange failed: before the handshake or after it (`Io`), or in it (`Handshake`).
#[derive(Debug)]
enum Failed {
    Io(std::io::Error),
    Handshake(Refused),
}

impl From<std::io::Error> for Failed {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Everything the peer sends after `request`, over a handshake verified by `connector`.
async fn exchange(connector: &Connector, port: u16, request: &[u8]) -> Result<Vec<u8>, Failed> {
    let within = Duration::from_secs(10);
    let timed_out = |_| Failed::Io(std::io::ErrorKind::TimedOut.into());
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    let mut tls = tokio::time::timeout(within, connector.connect("localhost", tcp))
        .await
        .map_err(timed_out)?
        .map_err(Failed::Handshake)?;
    tls.write_all(request).await?;
    let mut reply = Vec::new();
    // An abrupt close after the reply is not this test's subject.
    let _ = tokio::time::timeout(within, tls.read_to_end(&mut reply)).await;
    Ok(reply)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn the_products_tls_client_completes_a_handshake_and_exchange_with_the_server() {
    runtime().block_on(async {
        let handler: Handler = Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "fixture-ok") })
        });
        let server = Server::start(handler).await;
        let (connector, port) = (trusting(&server.ca), port(&server.url));
        let reply = exchange(&connector, port, REQUEST).await.unwrap();
        let reply = String::from_utf8(reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.ends_with("fixture-ok"), "{reply}");
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn a_client_that_does_not_trust_the_fixture_ca_is_refused() {
    runtime().block_on(async {
        let handler: Handler = Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "never") })
        });
        let server = Server::start(handler).await;
        let port = port(&server.url);
        let default_roots = Connector::build(platform_roots().as_ref(), &[]).unwrap();
        let refused = exchange(&default_roots, port, REQUEST).await;
        // The connection was made and the handshake is what failed: not a refused or reset
        // connection, nor a read error. The platform names the cause in its own words, so the
        // positive control below shows the server is healthy for a client that trusts its CA.
        assert!(
            matches!(refused, Err(Failed::Handshake(_))),
            "the platform roots must not trust the fixture CA: {refused:?}"
        );
        let connector = trusting(&server.ca);
        let reply = exchange(&connector, port, REQUEST).await.unwrap();
        assert!(reply.starts_with(b"HTTP/1.1 200"));
    });
}

#[test]
fn a_scripted_server_hands_each_connection_to_its_script_after_the_handshake() {
    runtime().block_on(async {
        let script: Script = Arc::new(|mut tls| {
            Box::pin(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let Ok(byte) = tls.read_u8().await else {
                        return;
                    };
                    head.push(byte);
                }
                let _ = tls.write_all(b"scripted").await;
                let _ = tls.shutdown().await;
            })
        });
        let server = Server::scripted(script).await;
        let (connector, port) = (trusting(&server.ca), port(&server.url));
        let reply = exchange(&connector, port, REQUEST).await.unwrap();
        assert_eq!(reply, b"scripted");
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn the_raw_server_completes_a_handshake_before_its_scripted_bytes() {
    runtime().block_on(async {
        let server = Server::raw(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()).await;
        let (connector, port) = (trusting(&server.ca), port(&server.url));
        let reply = exchange(&connector, port, REQUEST).await.unwrap();
        assert_eq!(reply, b"HTTP/1.1 204 No Content\r\n\r\n");
    });
}

#[test]
fn the_secure_proxy_completes_a_handshake_before_its_connect_reply() {
    runtime().block_on(async {
        let proxy = Tunnel::with_tls(407, true).await;
        let (ca, _) = identity();
        let (connector, port) = (trusting(&ca), proxy.config.port);
        let reply = exchange(&connector, port, b"CONNECT 127.0.0.1:1 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        assert!(
            reply.starts_with(b"HTTP/1.1 407"),
            "{:?}",
            String::from_utf8_lossy(&reply)
        );
        assert_eq!(proxy.seen.lock().unwrap().len(), 1);
    });
}

#[test]
fn localhost_connects_without_waiting_out_a_refusal_on_the_other_loopback() {
    // "localhost" resolves to ::1 before 127.0.0.1 on Windows, where a connection to a closed
    // port takes about two seconds to fail; a fixture that serves only IPv4 makes every
    // connection through its URL pay that.
    runtime().block_on(async {
        let handler: Handler = Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        });
        let server = Server::start(handler).await;
        let proxy = crate::git::endpoint::cut_proxy::CutProxy::start(port(&server.url));
        for port in [port(&server.url), proxy.port] {
            let began = std::time::Instant::now();
            let _connection = std::net::TcpStream::connect(("localhost", port)).unwrap();
            assert!(
                began.elapsed() < Duration::from_millis(500),
                "localhost:{port} took {:?} to connect",
                began.elapsed()
            );
        }
    });
}
