//! The fixture's own TLS servers, judged by a native-tls client: the product's TLS stack
//! (schannel on Windows, Security.framework on macOS, OpenSSL on Linux). The servers are
//! rustls so that they load no identity from the platform's key store, which a key-based
//! OpenSSH logon on Windows denies; the client verifies against the fixture's CA exactly
//! as the product's tests do, and trusts nothing else.
use super::*;
use crate::git::endpoint::ca_bundle;
use std::io::{Read, Write};

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

fn trusting(ca: &[u8]) -> native_tls::TlsConnector {
    let mut builder = native_tls::TlsConnector::builder();
    for root in ca_bundle::certificates(ca).unwrap() {
        builder.add_root_certificate(root);
    }
    builder.build().unwrap()
}

/// Everything the peer sends after `request`, over a handshake verified by `connector`.
fn exchange(
    connector: &native_tls::TlsConnector,
    port: u16,
    request: &[u8],
) -> std::io::Result<Vec<u8>> {
    let tcp = std::net::TcpStream::connect(("127.0.0.1", port))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut tls = connector
        .connect("localhost", tcp)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    tls.write_all(request)?;
    let mut reply = Vec::new();
    // An abrupt close after the reply is not this test's subject.
    let _ = tls.read_to_end(&mut reply);
    Ok(reply)
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work).await.unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_native_tls_client_completes_a_handshake_and_exchange_with_the_server() {
    runtime().block_on(async {
        let handler: Handler = Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "fixture-ok") })
        });
        let server = Server::start(handler).await;
        let (connector, port) = (trusting(&server.ca), port(&server.url));
        let reply = blocking(move || exchange(&connector, port, REQUEST))
            .await
            .unwrap();
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
        let default_roots = native_tls::TlsConnector::new().unwrap();
        let refused = blocking(move || exchange(&default_roots, port, REQUEST)).await;
        assert!(
            refused.is_err(),
            "the platform roots must not trust the fixture CA"
        );
    });
}

#[test]
fn the_raw_server_completes_a_handshake_before_its_scripted_bytes() {
    runtime().block_on(async {
        let server = Server::raw(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()).await;
        let (connector, port) = (trusting(&server.ca), port(&server.url));
        let reply = blocking(move || exchange(&connector, port, REQUEST))
            .await
            .unwrap();
        assert_eq!(reply, b"HTTP/1.1 204 No Content\r\n\r\n");
    });
}

#[test]
fn the_secure_proxy_completes_a_handshake_before_its_connect_reply() {
    runtime().block_on(async {
        let proxy = Tunnel::with_tls(407, true).await;
        let (ca, _) = identity();
        let (connector, port) = (trusting(&ca), proxy.config.port);
        let reply =
            blocking(move || exchange(&connector, port, b"CONNECT 127.0.0.1:1 HTTP/1.1\r\n\r\n"))
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
