//! Endpoint-owned TLS and a single Hyper HTTP/1 connection; no secondary pool.
use super::{
    agent_job::Job,
    ssh_pool::{Connector, Resource},
};
use bytes::Bytes;
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{Effect, ErrorCode, Failure, Scheme},
};
use hyper::{
    body::{Body, Frame},
    client::conn::http1,
};
use hyper_util::rt::TokioIo;
use std::{
    future::Future,
    io,
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{Mutex, Semaphore, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn failure(code: ErrorCode) -> Failure {
    Failure {
        code,
        effect: Effect::None,
        facts: None,
    }
}
#[derive(Clone)]
pub(crate) struct Proxy {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) tls: bool,
    /// Endpoint-owned value, never part of an Open or Debug output.
    pub(crate) authorization: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct Config {
    pub(crate) ca_pem: Option<Vec<u8>>,
    pub(crate) proxy: Option<Proxy>,
    /// Exact DNS hosts/IPs or leading-dot suffixes. No caller/core environment.
    pub(crate) no_proxy: Vec<String>,
}
impl Config {
    pub(crate) fn validate(&self) -> Result<(), Failure> {
        if self.no_proxy.iter().any(|h| {
            h.is_empty()
                || h.chars().any(|c| c.is_control() || c.is_whitespace())
                || h.contains(['/', ':', '@'])
        }) {
            return Err(failure(ErrorCode::InvalidRequest));
        }
        if let Some(proxy) = &self.proxy {
            if proxy.host.is_empty()
                || proxy.port == 0
                || proxy
                    .host
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || "/@?#".contains(c))
                || proxy.authorization.as_ref().is_some_and(|s| {
                    s.len() > 16384 || s.chars().any(char::is_control) || !s.starts_with("Basic ")
                })
            {
                return Err(failure(ErrorCode::UnsupportedOperation));
            }
        }
        Ok(())
    }
    fn proxy_for(&self, host: &str) -> Option<Proxy> {
        if self.no_proxy.iter().any(|entry| {
            entry == "*"
                || entry.eq_ignore_ascii_case(host)
                || entry.strip_prefix('.').is_some_and(|suffix| {
                    host.eq_ignore_ascii_case(suffix)
                        || host
                            .to_ascii_lowercase()
                            .ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
                })
        }) {
            None
        } else {
            self.proxy.clone()
        }
    }
}
pub(crate) struct RequestBody {
    pub(crate) rx: mpsc::Receiver<io::Result<Bytes>>,
}
impl Body for RequestBody {
    type Data = Bytes;
    type Error = io::Error;
    fn is_end_stream(&self) -> bool {
        self.rx.is_closed() && self.rx.is_empty()
    }
    fn size_hint(&self) -> hyper::body::SizeHint {
        if self.is_end_stream() {
            hyper::body::SizeHint::with_exact(0)
        } else {
            hyper::body::SizeHint::default()
        }
    }
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.rx
            .poll_recv(cx)
            .map(|item| item.map(|r| r.map(Frame::data)))
    }
}
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Socket = Box<dyn Io>;
pub(crate) struct Connection {
    pub(crate) sender: http1::SendRequest<RequestBody>,
    driver: JoinHandle<()>,
    pub(crate) progress: Arc<AtomicU64>,
}
pub(crate) struct HttpResource {
    setup: Option<Job<Setup>>,
    connecting: Option<JoinHandle<Result<Connection, Failure>>>,
    pub(crate) connection: Option<Arc<Mutex<Connection>>>,
    pub(crate) reusable: Arc<AtomicBool>,
    pub(crate) cancel: CancellationToken,
    driver_abort: Option<tokio::task::AbortHandle>,
    deadline: Option<Instant>,
    key: Key,
    pub(crate) disposed: Arc<AtomicBool>,
    pub(crate) connect_elapsed: Duration,
    connect_started: Instant,
}
struct Setup {
    addresses: Vec<SocketAddr>,
    tls: native_tls::TlsConnector,
    proxy: Option<Proxy>,
}
pub(crate) struct HttpConnector {
    pub(crate) config: Config,
    pub(crate) epoch: Instant,
    pub(crate) setup_slots: Arc<Semaphore>,
}
impl Connector for HttpConnector {
    type Resource = HttpResource;
    fn start(
        &mut self,
        key: &Key,
        _identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<HttpResource, Failure> {
        self.config.validate()?;
        if key.scheme != Scheme::Https {
            return Err(failure(ErrorCode::UnsupportedOperation));
        }
        let permit = self
            .setup_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure(ErrorCode::Capacity))?;
        let config = self.config.clone();
        let key_copy = key.clone();
        let deadline = deadline.and_then(|ms| self.epoch.checked_add(Duration::from_millis(ms)));
        let setup = Job::start(deadline, Duration::from_secs(5), move |control| {
            let _permit = permit;
            control.check()?;
            let proxy = config.proxy_for(&key_copy.host);
            let (host, port) = proxy
                .as_ref()
                .map_or((key_copy.host.as_str(), key_copy.port), |p| {
                    (p.host.as_str(), p.port)
                });
            let addresses = (host, port).to_socket_addrs()?.take(16).collect::<Vec<_>>();
            control.check()?;
            let mut builder = native_tls::TlsConnector::builder();
            if let Some(pem) = config.ca_pem {
                builder.add_root_certificate(
                    native_tls::Certificate::from_pem(&pem)
                        .map_err(|_| io::ErrorKind::InvalidInput)?,
                );
            }
            let tls = builder.build().map_err(|_| io::ErrorKind::InvalidInput)?;
            Ok(Setup {
                addresses,
                tls,
                proxy,
            })
        })
        .map_err(|_| failure(ErrorCode::Capacity))?;
        Ok(HttpResource {
            setup: Some(setup),
            connecting: None,
            connection: None,
            reusable: Arc::new(AtomicBool::new(false)),
            cancel: CancellationToken::new(),
            driver_abort: None,
            deadline,
            key: key.clone(),
            disposed: Arc::new(AtomicBool::new(false)),
            connect_elapsed: Duration::ZERO,
            connect_started: Instant::now(),
        })
    }
}
impl Resource for HttpResource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        if let Some(setup) = &mut self.setup {
            let result = match setup.poll_result(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.setup = None;
            let setup = match result {
                Ok(setup) => setup,
                Err(error) => {
                    return Poll::Ready(Err(failure(if error.kind() == io::ErrorKind::TimedOut {
                        ErrorCode::Timeout
                    } else {
                        ErrorCode::Io
                    })));
                }
            };
            let key = self.key.clone();
            let cancelled = self.cancel.clone();
            let deadline = self.deadline;
            self.connecting = Some(tokio::spawn(async move {
                tokio::select! {
                    _=cancelled.cancelled()=>Err(failure(ErrorCode::Cancelled)),
                    _=async {if let Some(at)=deadline {tokio::time::sleep_until(at.into()).await;} else {std::future::pending::<()>().await;}}=>Err(failure(ErrorCode::Timeout)),
                    result=connect(setup,key)=>result,
                }
            }));
        }
        if let Some(task) = &mut self.connecting {
            match Pin::new(task).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    self.connecting = None;
                    let connection = match result {
                        Ok(Ok(c)) => c,
                        Ok(Err(e)) => return Poll::Ready(Err(e)),
                        Err(_) => return Poll::Ready(Err(failure(ErrorCode::Io))),
                    };
                    self.connect_elapsed = self.connect_started.elapsed();
                    self.driver_abort = Some(connection.driver.abort_handle());
                    self.connection = Some(Arc::new(Mutex::new(connection)));
                    self.reusable.store(true, Ordering::Release);
                }
            }
        }
        // HTTPS proves the TLS resource identity, not an authenticated account.
        // The generic pool requires this proof before admitting idle reuse.
        Poll::Ready(Ok(Some(Identity::Https)))
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>, _force: bool) -> Poll<io::Result<()>> {
        self.cancel.cancel();
        self.reusable.store(false, Ordering::Release);
        if let Some(job) = &mut self.setup {
            match job.poll_disposed(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => self.setup = None,
            }
        }
        if let Some(task) = &mut self.connecting {
            // Cancellation is selected inside connect; join before acknowledging capacity.
            match Pin::new(task).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    self.connecting = None;
                    if let Ok(Ok(connection)) = result {
                        self.connect_elapsed = self.connect_started.elapsed();
                        self.driver_abort = Some(connection.driver.abort_handle());
                        self.connection = Some(Arc::new(Mutex::new(connection)));
                    }
                }
            }
        }
        if let Some(abort) = &self.driver_abort {
            abort.abort();
        }
        if let Some(connection) = &self.connection {
            if Arc::strong_count(connection) > 1 {
                return Poll::Pending;
            }
            let Ok(mut connection) = connection.try_lock() else {
                return Poll::Pending;
            };
            if Pin::new(&mut connection.driver).poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
        self.connection = None;
        self.driver_abort = None;
        self.disposed.store(true, Ordering::Release);
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        self.reusable.load(Ordering::Acquire)
            && !self.cancel.is_cancelled()
            && self
                .driver_abort
                .as_ref()
                .is_some_and(|task| !task.is_finished())
    }
}
impl Drop for HttpResource {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = &self.driver_abort {
            task.abort();
        }
        if let Some(task) = &self.connecting {
            task.abort();
        }
    }
}
async fn connect(setup: Setup, key: Key) -> Result<Connection, Failure> {
    let mut socket = None;
    for address in setup.addresses {
        if let Ok(connected) = TcpStream::connect(address).await {
            socket = Some(connected);
            break;
        }
    }
    let socket = socket.ok_or_else(|| failure(ErrorCode::Io))?;
    socket
        .set_nodelay(true)
        .map_err(|_| failure(ErrorCode::Io))?;
    let tls = tokio_native_tls::TlsConnector::from(setup.tls);
    let mut io: Socket = Box::new(socket);
    if let Some(proxy) = setup.proxy {
        if proxy.tls {
            io = Box::new(
                tls.connect(&proxy.host, io)
                    .await
                    .map_err(|_| failure(ErrorCode::Trust))?,
            );
        }
        let host = if key.host.contains(':') {
            format!("[{}]", key.host)
        } else {
            key.host.clone()
        };
        let authority = format!("{host}:{}", key.port);
        let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
        if let Some(auth) = proxy.authorization {
            request.push_str("Proxy-Authorization: ");
            request.push_str(&auth);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        io.write_all(request.as_bytes())
            .await
            .map_err(|_| failure(ErrorCode::Io))?;
        // Do not consume bytes beyond the CONNECT header into a private TLS buffer.
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 65536 {
                return Err(failure(ErrorCode::Protocol));
            }
            header.push(io.read_u8().await.map_err(|_| failure(ErrorCode::Io))?);
        }
        let header = std::str::from_utf8(&header).map_err(|_| failure(ErrorCode::Protocol))?;
        if header.split("\r\n").count() > 103
            || header
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
        {
            return Err(failure(ErrorCode::Protocol));
        }
        for line in header.split("\r\n").skip(1).filter(|line| !line.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                return Err(failure(ErrorCode::Protocol));
            };
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || value.contains(['\r', '\n'])
            {
                return Err(failure(ErrorCode::Protocol));
            }
        }
        let mut first = header.lines().next().unwrap_or("").split_whitespace();
        if !matches!(first.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
            return Err(failure(ErrorCode::Protocol));
        }
        match first.next() {
            Some("200") => {}
            Some("407") => return Err(failure(ErrorCode::Authentication)),
            _ => return Err(failure(ErrorCode::Io)),
        }
    }
    let io = tls
        .connect(&key.host, io)
        .await
        .map_err(|_| failure(ErrorCode::Trust))?;
    let progress = Arc::new(AtomicU64::new(0));
    let io = super::https_progress::Tracked {
        io,
        count: progress.clone(),
    };
    let (sender, driver) = http1::Builder::new()
        .max_buf_size(65536)
        .max_headers(100)
        .handshake(TokioIo::new(io))
        .await
        .map_err(|_| failure(ErrorCode::Protocol))?;
    let driver = tokio::spawn(async move {
        let _ = driver.await;
    });
    Ok(Connection {
        sender,
        driver,
        progress,
    })
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
