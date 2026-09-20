//! Controlled local composition host; production credential/routing policy is
//! deliberately not implemented by this fixture. Connections arrive authenticated.
use crate::{
    ssh_channel::{GitService, SshChannel},
    ssh_connection::SshConnection,
    ssh_pool::{Connector, PoolHost, Resource},
    ssh_pump::SshPump,
    ssh_remote::OpenStream,
    stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Checkout, Config as PoolConfig, Identity, Key, Lease, Owner, Request},
    protocol::{Disposition, Effect, ErrorCode, Failure},
    stream::{Config, MessageEndpoint, Side, Stream},
};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    path::PathBuf,
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Native {
    idle: Option<SshConnection>,
    pump: Option<SshPump<SshChannel>>,
}
impl Resource for Native {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Ready(Ok(Some(Identity::Ambient)))
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        if let Some(pump) = &mut self.pump {
            let result = if force {
                pump.force_dispose()
            } else {
                pump.poll_dispose()
            };
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
            {
                return Poll::Pending;
            }
            Poll::Ready(result)
        } else {
            self.idle.take();
            Poll::Ready(Ok(()))
        }
    }
    fn reusable(&self) -> bool {
        self.idle.is_some() && self.pump.is_none()
    }
}
struct Prepared {
    session: Option<SshConnection>,
    opens: Arc<AtomicUsize>,
}
impl Connector for Prepared {
    type Resource = Native;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Native, Failure> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.session
            .take()
            .map(|session| Native {
                idle: Some(session),
                pump: None,
            })
            .ok_or(Failure {
                code: ErrorCode::Io,
                effect: Effect::None,
            })
    }
}
struct Open {
    url: String,
    service: GitService,
    reply: SyncSender<io::Result<BlockingStream>>,
}
pub(crate) struct Endpoint {
    sender: SyncSender<Open>,
}
impl OpenStream for Endpoint {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream> {
        let (reply, result) = mpsc::sync_channel(1);
        self.sender
            .try_send(Open {
                url: url.to_owned(),
                service,
                reply,
            })
            .map_err(|_| io::Error::other("fixture endpoint stopped/full"))?;
        result
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "fixture allocation timeout"))?
    }
}
pub(crate) struct Harness {
    pub(crate) endpoint: Arc<Endpoint>,
    pub(crate) opens: Arc<AtomicUsize>,
    pub(crate) commands: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), String>>>,
}
struct Pending {
    checkout: Checkout,
    open: Open,
    path: PathBuf,
}
struct Active {
    lease: Lease,
    peer: MessageEndpoint,
}

