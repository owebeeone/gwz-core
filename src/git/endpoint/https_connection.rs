//! Endpoint-owned TLS and a single Hyper HTTP/1 connection; no secondary pool.
use super::{
    agent_job::Job,
    ssh_pool::{Connector, Resource},
};
use bytes::Bytes;
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{Effect, ErrorCode, Failure, Scheme, SetupFailureCause},
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
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{AcquireError, Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn failure(code: ErrorCode) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        code,
        effect: Effect::None,
        facts: None,
    }
}
fn failure_from_io(error: &io::Error) -> Failure {
    let (code, setup_cause) = match error.kind() {
        io::ErrorKind::ConnectionRefused => (
            ErrorCode::Unavailable,
            Some(SetupFailureCause::ConnectionRefused),
        ),
        io::ErrorKind::NotFound => (ErrorCode::Unavailable, Some(SetupFailureCause::NotFound)),
        io::ErrorKind::AddrNotAvailable => (
            ErrorCode::Unavailable,
            Some(SetupFailureCause::AddressNotAvailable),
        ),
        _ => (ErrorCode::Io, None),
    };
    Failure {
        code,
        detail: None,
        setup_cause,
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
    /// The CA file's certificates (`super::ca_bundle`), each added as a root
    /// beside the platform's built-in roots.
    pub(crate) ca_roots: Vec<native_tls::Certificate>,
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
    /// Closed when the driver ends: Hyper reads the idle socket itself and
    /// ends on EOF, a reset or unexpected bytes. Taken by the resource.
    ended: Option<oneshot::Receiver<()>>,
    pub(crate) progress: Arc<AtomicU64>,
    pub(crate) binding: Option<gwz_sspi::SecretBytes>,
}
impl Connection {
    pub(crate) fn alive(&self) -> bool {
        !self.driver.is_finished()
    }
}
/// A connection's setup, queued for one of the connector's setup slots.
struct QueuedSetup {
    slot: Pin<Box<dyn Future<Output = Result<OwnedSemaphorePermit, AcquireError>> + Send>>,
    config: Config,
}
pub(crate) struct HttpResource {
    identity: Identity,
    queued: Option<QueuedSetup>,
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
    pub(crate) connect_started: Instant,
    /// The connection's driver end (`Connection::ended`), until it is seen.
    ended: Option<oneshot::Receiver<()>>,
    driver_ended: bool,
}
struct Setup {
    addresses: Vec<SocketAddr>,
    tls: native_tls::TlsConnector,
    proxy: Option<Proxy>,
}
pub(crate) struct HttpConnector {
    pub(crate) config: Config,
    pub(crate) epoch: Instant,
    /// Bounds the blocking resolver and TLS-configuration jobs. A connection
    /// past them waits for a slot (`HttpResource::poll_slot`); it is not refused.
    pub(crate) setup_slots: Arc<Semaphore>,
}
impl Connector for HttpConnector {
    type Resource = HttpResource;
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<HttpResource, Failure> {
        self.config.validate()?;
        if key.scheme != Scheme::Https {
            return Err(failure(ErrorCode::UnsupportedOperation));
        }
        let deadline = deadline.and_then(|ms| self.epoch.checked_add(Duration::from_millis(ms)));
        let mut resource = HttpResource {
            identity: identity.clone(),
            queued: Some(QueuedSetup {
                slot: Box::pin(self.setup_slots.clone().acquire_owned()),
                config: self.config.clone(),
            }),
            setup: None,
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
            ended: None,
            driver_ended: false,
        };
        // Queue now, in the pool's order, and start at once when a slot is free.
        if let Poll::Ready(Err(failed)) =
            resource.poll_slot(&mut Context::from_waker(Waker::noop()))
        {
            return Err(failed);
        }
        Ok(resource)
    }
}
impl HttpResource {
    /// Starts the setup job once one of the connector's setup slots is free.
    /// The slots bound the blocking resolver and TLS-configuration jobs, not
    /// the connections: a connection past them waits here, within its connect
    /// deadline, and the pool's per-host and total ceilings decide how many
    /// connections exist.
    fn poll_slot(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Failure>> {
        let Some(queued) = &mut self.queued else {
            return Poll::Ready(Ok(()));
        };
        if self.deadline.is_some_and(|at| Instant::now() >= at) {
            return Poll::Ready(Err(Failure {
                setup_cause: Some(SetupFailureCause::Aggregate),
                ..failure(ErrorCode::Timeout)
            }));
        }
        let permit = match queued.slot.as_mut().poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(permit)) => permit,
            Poll::Ready(Err(_)) => return Poll::Ready(Err(failure(ErrorCode::Cancelled))),
        };
        let config = self.queued.take().expect("queued setup").config;
        let key = self.key.clone();
        let setup = Job::start(self.deadline, Duration::from_secs(5), move |control| {
            let _permit = permit;
            control.check()?;
            let proxy = config.proxy_for(&key.host);
            let (host, port) = proxy
                .as_ref()
                .map_or((key.host.as_str(), key.port), |p| (p.host.as_str(), p.port));
            let addresses = (host, port).to_socket_addrs()?.take(16).collect::<Vec<_>>();
            control.check()?;
            let mut builder = native_tls::TlsConnector::builder();
            for root in config.ca_roots {
                builder.add_root_certificate(root);
            }
            let tls = builder.build().map_err(|_| io::ErrorKind::InvalidInput)?;
            Ok(Setup {
                addresses,
                tls,
                proxy,
            })
        })
        .map_err(|_| failure(ErrorCode::Capacity))?;
        self.setup = Some(setup);
        Poll::Ready(Ok(()))
    }
}
impl Resource for HttpResource {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        if let Poll::Ready(Err(failed)) = self.poll_slot(cx) {
            return Poll::Ready(Err(failed));
        }
        if self.queued.is_some() {
            return Poll::Pending;
        }
        if let Some(setup) = &mut self.setup {
            let result = match setup.poll_result(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.setup = None;
            let setup = match result {
                Ok(setup) => setup,
                Err(error) => {
                    let failed = if let Some(reason) = super::agent_job::timeout_reason(&error) {
                        Failure {
                            setup_cause: Some(reason.setup_cause()),
                            ..failure(ErrorCode::Timeout)
                        }
                    } else {
                        failure_from_io(&error)
                    };
                    return Poll::Ready(Err(failed));
                }
            };
            let key = self.key.clone();
            let cancelled = self.cancel.clone();
            let deadline = self.deadline;
            self.connecting = Some(tokio::spawn(async move {
                tokio::select! {
                    _=cancelled.cancelled()=>Err(failure(ErrorCode::Cancelled)),
                    _=async {if let Some(at)=deadline {tokio::time::sleep_until(at.into()).await;} else {std::future::pending::<()>().await;}}=>Err(Failure { setup_cause: Some(SetupFailureCause::Aggregate), ..failure(ErrorCode::Timeout) }),
                    result=connect(setup,key)=>result,
                }
            }));
        }
        if let Some(task) = &mut self.connecting {
            match Pin::new(task).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    self.connecting = None;
                    let mut connection = match result {
                        Ok(Ok(c)) => c,
                        Ok(Err(e)) => return Poll::Ready(Err(e)),
                        Err(_) => return Poll::Ready(Err(failure(ErrorCode::Io))),
                    };
                    self.ended = connection.ended.take();
                    self.connect_elapsed = self.connect_started.elapsed();
                    self.driver_abort = Some(connection.driver.abort_handle());
                    self.connection = Some(Arc::new(Mutex::new(connection)));
                    self.reusable.store(true, Ordering::Release);
                }
            }
        }
        // HTTPS proves the TLS resource identity, not an authenticated account.
        // The generic pool requires this proof before admitting idle reuse.
        Poll::Ready(Ok(Some(self.identity.clone())))
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>, _force: bool) -> Poll<io::Result<()>> {
        self.cancel.cancel();
        self.reusable.store(false, Ordering::Release);
        self.queued = None;
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
    /// Idle is the `reusable` flag: cleared when a lease adopts the
    /// connection, set again only by a reusable release.
    fn poll_idle_lost(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(ended) = &mut self.ended
            && Pin::new(ended).poll(cx).is_ready()
        {
            self.ended = None;
            self.driver_ended = true;
        }
        if self.driver_ended && self.connection.is_some() && self.reusable.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
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
    let mut last_error = None;
    for address in setup.addresses {
        match TcpStream::connect(address).await {
            Ok(connected) => {
                socket = Some(connected);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let socket = socket.ok_or_else(|| {
        last_error
            .as_ref()
            .map_or_else(|| failure(ErrorCode::Io), failure_from_io)
    })?;
    socket
        .set_nodelay(true)
        .map_err(|_| failure(ErrorCode::Io))?;
    let tls = tokio_native_tls::TlsConnector::from(setup.tls);
    let mut io: Socket = Box::new(socket);
    if let Some(proxy) = setup.proxy {
        if proxy.tls {
            io = Box::new(super::https_handshake::handshake(&tls, &proxy.host, io).await?);
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
    let io = super::https_handshake::handshake(&tls, &key.host, io).await?;
    // Only the verified final origin TLS stream supplies CBT, after any proxy TLS.
    let binding = io
        .get_ref()
        .tls_server_end_point()
        .ok()
        .flatten()
        .and_then(|mut bytes| super::https_auth::SecretHeader::channel_binding_digest(&mut bytes));
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
    let (end, ended) = oneshot::channel();
    let driver = tokio::spawn(async move {
        // Dropped when the task ends, aborted or not.
        let _end: oneshot::Sender<()> = end;
        let _ = driver.await;
    });
    Ok(Connection {
        sender,
        driver,
        ended: Some(ended),
        progress,
        binding,
    })
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod setup_cause_tests {
            use super::*;

            #[test]
            fn native_connect_kinds_keep_typed_origins() {
                for (kind, cause) in [
                    (io::ErrorKind::ConnectionRefused, SetupFailureCause::ConnectionRefused),
                    (io::ErrorKind::NotFound, SetupFailureCause::NotFound),
                    (io::ErrorKind::AddrNotAvailable, SetupFailureCause::AddressNotAvailable),
                ] {
                    let failure = failure_from_io(&io::Error::from(kind));
                    assert_eq!(failure.code, ErrorCode::Unavailable);
                    assert_eq!(failure.setup_cause, Some(cause));
                }
                let unknown = failure_from_io(&io::Error::from(io::ErrorKind::BrokenPipe));
                assert_eq!(unknown.code, ErrorCode::Io);
                assert_eq!(unknown.setup_cause, None);
            }
        }
    }
}
