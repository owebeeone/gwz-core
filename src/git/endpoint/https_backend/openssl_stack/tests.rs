//! The OpenSSL TLS client: that it reads the roots once, and that apart from
//! that it is what `native-tls` made (its first record on the wire, its
//! channel binding, its host name rules), and that its stream moves bytes.
use super::*;
use crate::git::endpoint::https_fixture::{Handler, Server, leaf_digest, response};
use gwz_transport::protocol::GitService;
use http_body_util::BodyExt;
use openssl::{
    asn1::Asn1Time,
    bn::BigNum,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::PKey,
    ssl::SslConnector,
    x509::{
        X509NameBuilder,
        extension::{BasicConstraints, KeyUsage},
    },
};
use std::{
    io::Read,
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A self-signed CA certificate named `name`.
fn authority(name: &str) -> Certificate {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
    let mut subject = X509NameBuilder::new().unwrap();
    subject.append_entry_by_text("CN", name).unwrap();
    let subject = subject.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    let serial = BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap();
    builder.set_serial_number(&serial).unwrap();
    builder.set_subject_name(&subject).unwrap();
    builder.set_issuer_name(&subject).unwrap();
    builder.set_pubkey(&key).unwrap();
    builder
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    builder
        .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    builder
        .append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .crl_sign()
                .build()
                .unwrap(),
        )
        .unwrap();
    builder.sign(&key, MessageDigest::sha256()).unwrap();
    Certificate(builder.build())
}

/// How many certificates the connector's store holds.
fn stored(connector: &Connector) -> usize {
    connector.context.cert_store().all_certificates().len()
}

/// What was put into a store, in order.
#[derive(Debug, Default, PartialEq)]
struct Recorder(Vec<Event>);
#[derive(Debug, PartialEq)]
enum Event {
    Root(Vec<u8>),
    DefaultPaths,
}
impl Store for Recorder {
    fn add(&mut self, root: Certificate) {
        self.0.push(Event::Root(root.to_der().unwrap()));
    }
    fn add_defaults(&mut self) -> Result<(), ErrorStack> {
        self.0.push(Event::DefaultPaths);
        Ok(())
    }
}
fn der(root: &Certificate) -> Event {
    Event::Root(root.to_der().unwrap())
}

#[test]
fn a_loader_gives_the_only_roots_it_is_asked_once_and_the_default_paths_are_never_read() {
    let (a, b, c) = (authority("a"), authority("b"), authority("c"));
    let loads = Arc::new(AtomicUsize::new(0));
    let loader: RootsLoader = {
        let (loads, roots) = (loads.clone(), vec![a.clone(), b.clone()]);
        Arc::new(move || {
            loads.fetch_add(1, Ordering::SeqCst);
            roots.clone()
        })
    };
    let mut store = Recorder::default();
    fill(&mut store, Some(&loader), std::slice::from_ref(&c)).unwrap();
    assert_eq!(
        loads.load(Ordering::SeqCst),
        1,
        "the bundle is decoded once"
    );
    assert_eq!(store.0, [der(&a), der(&b), der(&c)], "and no default paths");
}

#[test]
fn without_a_loader_the_default_paths_are_read_once_before_the_endpoints_roots() {
    let c = authority("c");
    let mut store = Recorder::default();
    fill(&mut store, None, std::slice::from_ref(&c)).unwrap();
    assert_eq!(store.0, [Event::DefaultPaths, der(&c)]);
}

#[test]
fn a_connectors_store_holds_the_loaders_roots_and_the_endpoints_and_nothing_else() {
    let (a, b, c) = (authority("a"), authority("b"), authority("c"));
    let loader: RootsLoader = {
        let roots = vec![a, b];
        Arc::new(move || roots.clone())
    };
    let connector = Connector::build(Some(&loader), &[c]).unwrap();
    assert_eq!(stored(&connector), 3);
}

#[test]
fn without_a_loader_the_roots_are_the_default_verify_paths() {
    let defaults = SslConnector::builder(SslMethod::tls()).unwrap().build();
    let connector = Connector::build(None, &[]).unwrap();
    assert_eq!(
        stored(&connector),
        defaults.context().cert_store().all_certificates().len()
    );
}

