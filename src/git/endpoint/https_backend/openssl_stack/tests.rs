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
    ssl::{SslConnector, SslVersion},
    x509::{
        X509NameBuilder,
        extension::{BasicConstraints, KeyUsage},
    },
};
use std::time::Duration;
use std::{
    io::Read,
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
struct Recorder(Vec<Vec<u8>>);
impl Store for Recorder {
    fn add(&mut self, root: Certificate) {
        self.0.push(root.to_der().unwrap());
    }
}
fn der(root: &Certificate) -> Vec<u8> {
    root.to_der().unwrap()
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
    fill(&mut store, Some(&loader), std::slice::from_ref(&c));
    assert_eq!(
        loads.load(Ordering::SeqCst),
        1,
        "the bundle is decoded once"
    );
    assert_eq!(store.0, [der(&a), der(&b), der(&c)], "and no default paths");
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
    // `native-tls` 0.2.18 builds on `SslConnector::builder` and then applies its builder's protocol
    // bounds, whose default is `min_protocol: Some(Protocol::Tlsv12)` and no maximum
    // (`TlsConnectorBuilder`, `supported_protocols` in `imp/openssl.rs`). The reference is not
    // normalized to anything else: the floor is TLS 1.2 whatever the host's `openssl.cnf` says.
    reference
        .set_min_proto_version(Some(SslVersion::TLS1_2))
        .unwrap();
    reference.set_max_proto_version(None).unwrap();
    let mut ours = base().unwrap();
    assert_eq!(ours.min_proto_version(), Some(SslVersion::TLS1_2));
    assert_eq!(ours.max_proto_version(), None);
    assert_eq!(ours.options(), reference.options());
    assert_eq!(ours.min_proto_version(), reference.min_proto_version());
    assert_eq!(ours.max_proto_version(), reference.max_proto_version());
    // `set_mode` returns the mode after adding what it is given, so adding nothing reads it.
    assert_eq!(
        ours.set_mode(SslMode::empty()),
        reference.set_mode(SslMode::empty()),
        "the SSL mode mask is SslConnector's"
    );
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

/// The first record of `native-tls`'s client and of ours are the same. This host-dependent oracle
/// sees the cipher suites, the extensions and the offered versions, but it cannot see a difference
/// in the protocol range on OpenSSL 3 at security level 1 or above, which refuses to offer TLS 1.1
/// and below whatever the context's minimum says: the floor itself is asserted in
/// `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots` and refused on the wire in
/// `a_server_capped_at_tls_1_1_is_refused`.
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

/// A row with no loader: the store is empty. The OpenSSL stack has no fallback to the process's
/// default verify paths; the endpoint always supplies the loader (`endpoint_environment`).
#[test]
fn without_a_loader_the_store_is_empty() {
    let connector = Connector::build(None, &[]).unwrap();
    assert_eq!(stored(&connector), 0);
    let with_ca = Connector::build(None, &[authority("a")]).unwrap();
    assert_eq!(stored(&with_ca), 1, "only the endpoint's own roots");
}

/// Set in the child of the protocol-floor test.
const FLOOR_CHILD: &str = "GWZ_TLS_FLOOR_CHILD";

/// An `openssl.cnf` for the child that enables TLS 1.0 and 1.1 the way a host with a laxer policy
/// has them: security level 0, and a system floor of TLS 1.0.
const LAX_POLICY: &str = "openssl_conf = gwz_test\n[gwz_test]\nssl_conf = gwz_ssl\n[gwz_ssl]\nsystem_default = gwz_system\n[gwz_system]\nCipherString = DEFAULT:@SECLEVEL=0\nMinProtocol = TLSv1\n";

/// A self-signed server certificate for `localhost`, its key, and the certificate as a root.
fn server_identity() -> (
    openssl::x509::X509,
    openssl::pkey::PKey<openssl::pkey::Private>,
) {
    use openssl::x509::extension::SubjectAlternativeName;
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    let serial = BigNum::from_u32(7).unwrap().to_asn1_integer().unwrap();
    builder.set_serial_number(&serial).unwrap();
    builder.set_subject_name(&name).unwrap();
    builder.set_issuer_name(&name).unwrap();
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
    let names = SubjectAlternativeName::new()
        .dns("localhost")
        .build(&builder.x509v3_context(None, None))
        .unwrap();
    builder.append_extension(names).unwrap();
    builder.sign(&key, MessageDigest::sha256()).unwrap();
    (builder.build(), key)
}

/// One TLS server on a loopback port that speaks TLS 1.0 and 1.1 only; the handshake's outcome
/// comes back from `join`.
fn tls_1_1_server() -> (u16, thread::JoinHandle<bool>, Certificate) {
    use openssl::ssl::Ssl;
    let (certificate, key) = server_identity();
    let mut context = SslContextBuilder::new(SslMethod::tls()).unwrap();
    context.set_cipher_list("DEFAULT:@SECLEVEL=0").unwrap();
    context
        .set_min_proto_version(Some(SslVersion::TLS1))
        .unwrap();
    context
        .set_max_proto_version(Some(SslVersion::TLS1_1))
        .unwrap();
    context.set_certificate(&certificate).unwrap();
    context.set_private_key(&key).unwrap();
    let context = context.build();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let ssl = Ssl::new(&context).unwrap();
        ssl.accept(socket).is_ok()
    });
    (port, server, Certificate(certificate))
}

