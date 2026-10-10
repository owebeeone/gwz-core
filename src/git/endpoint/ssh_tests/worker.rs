//! The worker over pre-authenticated fixture connections, through the
//! attachment path the placement endpoint takes.
use super::attachment;
use crate::git::endpoint::{
    ssh_channel::{self, GitService},
    ssh_connection, ssh_fixture as common,
    ssh_key_snapshot::Registry,
    ssh_pool::{Connector, Resource},
    ssh_pump::SshPump,
    ssh_remote::{OpenStream, RemoteTransport},
    ssh_worker::{ChannelResource, Endpoint},
    stream_io::BlockingStream,
};
use git2::{PushOptions, RemoteCallbacks, Repository, Signature};
use gwz_transport::{
    pool::{Config as PoolConfig, Identity, Key},
    protocol::{Deadlines, Effect, ErrorCode, Failure},
    stream::{MessageEndpoint, Stream},
};
use std::{
    io::{self, Read},
    net::TcpStream,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

struct NativeResource {
    idle: Option<ssh_connection::SshConnection>,
    pump: Option<SshPump<ssh_channel::SshChannel>>,
}
struct NativeConnector {
    sessions: Vec<ssh_connection::SshConnection>,
}
impl Connector for NativeConnector {
    type Resource = NativeResource;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Self::Resource, Failure> {
        self.sessions
            .pop()
            .map(|session| NativeResource {
                idle: Some(session),
                pump: None,
            })
            .ok_or(Failure {
                detail: None,
                setup_cause: None,
                facts: None,
                code: ErrorCode::Io,
                effect: Effect::None,
            })
    }
}
impl Resource for NativeResource {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Ready(Ok(Some(Identity::Ambient)))
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
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
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            self.pump = None;
            return Poll::Ready(result);
        }
        self.idle.take();
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        self.idle.is_some() && self.pump.is_none()
    }
}
impl ChannelResource for NativeResource {
    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: GitService,
        path: &str,
    ) -> io::Result<()> {
        if path == "reject" {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture rejection",
            ));
        }
        let session = self
            .idle
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "resource is active"))?;
        let channel = ssh_channel::SshChannel::new(session, service, path)?;
        self.pump = Some(SshPump::new(stream, endpoint, channel, 65_536, 65_536));
        Ok(())
    }
    fn pump(&mut self) -> Option<&mut SshPump<ssh_channel::SshChannel>> {
        self.pump.as_mut()
    }
    fn reclaim(&mut self) -> bool {
        let Some(pump) = self.pump.take() else {
            return false;
        };
        match pump.into_owner() {
            Ok(session) => {
                self.idle = Some(session);
                true
            }
            Err(pump) => {
                self.pump = Some(pump);
                false
            }
        }
    }
}

struct PendingResource {
    ready: Arc<std::sync::atomic::AtomicBool>,
}
struct PendingConnector {
    started: Arc<std::sync::atomic::AtomicBool>,
    ready: Arc<std::sync::atomic::AtomicBool>,
}
impl Connector for PendingConnector {
    type Resource = PendingResource;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Self::Resource, Failure> {
        self.started
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(PendingResource {
            ready: self.ready.clone(),
        })
    }
}
impl Resource for PendingResource {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        if self.ready.load(std::sync::atomic::Ordering::Acquire) {
            Poll::Ready(Ok(Some(Identity::Ambient)))
        } else {
            Poll::Pending
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for PendingResource {
    fn start_exchange(
        &mut self,
        _: Stream,
        _: MessageEndpoint,
        _: GitService,
        _: &str,
    ) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "pending fixture"))
    }
    fn pump(&mut self) -> Option<&mut SshPump<ssh_channel::SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}

fn authenticated(fixture: &mut common::SshdFixture) -> ssh_connection::SshConnection {
    use ssh2::{CheckResult, KnownHostFileKind};
    let (mut connection, _) = common::handshaken(fixture.port, 5_000).unwrap();
    {
        let session = connection.session();
        let (key, _) = session.host_key().unwrap();
        let mut known = session.known_hosts().unwrap();
        known
            .read_file(&fixture.known_hosts, KnownHostFileKind::OpenSSH)
            .unwrap();
        assert!(matches!(
            known.check_port("127.0.0.1", fixture.port, key),
            CheckResult::Match
        ));
        session
            .userauth_pubkey_file(
                &fixture.user,
                None,
                &fixture.temp.path().join("client_ed25519"),
                None,
            )
            .unwrap();
        assert!(session.authenticated());
    }
    connection.set_nonblocking().unwrap();
    connection
}