#[test]
fn a_host_that_has_default_verify_paths_shows_the_loader_leaves_them_out() {
    // The control for the first test: where the host has a system bundle, a context
    // made the `SslConnector` way holds it, and the loader's does not.
    let defaults = SslConnector::builder(SslMethod::tls()).unwrap().build();
    let held = defaults.context().cert_store().all_certificates().len();
    if held == 0 {
        eprintln!("this host has no default CA bundle: the control has nothing to show");
        return;
    }
    let loader: RootsLoader = Arc::new(Vec::new);
    let connector = Connector::build(Some(&loader), &[]).unwrap();
    assert_eq!(
        stored(&connector),
        0,
        "the host's {held} default roots are not read"
    );
}

#[test]
fn the_context_is_the_one_sslconnector_makes_bar_its_trust_roots() {
    let mut reference = SslConnector::builder(SslMethod::tls()).unwrap();
    // `native-tls` sets no protocol bounds, which clears them to the library's.
    reference.set_min_proto_version(None).unwrap();
    reference.set_max_proto_version(None).unwrap();
    let mut ours = base().unwrap();
    assert_eq!(ours.options(), reference.options());
    assert_eq!(ours.min_proto_version(), reference.min_proto_version());
    assert_eq!(ours.max_proto_version(), reference.max_proto_version());
    assert_eq!(
        ours.build().verify_mode(),
        reference.build().context().verify_mode()
    );
}

/// What a ClientHello says of the client, without what changes with every one:
/// the random, the session id and the key shares' keys.
#[derive(Debug, PartialEq)]
struct Hello {
    record_version: [u8; 2],
    version: [u8; 2],
    suites: Vec<u8>,
    compression: Vec<u8>,
    extensions: Vec<(u16, Vec<u8>)>,
}

fn word(bytes: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn parse(record: &[u8]) -> Hello {
    assert_eq!(record[0], 22, "a handshake record");
    assert_eq!(record[5], 1, "a ClientHello");
    let mut at = 9;
    let version = [record[at], record[at + 1]];
    at += 2 + 32;
    at += 1 + usize::from(record[at]);
    let suites_length = word(&record[at..]);
    let suites = record[at + 2..at + 2 + suites_length].to_vec();
    at += 2 + suites_length;
    let compression_length = usize::from(record[at]);
    let compression = record[at + 1..at + 1 + compression_length].to_vec();
    at += 1 + compression_length;
    let end = at + 2 + word(&record[at..]);
    at += 2;
    let mut extensions = Vec::new();
    while at < end {
        let (kind, length) = (word(&record[at..]), word(&record[at + 2..]));
        let data = &record[at + 4..at + 4 + length];
        at += 4 + length;
        let data = if kind == 51 {
            // key_share: the groups and the lengths of their keys, not the keys.
            let mut shares = Vec::new();
            let mut share = 2;
            while share < data.len() {
                let key = word(&data[share + 2..]);
                shares.extend_from_slice(&data[share..share + 4]);
                share += 4 + key;
            }
            shares
        } else {
            data.to_vec()
        };
        extensions.push((u16::try_from(kind).unwrap(), data));
    }
    Hello {
        record_version: [record[1], record[2]],
        version,
        suites,
        compression,
        extensions,
    }
}

/// The first record `client` sends to a server that reads it and hangs up.
fn first_record(client: impl FnOnce(TcpStream) + Send + 'static) -> Vec<u8> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let client = thread::spawn(move || client(TcpStream::connect(("127.0.0.1", port)).unwrap()));
    let (mut socket, _) = listener.accept().unwrap();
    let mut header = [0u8; 5];
    socket.read_exact(&mut header).unwrap();
    let mut body = vec![0u8; word(&header[3..])];
    socket.read_exact(&mut body).unwrap();
    drop(socket);
    client.join().unwrap();
    [header.to_vec(), body].concat()
}

#[test]
fn the_client_hello_is_what_native_tls_sends() {
    let native = first_record(|tcp| {
        let connector = native_tls::TlsConnector::new().unwrap();
        assert!(connector.connect("localhost", tcp).is_err());
    });
    let ours = first_record(|tcp| {
        tcp.set_nonblocking(true).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let tcp = tokio::net::TcpStream::from_std(tcp).unwrap();
            let connector = Connector::build(None, &[]).unwrap();
            assert!(connector.connect("localhost", tcp).await.is_err());
        });
    });
    let (native, ours) = (parse(&native), parse(&ours));
    assert!(
        native.extensions.iter().any(|(kind, _)| *kind == 43),
        "the offered versions are in it"
    );
    assert_eq!(ours, native);
}