async fn handshake(connector: &Connector, port: u16) -> bool {
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    connector.connect("localhost", tcp).await.is_ok()
}

/// The client refuses a server capped at TLS 1.1, which `native-tls` refused (its floor is TLS
/// 1.2) and a context with the library's lowest minimum would complete. Run in a child process
/// with an `OPENSSL_CONF` that enables TLS 1.0 and 1.1 (security level 0, system floor TLS 1.0),
/// so that the refusal is the context's own floor and not the host's policy. In the child a
/// control context with a TLS 1.0 floor must complete the handshake with the same server; where it
/// cannot (this OpenSSL has no TLS 1.1 at all), the row prints why and has nothing to show.
#[test]
fn a_server_capped_at_tls_1_1_is_refused() {
    if std::env::var_os(FLOOR_CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (port, server, certificate) = tls_1_1_server();
            let mut lax = base().unwrap();
            lax.set_min_proto_version(Some(SslVersion::TLS1)).unwrap();
            let mut store = X509StoreBuilder::new().unwrap();
            store.add(certificate.clone());
            lax.set_cert_store(store.build());
            let control = Connector {
                context: lax.build(),
            };
            let completed = handshake(&control, port).await;
            let _ = server.join();
            if !completed {
                println!(
                    "tls-floor: host-conditional: this OpenSSL does not complete TLS 1.1 even at \
                     SECLEVEL=0, so the refusal below shows nothing here"
                );
                return;
            }
            println!("tls-floor: the control (a TLS 1.0 floor) completes TLS 1.1 here");
            let (port, server, certificate) = tls_1_1_server();
            let roots: RootsLoader = Arc::new(move || vec![certificate.clone()]);
            let connector = Connector::build(Some(&roots), &[]).unwrap();
            let refused = !handshake(&connector, port).await;
            let _ = server.join();
            assert!(refused, "the context completed a TLS 1.1 handshake");
            println!("tls-floor: the product's connector refuses TLS 1.1");
        });
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let conf = dir.path().join("openssl.cnf");
    std::fs::write(&conf, LAX_POLICY).unwrap();
    let name = format!(
        "{}::a_server_capped_at_tls_1_1_is_refused",
        module_path!().split_once("::").unwrap().1
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads", "1"])
        .env(FLOOR_CHILD, "1")
        .env("OPENSSL_CONF", &conf)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| line.starts_with("tls-floor:")) {
        println!("{line}");
    }
    assert!(
        output.status.success(),
        "the floor test's child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the child ran no single passing test:\n{stdout}"
    );
}

/// What the 8 MiB rows move each way.
const EIGHT_MIB: usize = 8 << 20;