fn config(max_requests: usize) -> PoolConfig {
    PoolConfig {
        total: max_requests,
        per_host: max_requests,
        per_user_host: max_requests,
        max_requests,
        ..PoolConfig::default()
    }
}

fn key(fixture: &common::SshdFixture) -> Key {
    Key::ssh(&fixture.user, "127.0.0.1", fixture.port)
}

/// An endpoint over `connector`, built as production builds one.
fn endpoint<C>(config: PoolConfig, connector: C, io_timeout_ms: u64) -> Endpoint
where
    C: Connector + Send + 'static,
    C::Resource: ChannelResource,
{
    Endpoint::with_registry(config, Registry::new(), |_, _| connector, io_timeout_ms).unwrap()
}

#[test]
fn worker_reads_native_upload_pack_and_endpoint_clone_keeps_worker_alive() {
    let mut fixture = common::SshdFixture::new();
    let session = authenticated(&mut fixture);
    let endpoint = endpoint(
        config(1),
        NativeConnector {
            sessions: vec![session],
        },
        5_000,
    );
    let clone = endpoint.clone();
    drop(endpoint);
    let (mut stream, _) = attachment::open(
        &clone,
        key(&fixture),
        None,
        GitService::UploadPack,
        fixture.repository.to_str().unwrap(),
        attachment::deadlines(&config(1), 5_000),
    )
    .unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let packet_len = usize::from_str_radix(std::str::from_utf8(&header).unwrap(), 16).unwrap();
    assert!((4..=65_520).contains(&packet_len));
    let mut advertisement = vec![0; packet_len - 4];
    stream.read_exact(&mut advertisement).unwrap();
    assert!(!advertisement.is_empty());
    drop(stream);
    drop(clone);
}

