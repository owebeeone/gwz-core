#![allow(dead_code, unused_imports)]

#[path = "../../../src/git/endpoint/ssh_channel.rs"]
mod ssh_channel;
#[path = "../../../src/git/endpoint/ssh_connection.rs"]
mod ssh_connection;
#[path = "../../../src/git/endpoint/ssh_destination.rs"]
mod ssh_destination;
#[path = "../../../src/git/endpoint/ssh_endpoint.rs"]
mod ssh_endpoint;
#[path = "../../../src/git/endpoint/ssh_pool.rs"]
mod ssh_pool;
#[path = "../../../src/git/endpoint/ssh_pump.rs"]
mod ssh_pump;
#[path = "../../../src/git/endpoint/ssh_remote.rs"]
mod ssh_remote;
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
#[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
mod ssh_shutdown;
#[path = "../../../src/git/endpoint/ssh_worker.rs"]
mod ssh_worker;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;

use gwz_transport::{
    pool::{Config as PoolConfig, Identity, Key},
    protocol::{Effect, ErrorCode, Failure},
    stream::{MessageEndpoint, Stream},
};
mod common;
use ssh_channel::GitService;
use ssh_endpoint::{IdentityResolver, Route};
use ssh_pool::{Connector, Resource};
use ssh_pump::{ChannelIo, SshPump};
use ssh_remote::OpenStream;
use ssh_worker::{ChannelResource, Endpoint};
use std::{
    io::{self, Read},
    net::TcpStream,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

fn commit(repo: &git2::Repository) -> git2::Oid {
    let blob = repo.blob(b"worker route").unwrap();
    let mut tree = repo.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(tree.write().unwrap()).unwrap();
    let signature = git2::Signature::now("worker", "worker@example.invalid").unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<_> = parent.iter().collect();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "worker",
        &tree,
        &parents,
    )
    .unwrap()
}

struct AmbientResolver;
impl IdentityResolver for AmbientResolver {
    fn resolve(&self, _: &Key) -> io::Result<Identity> {
        Ok(Identity::Ambient)
    }
}

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
    let socket = TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
    let mut connection = ssh_connection::SshConnection::new(socket).unwrap();
    {
        let session = connection.session();
        session.set_timeout(5_000);
        session.handshake().unwrap();
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

#[test]
fn worker_reads_native_upload_pack_and_endpoint_clone_keeps_worker_alive() {
    let mut fixture = common::SshdFixture::new();
    let session = authenticated(&mut fixture);
    let endpoint = Endpoint::new(
        config(1),
        NativeConnector {
            sessions: vec![session],
        },
        5_000,
    )
    .unwrap();
    let clone = endpoint.clone();
    let route = Route::new(clone, Arc::new(AmbientResolver));
    drop(endpoint);
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        fixture.repository.display()
    );
    let mut stream = route.open(&url, GitService::UploadPack).unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let packet_len = usize::from_str_radix(std::str::from_utf8(&header).unwrap(), 16).unwrap();
    assert!((4..=65_520).contains(&packet_len));
    let mut advertisement = vec![0; packet_len - 4];
    stream.read_exact(&mut advertisement).unwrap();
    assert!(!advertisement.is_empty());
    drop(stream);
    drop(route);
}

