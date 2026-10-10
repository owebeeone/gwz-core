//! Local TLS fixtures. Git subprocesses here are remote server implementations only.
use super::{
    https_connection, https_policy,
    https_wake::CloseWake,
    https_worker::{Input, Prepared},
    loopback::Loopback,
};
use bytes::Bytes;
use gwz_transport::{
    protocol::{AuthPolicy, GitService, MessageKind},
    stream::Stream,
};
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
mod tests;
pub(crate) type Handler = Arc<
    dyn Fn(Request<Incoming>) -> Pin<Box<dyn Future<Output = Response<Full<Bytes>>> + Send>>
        + Send
        + Sync,
>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionId(pub usize);
pub(crate) struct Server {
    pub url: String,
    pub ca: Vec<u8>,
    pub connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
/// The fixture CA (PEM, the bytes a client trusts) and a TLS acceptor over its leaf certificate.
///
/// The servers are rustls, not the platform's TLS: schannel loads a server identity through
/// the user's key store, which Windows denies under a key-based OpenSSH logon (DPAPI and
/// persisted CAPI and CNG keys: access denied), so a native-tls server cannot run there. The
/// clients under test stay native-tls, the product's stack, and verify against this CA.
fn identity() -> (Vec<u8>, TlsAcceptor) {
    static IDENTITY: OnceLock<(Vec<u8>, Arc<ServerConfig>)> = OnceLock::new();
    let (ca, config) = IDENTITY.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let output = std::process::Command::new("openssl").current_dir(dir.path()).args(args).output().expect("the openssl program starts");
            assert!(output.status.success(), "fixture certificate command failed: openssl {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        };
        run(&["req","-x509","-newkey","rsa:2048","-nodes","-keyout","ca-key.pem","-out","ca.pem","-days","2","-subj","/CN=GWZ Fixture CA","-addext","basicConstraints=critical,CA:TRUE","-addext","keyUsage=critical,keyCertSign,cRLSign"]);
        run(&["req","-new","-newkey","rsa:2048","-nodes","-keyout","key.pem","-out","request.pem","-subj","/CN=localhost"]);
        std::fs::write(dir.path().join("extensions"),"subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n").unwrap();
        run(&["x509","-req","-in","request.pem","-CA","ca.pem","-CAkey","ca-key.pem","-CAcreateserial","-out","cert.pem","-days","1","-extfile","extensions"]);
        run(&["x509","-in","cert.pem","-outform","DER","-out","cert.der"]);
        run(&["x509","-in","ca.pem","-outform","DER","-out","ca.der"]);
        run(&["pkcs8","-topk8","-nocrypt","-in","key.pem","-outform","DER","-out","key.der"]);
        let read = |name: &str| std::fs::read(dir.path().join(name)).unwrap();
        let chain = vec![CertificateDer::from(read("cert.der")), CertificateDer::from(read("ca.der"))];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(read("key.der")));
        // The ring provider is named here, so that no process-wide default is installed.
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        (read("ca.pem"), Arc::new(config))
    });
    (ca.clone(), TlsAcceptor::from(config.clone()))
}
impl Server {
    pub async fn start(handler: Handler) -> Self {
        let (ca, acceptor) = identity();
        let listener = Loopback::bind();
        let url = format!("https://localhost:{}/repo", listener.port);
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{let socket=result.unwrap();let connection_id=ConnectionId(count.fetch_add(1,Ordering::SeqCst)+1);let acceptor=acceptor.clone();let handler=handler.clone();children.spawn(async move {
                        if let Ok(tls)=acceptor.accept(socket).await {
                            let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls),service_fn(move |mut req|{let handler=handler.clone();req.extensions_mut().insert(connection_id);async move {Ok::<_,Infallible>(handler(req).await)}})).await;
                        }
                    });},
                    _=children.join_next(),if !children.is_empty()=>{},
                }
            }
        });
        Self {
            url,
            ca,
            connections,
            task,
        }
    }
    pub fn config(&self) -> https_connection::Config {
        https_connection::Config {
            ca_roots: super::ca_bundle::certificates(&self.ca).unwrap(),
            ..Default::default()
        }
    }
}
pub(crate) fn response(
    status: u16,
    service: GitService,
    body: impl Into<Bytes>,
) -> Response<Full<Bytes>> {
    let mut builder = Response::builder();
    if status == 401 {
        builder = builder.header("WWW-Authenticate", "Basic realm=\"fixture\"");
    }
    builder
        .status(status)
        .header("Content-Type", https_policy::response_type(service))
        .body(Full::new(body.into()))
        .unwrap()
}
pub(crate) fn input(server: &Server, service: GitService) -> Input {
    Input {
        destination: server.url.clone(),
        service,
        policy: AuthPolicy::Anonymous,
        session: "session".into(),
        operation: "operation".into(),
    }
}
/// Exchange discrete transport messages in memory; no framing/wire emulation.
pub(crate) fn attach(prepared: Prepared) -> (Stream, JoinHandle<()>) {
    let mut a =
        gwz_transport::stream::Config::new("session", 1, gwz_transport::stream::Side::Initiator);
    a.profile_version = 2;
    a.io_timeout_ms = prepared.io_timeout_ms();
    let mut b = a.clone();
    b.side = gwz_transport::stream::Side::Endpoint;
    let (stream, left) = Stream::new(a).unwrap();
    let (endpoint, right) = Stream::new(b).unwrap();
    let left = Arc::new(left);
    let right = Arc::new(right);
    let task = tokio::spawn(async move {
        let _endpoint_owner = endpoint.clone();
        let ready = CloseWake::default();
        let work = prepared.serve(
            endpoint,
            right.clone(),
            CancellationToken::new(),
            ready.clone(),
        );
        tokio::pin!(work);
        let start = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        let mut finished = false;
        loop {
            tokio::select! {
                _=&mut work,if !finished=>{finished=true;},
                message=left.next_message()=>match message {Ok(Some(m))=>{let delivered=right.deliver(m);ready.notify();if delivered.is_err(){break;}},_=>break},
                message=right.next_message()=>match message {Ok(Some(m))=>{ready.notify();let terminal=matches!(m.kind,MessageKind::Closed|MessageKind::Failed);let _=left.deliver(m);if terminal {break;}},_=>break},
                _=tick.tick()=>{let now=start.elapsed().as_millis() as u64;assert!(now<10000,"message fixture stalled: left={:?} right={:?} finished={finished}",left.stats(),right.stats());left.advance(now);right.advance(now);ready.notify();},
            }
        }
        left.disconnect();
        right.disconnect();
    });
    (stream, task)
}
impl Server {
    /// A deliberately malformed/partial HTTP peer, while retaining valid TLS.
    pub async fn raw(bytes: Vec<u8>) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (ca, acceptor) = identity();
        let listener = Loopback::bind();
        let url = format!("https://localhost:{}/repo", listener.port);
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{let socket=result.unwrap();let acceptor=acceptor.clone();let bytes=bytes.clone();count.fetch_add(1,Ordering::SeqCst);children.spawn(async move {
                        if let Ok(mut tls)=acceptor.accept(socket).await {let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n") && header.len()<65536 {let Ok(b)=tls.read_u8().await else{return;};header.push(b);}
                            let _=tls.write_all(&bytes).await;let _=tls.flush().await;tokio::time::sleep(Duration::from_millis(20)).await;let _=tls.shutdown().await;
                        }
                    });},
                    _=children.join_next(),if !children.is_empty()=>{},
                }
            }
        });
        Self {
            url,
            ca,
            connections,
            task,
        }
    }
}
pub(crate) struct Tunnel {
    pub config: https_connection::Proxy,
    pub seen: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
trait TunnelIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> TunnelIo for T {}
impl Tunnel {
    pub async fn start(status: u16) -> Self {
        Self::with_tls(status, false).await
    }
    pub async fn with_tls(status: u16, secure: bool) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_, acceptor) = identity();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{let (socket,_)=accepted.unwrap();let requests=requests.clone();let acceptor=acceptor.clone();children.spawn(async move {
                let mut socket:Box<dyn TunnelIo>=if secure {match acceptor.accept(socket).await{Ok(tls)=>Box::new(tls),Err(_)=>return}}else{Box::new(socket)};
                        let mut request=Vec::new();while !request.ends_with(b"\r\n\r\n") && request.len()<65536 {let Ok(b)=socket.read_u8().await else{return;};request.push(b);}
                        let request=String::from_utf8(request).unwrap();let authority=request.split_whitespace().nth(1).unwrap().to_owned();requests.lock().unwrap().push(request);
                        let reply=format!("HTTP/1.1 {status} Tunnel\r\n\r\n");socket.write_all(reply.as_bytes()).await.unwrap();
                        if status==200 {let mut origin=tokio::net::TcpStream::connect(authority).await.unwrap();let _=tokio::io::copy_bidirectional(&mut socket,&mut origin).await;}
                    });},
                    _=children.join_next(),if !children.is_empty()=>{},
                }
            }
        });
        Self {
            config: https_connection::Proxy {
                host: "127.0.0.1".into(),
                port,
                tls: secure,
                authorization: Some("Basic fixture-proxy-only".into()),
            },
            seen,
            task,
        }
    }
}