/// A server that writes 8 MiB before it reads anything, then reads 8 MiB.
fn writes_first() -> crate::git::endpoint::https_fixture::Script {
    Arc::new(|mut tls| {
        Box::pin(async move {
            let chunk = vec![0xAAu8; 64 * 1024];
            for _ in 0..EIGHT_MIB / chunk.len() {
                if tls.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            let _ = tls.flush().await;
            let mut sink = vec![0u8; 64 * 1024];
            let mut total = 0;
            while total < EIGHT_MIB {
                match tls.read(&mut sink).await {
                    Ok(0) | Err(_) => return,
                    Ok(count) => total += count,
                }
            }
            let _ = tls.write_all(b"done").await;
            let _ = tls.shutdown().await;
        })
    })
}

/// The client writes 8 MiB and reads 8 MiB concurrently on one stream, from one task as hyper's
/// connection task does. Neither side can finish unless reads progress while writes are blocked.
async fn full_duplex<S: AsyncRead + AsyncWrite + Unpin>(tls: S) {
    let (mut reader, mut writer) = tokio::io::split(tls);
    let write = async {
        let chunk = vec![0x55u8; 64 * 1024];
        for _ in 0..EIGHT_MIB / chunk.len() {
            writer.write_all(&chunk).await?;
        }
        writer.flush().await
    };
    let read = async {
        let mut got = vec![0u8; EIGHT_MIB];
        reader.read_exact(&mut got).await.map(|_| ())
    };
    let both = async { tokio::try_join!(write, read).map(|_| ()) };
    tokio::time::timeout(Duration::from_secs(30), both)
        .await
        .expect("the 8 MiB exchange stalled")
        .expect("the 8 MiB exchange failed");
}

/// The oracle: `native-tls` through `tokio-native-tls`, trusting the fixture's CA.
async fn native_client(server: &Server) -> tokio_native_tls::TlsStream<tokio::net::TcpStream> {
    let mut builder = native_tls::TlsConnector::builder();
    builder.disable_built_in_roots(true);
    builder.add_root_certificate(native_tls::Certificate::from_pem(&server.ca).unwrap());
    let connector = tokio_native_tls::TlsConnector::from(builder.build().unwrap());
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", server_port(server)))
        .await
        .unwrap();
    connector.connect("localhost", tcp).await.unwrap()
}

fn server_port(server: &Server) -> u16 {
    server
        .url
        .rsplit(':')
        .next()
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn native_tls_writes_and_reads_8_mib_at_once_against_a_peer_that_writes_first() {
    let server = Server::scripted(writes_first()).await;
    full_duplex(native_client(&server).await).await;
}

#[tokio::test]
async fn our_stream_writes_and_reads_8_mib_at_once_against_a_peer_that_writes_first() {
    let server = Server::scripted(writes_first()).await;
    full_duplex(connect(&server, "localhost").await.unwrap()).await;
}

/// A server that reads the request head, answers `413` and stops reading, keeping the connection.
fn answers_early() -> crate::git::endpoint::https_fixture::Script {
    Arc::new(|mut tls| {
        Box::pin(async move {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
                let Ok(byte) = tls.read_u8().await else {
                    return;
                };
                head.push(byte);
            }
            let reply = b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n";
            if tls.write_all(reply).await.is_err() || tls.flush().await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        })
    })
}

/// The client streams a 16 MiB body to a server that has answered and stopped reading. The
/// response head must arrive within 2 s, while the body is still blocked.
async fn early_response<S: AsyncRead + AsyncWrite + Unpin>(tls: S) {
    let (mut reader, mut writer) = tokio::io::split(tls);
    let write = async {
        let head = b"POST /repo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 16777216\r\n\r\n";
        let _ = writer.write_all(head).await;
        let chunk = vec![0x55u8; 64 * 1024];
        for _ in 0..(16 << 20) / chunk.len() {
            if writer.write_all(&chunk).await.is_err() {
                break;
            }
        }
        std::future::pending::<()>().await;
    };
    let read = async {
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(reader.read_u8().await.expect("the connection ended first"));
        }
        head
    };
    let head = tokio::select! {
        head = tokio::time::timeout(Duration::from_secs(20), read) => head,
        _ = write => unreachable!(),
    };
    let head = head.expect("the response head did not arrive within 20 s");
    assert!(head.starts_with(b"HTTP/1.1 413"), "{head:?}");
}

#[tokio::test]
async fn native_tls_reads_an_early_answer_while_a_16_mib_body_is_blocked() {
    let server = Server::scripted(answers_early()).await;
    early_response(native_client(&server).await).await;
}

#[tokio::test]
async fn our_stream_reads_an_early_answer_while_a_16_mib_body_is_blocked() {
    let server = Server::scripted(answers_early()).await;
    early_response(connect(&server, "localhost").await.unwrap()).await;
}