fn nothing() -> Handler {
    Arc::new(|_| Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") }))
}

async fn connect(server: &Server, host: &str) -> Result<Stream<tokio::net::TcpStream>, Refused> {
    let port = server.url.rsplit(':').next().unwrap().split('/').next();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port.unwrap().parse::<u16>().unwrap()))
        .await
        .unwrap();
    let roots = crate::git::endpoint::ca_bundle::certificates(&server.ca).unwrap();
    let nothing: RootsLoader = Arc::new(Vec::new);
    Connector::build(Some(&nothing), &roots)
        .unwrap()
        .connect(host, tcp)
        .await
}

#[tokio::test]
async fn the_name_or_address_must_be_one_the_certificate_is_for() {
    let server = Server::start(nothing()).await;
    // The fixture's certificate is for localhost, proxied.test and 127.0.0.1.
    for host in ["localhost", "127.0.0.1", "proxied.test"] {
        assert!(connect(&server, host).await.is_ok(), "{host}");
    }
    for host in ["other.example", "127.0.0.2", "sub.localhost"] {
        assert!(connect(&server, host).await.is_err(), "{host}");
    }
}

#[tokio::test]
async fn the_channel_binding_is_the_one_native_tls_computes() {
    let server = Server::start(nothing()).await;
    let port: u16 = server
        .url
        .rsplit(':')
        .next()
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let ca = server.ca.clone();
    let native = tokio::task::spawn_blocking(move || {
        let mut builder = native_tls::TlsConnector::builder();
        builder.disable_built_in_roots(true);
        builder.add_root_certificate(native_tls::Certificate::from_pem(&ca).unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let tls = builder.build().unwrap().connect("localhost", tcp).unwrap();
        tls.tls_server_end_point().unwrap()
    })
    .await
    .unwrap();
    let stream = connect(&server, "localhost").await.unwrap();
    let ours = tls_server_end_point(&stream);
    assert_eq!(ours, native);
    assert_eq!(
        ours,
        Some(leaf_digest()),
        "the SHA-256 of the leaf (RFC 5929)"
    );
}

/// The next `count` bytes of a request's body, counted by the server, are
/// answered with their number.
fn counting() -> Handler {
    Arc::new(|request| {
        Box::pin(async move {
            let body = request.into_body().collect().await.unwrap().to_bytes();
            response(
                200,
                GitService::UploadPackAdvertisement,
                body.len().to_string(),
            )
        })
    })
}

#[tokio::test]
async fn a_megabyte_goes_out_in_many_records_and_the_answer_comes_back() {
    let server = Server::start(counting()).await;
    let mut tls = connect(&server, "localhost").await.unwrap();
    let body = vec![7u8; 1 << 20];
    let head = format!(
        "POST /repo HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(head.as_bytes()).await.unwrap();
    tls.write_all(&body).await.unwrap();
    tls.flush().await.unwrap();
    let mut reply = Vec::new();
    // An abrupt close after the reply is not this test's subject.
    let _ = tls.read_to_end(&mut reply).await;
    let reply = String::from_utf8(reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with(&format!("\r\n\r\n{}", 1 << 20)), "{reply}");
}

#[tokio::test]
async fn a_megabyte_comes_in_many_records_and_the_close_is_sent() {
    let body = vec![9u8; 1 << 20];
    let handler: Handler = {
        let body = bytes::Bytes::from(body.clone());
        Arc::new(move |_| {
            let body = body.clone();
            Box::pin(async move { response(200, GitService::UploadPackAdvertisement, body) })
        })
    };
    let server = Server::start(handler).await;
    let mut tls = connect(&server, "localhost").await.unwrap();
    tls.write_all(b"GET /repo HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = Vec::new();
    tls.read_to_end(&mut reply).await.unwrap_or_default();
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    assert!(reply.ends_with(&body), "{} bytes", reply.len());
    tls.shutdown().await.unwrap();
}