impl Harness {
    pub(crate) fn new(
        session: SshConnection,
        user: String,
        port: u16,
        routes: BTreeMap<String, PathBuf>,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Open>(32);
        let endpoint = Arc::new(Endpoint { sender });
        let opens = Arc::new(AtomicUsize::new(0));
        let commands = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let command_count = commands.clone();
        let prepared = Prepared {
            session: Some(session),
            opens: opens.clone(),
        };
        let worker = thread::spawn(move || {
            let (pool, mut host) = PoolHost::new(
                PoolConfig {
                    total: 1,
                    per_host: 1,
                    per_user_host: 1,
                    ..PoolConfig::default()
                },
                prepared,
                0,
            )
            .map_err(|e| e.to_string())?;
            let mut cx = Context::from_waker(Waker::noop());
            let origin = Instant::now();
            let mut pending = Vec::<Pending>::new();
            let mut active = Vec::<Active>::new();
            let mut serial = 0;
            let mut stopping = false;
            loop {
                let now = origin.elapsed().as_millis() as u64;
                if now > 45_000 {
                    return Err("fixture worker watchdog expired".into());
                }
                if flag.load(Ordering::SeqCst) && !stopping {
                    pool.shutdown();
                    stopping = true;
                }
                host.step(&mut cx, now).map_err(|e| e.to_string())?;
                if stopping && host.shutdown_complete() {
                    return Ok(());
                }
                for open in receiver.try_iter().take(32) {
                    let Some(path) = routes.get(&open.url).cloned() else {
                        let _ = open
                            .reply
                            .send(Err(io::Error::other("unbound fixture route")));
                        continue;
                    };
                    serial += 1;
                    let checkout = pool
                        .checkout(Request::new(
                            Key::ssh(&user, "127.0.0.1", port),
                            Identity::Ambient,
                            Owner::new("fixture", serial.to_string()),
                        ))
                        .map_err(|e| e.to_string())?;
                    pending.push(Pending {
                        checkout,
                        open,
                        path,
                    });
                }
                let mut index = 0;
                while index < pending.len() {
                    match pin!(&mut pending[index].checkout).poll(&mut cx) {
                        Poll::Pending => index += 1,
                        Poll::Ready(result) => {
                            let pending = pending.swap_remove(index);
                            match result {
                                Err(error) => {
                                    let _ = pending.open.reply.send(Err(io::Error::other(error)));
                                }
                                Ok(lease) => {
                                    let id =
                                        command_count.fetch_add(1, Ordering::SeqCst) as i64 + 1;
                                    let config = |side| {
                                        let mut c = Config::new("fixture", id, side);
                                        c.receive_window = 4096;
                                        c.peer_receive_window = 4096;
                                        c.send_buffer = 4096;
                                        c.max_payload = 1024;
                                        c.coalesce_delay_ms = 2;
                                        c.io_timeout_ms = 10_000;
                                        c
                                    };
                                    let (client, peer) =
                                        Stream::new(config(Side::Initiator)).unwrap();
                                    peer.advance(now);
                                    let (stream, endpoint) =
                                        Stream::new(config(Side::Endpoint)).unwrap();
                                    endpoint.advance(now);
                                    let resource =
                                        host.resource(&lease).map_err(|e| e.to_string())?;
                                    let channel = SshChannel::new(
                                        resource.idle.take().unwrap(),
                                        pending.open.service,
                                        pending.path.to_str().unwrap(),
                                    )
                                    .map_err(|e| e.to_string())?;
                                    resource.pump =
                                        Some(SshPump::new(stream, endpoint, channel, 4096, 4096));
                                    active.push(Active { lease, peer });
                                    let _ =
                                        pending.open.reply.send(Ok(BlockingStream::new(client)));
                                }
                            }
                        }
                    }
                }
                let mut index = 0;
                while index < active.len() {
                    let exchange = &mut active[index];
                    exchange.peer.advance(now);
                    if exchange.lease.connection().is_err() {
                        active.swap_remove(index);
                        continue;
                    }
                    let resource = host.resource(&exchange.lease).map_err(|e| e.to_string())?;
                    let pump = resource.pump.as_mut().unwrap();
                    for _ in 0..8 {
                        let message = match pin!(exchange.peer.next_message()).poll(&mut cx) {
                            Poll::Ready(Ok(Some(message))) => message,
                            Poll::Ready(Err(error)) => return Err(error.to_string()),
                            _ => break,
                        };
                        pump.deliver(message)
                            .map_err(|error| format!("deliver: {error:?}"))?;
                    }
                    pump.tick(&mut cx, now)
                        .map_err(|e| format!("pump: {e:?}"))?;
                    for _ in 0..8 {
                        match pump.poll_next_message(&mut cx) {
                            Poll::Ready(Ok(Some(message))) => {
                                exchange.peer.deliver(message).map_err(|e| e.to_string())?
                            }
                            Poll::Ready(Err(error)) => return Err(error.to_string()),
                            _ => break,
                        }
                    }
                    if pump.stream_stats().terminal {
                        let disposition = match resource.pump.take().unwrap().into_owner() {
                            Ok(session) => {
                                resource.idle = Some(session);
                                Disposition::Reusable
                            }
                            Err(pump) => {
                                resource.pump = Some(pump);
                                Disposition::Discarded
                            }
                        };
                        let exchange = active.swap_remove(index);
                        host.release(exchange.lease, disposition)
                            .map_err(|e| e.to_string())?;
                    } else {
                        index += 1;
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            endpoint,
            opens,
            commands,
            stop,
            worker: Some(worker),
        }
    }
    pub(crate) fn finish(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            if let Ok(Err(error)) = worker.join() {
                eprintln!("fixture worker: {error}");
            }
        }
    }
}