#[test]
fn failed_exchange_isolated_from_a_second_native_connection() {
    let mut fixture = common::SshdFixture::new();
    let first = authenticated(&mut fixture);
    let second = authenticated(&mut fixture);
    let endpoint = endpoint(
        config(2),
        NativeConnector {
            sessions: vec![second, first],
        },
        5_000,
    );
    let deadlines = attachment::deadlines(&config(2), 5_000);
    let refused = attachment::open(
        &endpoint,
        key(&fixture),
        None,
        GitService::UploadPack,
        "reject",
        deadlines.clone(),
    );
    assert!(matches!(refused, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    let (mut stream, _) = attachment::open(
        &endpoint,
        key(&fixture),
        None,
        GitService::UploadPack,
        fixture.repository.to_str().unwrap(),
        deadlines,
    )
    .unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    assert!(header.iter().all(u8::is_ascii_hexdigit));
    drop(stream);
    endpoint.shutdown();
}

#[test]
fn pending_request_holds_admission_until_completion() {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let config = PoolConfig {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        max_requests: 1,
        allocation_timeout_ms: 100,
        connect_timeout_ms: 100,
        interaction_timeout_ms: 100,
        cleanup_timeout_ms: 10,
        ..PoolConfig::default()
    };
    let endpoint = endpoint(
        config.clone(),
        PendingConnector {
            started: started.clone(),
            ready: ready.clone(),
        },
        100,
    );
    let open = || {
        attachment::start(
            &endpoint,
            Key::ssh("git", "127.0.0.1", 22),
            None,
            GitService::UploadPack,
            "repo",
            attachment::deadlines(&config, 100),
        )
    };
    let first = open().unwrap();
    await_started(&started);
    // The worker refuses the second open as it is submitted, while the first
    // holds the endpoint's only admission.
    let second = open();
    assert!(matches!(second, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    ready.store(true, std::sync::atomic::Ordering::Release);
    assert!(attachment::finish(&first).is_err());
}

#[test]
fn shutdown_wakes_an_unbounded_connect_request_when_network_timeout_is_disabled() {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let config = PoolConfig {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        max_requests: 1,
        connect_timeout_ms: 0,
        allocation_timeout_ms: 100,
        interaction_timeout_ms: 100,
        cleanup_timeout_ms: 10,
        ..PoolConfig::default()
    };
    let endpoint = endpoint(
        config.clone(),
        PendingConnector {
            ready,
            started: started.clone(),
        },
        100,
    );
    let first = attachment::start(
        &endpoint,
        Key::ssh("git", "127.0.0.1", 22),
        None,
        GitService::UploadPack,
        "repo",
        attachment::deadlines(&config, 100),
    )
    .unwrap();
    await_started(&started);
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        first.poll().is_pending(),
        "disabled connect timeout must remain pending"
    );
    endpoint.shutdown();
    assert!(attachment::finish(&first).is_err());
}

fn await_started(started: &std::sync::atomic::AtomicBool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !started.load(std::sync::atomic::Ordering::Acquire) {
        assert!(
            std::time::Instant::now() < deadline,
            "worker failed to start checkout"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn failed_active_service_does_not_stop_another_active_stream() {
    let mut fixture = common::SshdFixture::new();
    let first = authenticated(&mut fixture);
    let second = authenticated(&mut fixture);
    let endpoint = endpoint(
        config(2),
        NativeConnector {
            sessions: vec![first, second],
        },
        5_000,
    );
    let deadlines = attachment::deadlines(&config(2), 5_000);
    let (mut healthy, _) = attachment::open(
        &endpoint,
        key(&fixture),
        None,
        GitService::UploadPack,
        fixture.repository.to_str().unwrap(),
        deadlines.clone(),
    )
    .unwrap();
    let missing = fixture.temp.path().join("does-not-exist.git");
    let (mut failed, _) = attachment::open(
        &endpoint,
        key(&fixture),
        None,
        GitService::UploadPack,
        missing.to_str().unwrap(),
        deadlines,
    )
    .unwrap();
    let mut discarded = Vec::new();
    failed.end_write().unwrap();
    // EOF may arrive before service status; close must reject failed cleanup.
    let _ = failed.read_to_end(&mut discarded);
    assert!(failed.close().is_err());
    let mut header = [0; 4];
    healthy.read_exact(&mut header).unwrap();
    assert!(header.iter().all(u8::is_ascii_hexdigit));
    endpoint.shutdown();
}

/// Opens every Git stream on one repository of the fixture through the
/// worker, as the placement endpoint's attachment path does.
struct WorkerStreams {
    endpoint: Endpoint,
    key: Key,
    repository: String,
    deadlines: Deadlines,
}
impl OpenStream for WorkerStreams {
    fn open(&self, _: &str, service: GitService) -> io::Result<BlockingStream> {
        attachment::open(
            &self.endpoint,
            self.key.clone(),
            None,
            service,
            &self.repository,
            self.deadlines.clone(),
        )
        .map(|(stream, _)| stream)
    }
}

/// A commit of `len` bytes that no pack compresses.
fn incompressible_commit(repo: &Repository, len: usize) -> git2::Oid {
    let mut state = 0x9e37_79b9_u32;
    let data: Vec<u8> = (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    let blob = repo.blob(&data).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = Signature::now("fixture", "fixture@example.invalid").unwrap();
    repo.set_head("refs/heads/main").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "payload", &tree, &[])
        .unwrap()
}

/// A push streams its pack to the server and then reads only the server's
/// report: the worker's endpoint end must neither lose nor stall what
/// follows the pack, whatever its coalescing. The time is printed for the
/// before and after of TR8.1's change to that end (TR8.1, 2026-10-07).
#[test]
fn a_large_push_through_the_worker_delivers_its_pack_and_the_servers_report() {
    let mut fixture = common::SshdFixture::new();
    let session = authenticated(&mut fixture);
    let endpoint = endpoint(
        config(1),
        NativeConnector {
            sessions: vec![session],
        },
        10_000,
    );
    let streams = Arc::new(WorkerStreams {
        key: key(&fixture),
        repository: fixture.repository.to_str().unwrap().to_owned(),
        deadlines: attachment::deadlines(&config(1), 10_000),
        endpoint: endpoint.clone(),
    });
    let source = Repository::init(fixture.temp.path().join("source")).unwrap();
    let pushed = incompressible_commit(&source, 4 * 1024 * 1024);
    let mut remote = source.remote_anonymous("ssh://worker/repository").unwrap();
    let mut callbacks = RemoteCallbacks::new();
    callbacks.smart_transport(false, move |_| Ok(RemoteTransport::new(streams.clone())));
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    let started = std::time::Instant::now();
    remote
        .push(&["refs/heads/main:refs/heads/main"], Some(&mut options))
        .unwrap();
    println!("push of 4 MiB through the worker: {:?}", started.elapsed());
    remote.disconnect().unwrap();
    let served = Repository::open_bare(&fixture.repository).unwrap();
    assert_eq!(
        served.find_reference("refs/heads/main").unwrap().target(),
        Some(pushed)
    );
    endpoint.shutdown();
}
