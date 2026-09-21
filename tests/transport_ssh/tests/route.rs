#![allow(dead_code)]
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
#[path = "../../../src/git/endpoint/ssh_key_auth.rs"]
mod ssh_key_auth;
#[path = "../../../src/git/endpoint/ssh_key_container.rs"]
mod ssh_key_container;
#[path = "../../../src/git/endpoint/ssh_key_snapshot.rs"]
mod ssh_key_snapshot;
#[path = "../../../src/git/endpoint/ssh_admission.rs"]
mod ssh_admission;
#[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
mod ssh_shutdown;
#[path = "../../../src/git/endpoint/ssh_worker.rs"]
mod ssh_worker;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;
use gwz_transport::{
    pool::{Config, Identity, Key},
    protocol::{Effect, ErrorCode, Failure},
    stream::{MessageEndpoint, Stream},
};
use ssh_channel::{GitService, SshChannel};
use ssh_endpoint::{IdentityResolver, Route};
use ssh_pool::{Connector, Resource};
use ssh_pump::SshPump;
use ssh_remote::OpenStream;
use ssh_worker::{ChannelResource, Endpoint};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
struct Counter {
    resolved: AtomicUsize,
    starts: AtomicUsize,
    eligible: AtomicBool,
}
impl IdentityResolver for Counter {
    fn resolve(&self, key: &Key) -> io::Result<Identity> {
        assert_eq!(key.host, "host");
        self.resolved.fetch_add(1, Ordering::SeqCst);
        if !self.eligible.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(Identity::Explicit("validated-public-proof".into()))
    }
}
struct Factory(Arc<Counter>);
struct Never;
impl Connector for Factory {
    type Resource = Never;
    fn start(&mut self, _: &Key, identity: &Identity, _: Option<u64>) -> Result<Never, Failure> {
        assert_eq!(
            identity,
            &Identity::Explicit("validated-public-proof".into())
        );
        self.0.starts.fetch_add(1, Ordering::SeqCst);
        Err(Failure {
            code: ErrorCode::Authentication,
            effect: Effect::None,
        })
    }
}
impl Resource for Never {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        unreachable!()
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        unreachable!()
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for Never {
    fn start_exchange(
        &mut self,
        _: Stream,
        _: MessageEndpoint,
        _: GitService,
        _: &str,
    ) -> io::Result<()> {
        unreachable!()
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}
#[test]
fn admission_precedes_identity_and_current_authority_is_resolved_on_every_open() {
    let counts = Arc::new(Counter {
        resolved: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        eligible: AtomicBool::new(false),
    });
    let endpoint = Endpoint::new(Config::default(), Factory(counts.clone()), 1000).unwrap();
    let route = Route::new(endpoint, counts.clone());
    for url in [
        "ssh://u:secret@host/repo",
        "https://host/repo",
        "C:\\repo",
        "C:/repo",
        "[host:0]:repo",
        "[host:65536]:repo",
        "[host:bad]:repo",
        "[host]:",
        "[host:repo",
        "host]:repo",
    ] {
        assert!(route.open(url, GitService::UploadPack).is_err());
    }
    assert_eq!(counts.resolved.load(Ordering::SeqCst), 0);
    assert!(route.open("git@host:repo", GitService::UploadPack).is_err());
    assert_eq!(counts.starts.load(Ordering::SeqCst), 0);
    counts.eligible.store(true, Ordering::SeqCst);
    assert!(
        route
            .open("git+ssh://git@host/repo", GitService::UploadPack)
            .is_err()
    );
    assert_eq!(counts.starts.load(Ordering::SeqCst), 1);
    counts.eligible.store(false, Ordering::SeqCst);
    assert!(route.open("git@host:repo", GitService::UploadPack).is_err());
    assert_eq!(counts.starts.load(Ordering::SeqCst), 1);
    assert_eq!(counts.resolved.load(Ordering::SeqCst), 3);
    counts.eligible.store(true, Ordering::SeqCst);
    for url in [
        "[host]:/resource",
        "[host:42]:/resource",
        "[git@host:42]:/resource",
        "host:/",
    ] {
        let before = counts.starts.load(Ordering::SeqCst);
        assert!(route.open(url, GitService::UploadPack).is_err()); // fixture authentication refusal
        assert_eq!(counts.starts.load(Ordering::SeqCst), before + 1, "{url}");
    }
}

#[test]
fn invalid_pool_configuration_is_refused_by_constructor() {
    let counts = Arc::new(Counter {
        resolved: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        eligible: AtomicBool::new(false),
    });
    let config = Config {
        total: 0,
        ..Config::default()
    };
    assert!(Endpoint::new(config, Factory(counts.clone()), 1000).is_err());
    assert_eq!(counts.starts.load(Ordering::SeqCst), 0);
}