#[test]
fn failed_exchange_isolated_from_a_second_native_connection() {
    let mut fixture = common::SshdFixture::new();
    let first = authenticated(&mut fixture);
    let second = authenticated(&mut fixture);
    let endpoint = Endpoint::new(
        config(2),
        NativeConnector {
            sessions: vec![second, first],
        },
        5_000,
    )
    .unwrap();
    let refused = endpoint.open(
        key(&fixture),
        Identity::Ambient,
        GitService::UploadPack,
        "reject",
    );
    assert!(matches!(refused, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    let mut stream = endpoint
        .open(
            key(&fixture),
            Identity::Ambient,
            GitService::UploadPack,
            fixture.repository.to_str().unwrap(),
        )
        .unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    assert!(header.iter().all(u8::is_ascii_hexdigit));
    drop(stream);
    endpoint.shutdown();
}

#[test]
fn route_drives_git_push_clone_and_fetch_through_one_shared_connection() {
    use git2::{FetchOptions, PushOptions, Repository, build::RepoBuilder};
    let mut fixture = common::SshdFixture::new();
    let session = authenticated(&mut fixture);
    let endpoint = Endpoint::new(
        config(1),
        NativeConnector {
            sessions: vec![session],
        },
        5_000,
    )
    .unwrap();
    let route = Arc::new(Route::new(endpoint, Arc::new(AmbientResolver)));
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        fixture.repository.display()
    );
    let source = Repository::init(fixture.temp.path().join("source")).unwrap();
    let expected = commit(&source);
    let mut remote = source.remote_anonymous(&url).unwrap();
    let mut push = PushOptions::new();
    push.remote_callbacks(ssh_remote::callbacks(route.clone()));
    remote
        .push(&["refs/heads/main:refs/heads/main"], Some(&mut push))
        .unwrap();
    remote.disconnect().unwrap();
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(ssh_remote::callbacks(route.clone()));
    let clone_path = fixture.temp.path().join("clone");
    let clone = RepoBuilder::new()
        .fetch_options(fetch)
        .clone(&url, &clone_path)
        .unwrap();
    assert_eq!(clone.head().unwrap().target(), Some(expected));
    let updated = commit(&source);
    assert_ne!(expected, updated);
    remote
        .push(&["refs/heads/main:refs/heads/main"], Some(&mut push))
        .unwrap();
    remote.disconnect().unwrap();
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(ssh_remote::callbacks(route.clone()));
    let mut remote = clone.find_remote("origin").unwrap();
    remote
        .fetch(
            &["refs/heads/main:refs/remotes/origin/main"],
            Some(&mut fetch),
            None,
        )
        .unwrap();
    assert_eq!(
        clone
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        Some(updated)
    );
    assert!(clone.find_commit(updated).is_ok());
    drop(route);
}

#[test]
fn pending_request_holds_admission_until_completion() {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let endpoint = Endpoint::new(
        PoolConfig {
            total: 1,
            per_host: 1,
            per_user_host: 1,
            max_requests: 1,
            allocation_timeout_ms: 100,
            connect_timeout_ms: 100,
            interaction_timeout_ms: 100,
            cleanup_timeout_ms: 10,
            ..PoolConfig::default()
        },
        PendingConnector {
            started: started.clone(),
            ready: ready.clone(),
        },
        100,
    )
    .unwrap();
    let waiting = endpoint.clone();
    let first = std::thread::spawn(move || {
        waiting.open(
            Key::ssh("git", "127.0.0.1", 22),
            Identity::Ambient,
            GitService::UploadPack,
            "repo",
        )
    });
    await_started(&started);
    let second = endpoint.open(
        Key::ssh("git", "127.0.0.1", 22),
        Identity::Ambient,
        GitService::UploadPack,
        "repo",
    );
    assert!(matches!(second, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    ready.store(true, std::sync::atomic::Ordering::Release);
    assert!(first.join().unwrap().is_err());
}

#[test]
fn shutdown_wakes_an_unbounded_connect_request_when_network_timeout_is_disabled() {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let endpoint = Endpoint::new(
        PoolConfig {
            total: 1,
            per_host: 1,
            per_user_host: 1,
            max_requests: 1,
            connect_timeout_ms: 0,
            allocation_timeout_ms: 100,
            interaction_timeout_ms: 100,
            cleanup_timeout_ms: 10,
            ..PoolConfig::default()
        },
        PendingConnector {
            ready,
            started: started.clone(),
        },
        100,
    )
    .unwrap();
    let waiting = endpoint.clone();
    let first = std::thread::spawn(move || {
        waiting.open(
            Key::ssh("git", "127.0.0.1", 22),
            Identity::Ambient,
            GitService::UploadPack,
            "repo",
        )
    });
    await_started(&started);
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        !first.is_finished(),
        "disabled connect timeout must remain pending"
    );
    endpoint.shutdown();
    assert!(first.join().unwrap().is_err());
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
    let endpoint = Endpoint::new(
        config(2),
        NativeConnector {
            sessions: vec![first, second],
        },
        5_000,
    )
    .unwrap();
    let mut healthy = endpoint
        .open(
            key(&fixture),
            Identity::Ambient,
            GitService::UploadPack,
            fixture.repository.to_str().unwrap(),
        )
        .unwrap();
    let missing = fixture.temp.path().join("does-not-exist.git");
    let mut failed = endpoint
        .open(
            key(&fixture),
            Identity::Ambient,
            GitService::UploadPack,
            missing.to_str().unwrap(),
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
