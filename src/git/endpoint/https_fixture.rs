//! Local TLS fixtures. Git subprocesses here are remote server implementations only.
use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
pub(crate) type Handler = Arc<
    dyn Fn(Request<Incoming>) -> Pin<Box<dyn Future<Output = Response<Full<Bytes>>> + Send>>
        + Send
        + Sync,
>;
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
fn identity() -> (Vec<u8>, Vec<u8>) {
    static CERT: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();
    CERT.get_or_init(||{
        let dir=tempfile::tempdir().unwrap();
        let run=|args:&[&str]|{let output=std::process::Command::new("openssl").current_dir(dir.path()).args(args).output().unwrap();assert!(output.status.success(),"fixture certificate command failed");};
        run(&["req","-x509","-newkey","rsa:2048","-nodes","-keyout","ca-key.pem","-out","ca.pem","-days","2","-subj","/CN=GWZ Fixture CA","-addext","basicConstraints=critical,CA:TRUE","-addext","keyUsage=critical,keyCertSign,cRLSign"]);
        run(&["req","-new","-newkey","rsa:2048","-nodes","-keyout","key.pem","-out","request.pem","-subj","/CN=localhost"]);
        std::fs::write(dir.path().join("extensions"),"subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n").unwrap();
        run(&["x509","-req","-in","request.pem","-CA","ca.pem","-CAkey","ca-key.pem","-CAcreateserial","-out","cert.pem","-days","1","-extfile","extensions"]);
        run(&["pkcs12","-export","-out","identity.p12","-inkey","key.pem","-in","cert.pem","-certfile","ca.pem","-passout","pass:fixture"]);
        (std::fs::read(dir.path().join("ca.pem")).unwrap(),std::fs::read(dir.path().join("identity.p12")).unwrap())
    }).clone()
}
impl Server {
    pub async fn start(handler: Handler) -> Self {
        let (ca, p12) = identity();
        let acceptor = tokio_native_tls::TlsAcceptor::from(
            native_tls::TlsAcceptor::new(
                native_tls::Identity::from_pkcs12(&p12, "fixture").unwrap(),
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "https://localhost:{}/repo",
            listener.local_addr().unwrap().port()
        );
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{let (socket,_)=result.unwrap();count.fetch_add(1,Ordering::SeqCst);let acceptor=acceptor.clone();let handler=handler.clone();children.spawn(async move {
                        if let Ok(tls)=acceptor.accept(socket).await {
                            let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls),service_fn(move |req|{let handler=handler.clone();async move {Ok::<_,Infallible>(handler(req).await)}})).await;
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
            ca_pem: Some(self.ca.clone()),
            ..Default::default()
        }
    }
}
pub(crate) fn response(
    status: u16,
    service: GitService,
    body: impl Into<Bytes>,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", https_policy::response_type(service))
        .body(Full::new(body.into()))
        .unwrap()
}
pub(super) fn input(server: &Server, service: GitService) -> Input {
    Input {
        destination: server.url.clone(),
        service,
        policy: AuthPolicy::Anonymous,
        session: "session".into(),
        operation: "operation".into(),
    }
}
/// Exchange discrete transport messages in memory; no framing/wire emulation.
pub(super) fn attach(prepared: Prepared) -> (Stream, JoinHandle<()>) {
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
        let work = prepared.serve(endpoint, right.clone(), CancellationToken::new());
        tokio::pin!(work);
        let start = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        let mut finished = false;
        loop {
            tokio::select! {
                _=&mut work,if !finished=>{finished=true;},
                message=left.next_message()=>match message {Ok(Some(m))=>{if right.deliver(m).is_err(){break;}},_=>break},
                message=right.next_message()=>match message {Ok(Some(m))=>{let terminal=matches!(m.kind,MessageKind::Closed|MessageKind::Failed);let _=left.deliver(m);if terminal {break;}},_=>break},
                _=tick.tick()=>{let now=start.elapsed().as_millis() as u64;assert!(now<10000,"message fixture stalled: left={:?} right={:?} finished={finished}",left.stats(),right.stats());left.advance(now);right.advance(now);},
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
        let (ca, p12) = identity();
        let acceptor = tokio_native_tls::TlsAcceptor::from(
            native_tls::TlsAcceptor::new(
                native_tls::Identity::from_pkcs12(&p12, "fixture").unwrap(),
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "https://localhost:{}/repo",
            listener.local_addr().unwrap().port()
        );
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{let (socket,_)=result.unwrap();let acceptor=acceptor.clone();let bytes=bytes.clone();count.fetch_add(1,Ordering::SeqCst);children.spawn(async move {
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
pub(super) struct Tunnel {
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
        let (_, p12) = identity();
        let acceptor = tokio_native_tls::TlsAcceptor::from(
            native_tls::TlsAcceptor::new(
                native_tls::Identity::from_pkcs12(&p12, "fixture").unwrap(),
            )
            .unwrap(),
        );
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
