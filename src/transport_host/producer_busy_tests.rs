//! A push whose producer is busy between its stream calls, end to end
//! (GwzPushProducerBusyDesign §7): the driver's stream, the endpoint's pump and
//! a real `sshd` running `git receive-pack`. libgit2 reads a whole object
//! before it writes the object's first byte; here the test is the producer,
//! writes the commands and part of a real pack, and then stays outside every
//! call on its stream for longer than the stall limit.
use super::driver_tests::{block_on, common, endpoint_home, local_meta};
use super::*;
use crate::git::endpoint::{
    cut_proxy::CutProxy, ssh_channel::GitService, ssh_close_fixture::read_advertisement,
    stream_io::BlockingStream,
};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

/// The stall limit of these rows. Long enough that a loaded machine's `receive-pack` is not silent for it.
const STALL_MS: u64 = 1_000;
/// The producer ceiling of the rows that reach it.
const CEILING_MS: u64 = 2_000;

fn pkt(line: &str) -> Vec<u8> {
    let mut bytes = format!("{:04x}", line.len() + 4).into_bytes();
    bytes.extend_from_slice(line.as_bytes());
    bytes
}

/// A push of one commit holding about 600 KiB of bytes zlib cannot shrink: the commands that create
/// `refs/heads/pushed` on an empty server, and the pack `git pack-objects` makes for them.
struct Material {
    commands: Vec<u8>,
    pack: Vec<u8>,
    commit: git2::Oid,
}
impl Material {
    fn new(dir: &Path) -> Self {
        let repo = git2::Repository::init(dir).unwrap();
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let bytes: Vec<u8> = (0..600 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect();
        let blob = repo.blob(&bytes).unwrap();
        let mut tree = repo.treebuilder(None).unwrap();
        tree.insert("payload", blob, 0o100644).unwrap();
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
        let commit = repo
            .commit(None, &signature, &signature, "blob", &tree, &[])
            .unwrap();
        let mut packer = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["pack-objects", "--revs", "--stdout", "-q"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        packer
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{commit}\n").as_bytes())
            .unwrap();
        let pack = packer.wait_with_output().unwrap().stdout;
        let commands = [
            pkt(&format!(
                "{} {commit} refs/heads/pushed\0report-status\n",
                "0".repeat(40)
            )),
            b"0000".to_vec(),
        ]
        .concat();
        Self {
            commands,
            pack,
            commit,
        }
    }
}

/// An `sshd` fixture with a runtime and a request on it, behind a proxy that can freeze its client's bytes.
struct Rig {
    fixture: common::SshdFixture,
    proxy: Option<CutProxy>,
    runtime: TransportRuntime,
    request: TransportRequest,
    url: String,
    identity: String,
}
impl Rig {
    fn new(name: &str, frozen_server: bool, ceiling_ms: Option<u64>) -> Self {
        let fixture = common::SshdFixture::new();
        let home = endpoint_home(&fixture);
        let proxy = frozen_server.then(|| CutProxy::start(fixture.port));
        let port = proxy.as_ref().map_or(fixture.port, |proxy| proxy.port);
        if proxy.is_some() {
            std::fs::write(
                home.join(".ssh/known_hosts"),
                format!("{}\n", fixture.known_host("127.0.0.1", port, false)),
            )
            .unwrap();
        }
        let runtime = TransportRuntime::new(
            SshEndpointConfig::fixture(home.clone(), None).with_io_timeout_ms(STALL_MS),
        )
        .unwrap();
        if let Some(ceiling_ms) = ceiling_ms {
            runtime.set_producer_ceiling_ms_for_test(ceiling_ms);
        }
        let request = block_on(runtime.request(local_meta(name, &home), "push".into())).unwrap();
        let url = format!(
            "ssh://{}@127.0.0.1:{port}{}",
            fixture.user,
            common::server_path(&fixture.url_repository)
        );
        let identity = home.join("client_ed25519").to_string_lossy().into_owned();
        Self {
            fixture,
            proxy,
            runtime,
            request,
            url,
            identity,
        }
    }

    fn open(&self, service: GitService) -> BlockingStream {
        self.open_observing(service, Arc::new(|_, _| {}))
    }

    fn open_observing(
        &self,
        service: GitService,
        observe: Arc<dyn Fn(i64, &gwz_transport::protocol::Opened) + Send + Sync>,
    ) -> BlockingStream {
        self.request
            .context
            .open(
                &self.url,
                service,
                Some(self.identity.clone()),
                observe,
                Arc::new(|_| {}),
            )
            .unwrap()
    }

    /// The push is open and past the server's advertisement, with the commands and `part` of the pack
    /// written: all the server has seen when the producer's pause begins.
    fn push_started(&self, material: &Material, part: usize) -> BlockingStream {
        let mut stream = self.open(GitService::ReceivePack);
        read_advertisement(&mut stream);
        stream.write_all(&material.commands).unwrap();
        stream.write_all(&material.pack[..part]).unwrap();
        stream
    }

    fn finish(self) {
        let _ = block_on(self.request.finish());
        block_on(self.runtime.shutdown());
        drop(self.proxy);
        drop(self.fixture);
    }
}

/// How long after `since` the stream's retained failure appears, with the producer outside every call on it.
fn failure_after(stream: &BlockingStream, since: Instant, patience: Duration) -> Option<Duration> {
    while since.elapsed() < patience {
        if stream.retained_failure().is_some() {
            return Some(since.elapsed());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn a_push_whose_producer_pauses_past_the_stall_limit_between_calls_completes() {
    let rig = Rig::new("producer-busy-pause", false, Some(600_000));
    let material = Material::new(&rig.fixture.temp.path().join("local"));
    let part = material.pack.len() * 2 / 5;
    let mut stream = rig.push_started(&material, part);
    // libgit2 is reading the next object: no call on the stream, for several stall limits.
    thread::sleep(Duration::from_millis(STALL_MS * 7 / 2));
    stream
        .write_all(&material.pack[part..])
        .expect("the pause was the producer's, not the network's");
    let mut report = Vec::new();
    stream.read_to_end(&mut report).unwrap();
    let report = String::from_utf8_lossy(&report);
    assert!(report.contains("unpack ok"), "{report}");
    assert!(report.contains("ok refs/heads/pushed"), "{report}");
    stream.close().unwrap();
    let server = git2::Repository::open_bare(&rig.fixture.repository).unwrap();
    assert_eq!(
        server.find_reference("refs/heads/pushed").unwrap().target(),
        Some(material.commit)
    );
    rig.finish();
}

#[test]
fn a_frozen_server_is_found_at_the_ceiling_plus_the_stall_limit_while_the_producer_is_away() {
    let rig = Rig::new("producer-busy-frozen", true, Some(CEILING_MS));
    let material = Material::new(&rig.fixture.temp.path().join("local"));
    let part = material.pack.len() * 2 / 5;
    let mut stream = rig.push_started(&material, part);
    let last_byte = Instant::now();
    // The server stops answering and takes no more; the producer is away, outside every call.
    rig.proxy.as_ref().unwrap().freeze_existing();
    let limit = Duration::from_millis(CEILING_MS + STALL_MS);
    let found = failure_after(&stream, last_byte, limit + Duration::from_secs(10))
        .expect("a frozen server is found");
    assert!(
        found + Duration::from_millis(150) >= limit,
        "found after {found:?}, before the ceiling plus the stall limit {limit:?}: the ceiling did not hold"
    );
    assert!(
        found <= limit + Duration::from_secs(3),
        "found after {found:?}, long after the ceiling plus the stall limit {limit:?}"
    );
    // The producer wakes: the stream says so, in the words of a stall.
    let error = stream.write_all(&material.pack[part..]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
    assert!(error.to_string().contains("--ssh-timeout 1"), "{error}");
    rig.finish();
}

#[test]
fn a_producer_that_never_returns_is_given_up_on_and_its_connection_released() {
    let rig = Rig::new("producer-busy-hung", false, Some(CEILING_MS));
    let material = Material::new(&rig.fixture.temp.path().join("local"));
    let part = material.pack.len() * 2 / 5;
    let stream = rig.push_started(&material, part);
    let last_byte = Instant::now();
    let limit = Duration::from_millis(CEILING_MS + STALL_MS);
    // The server is alive and the producer is hung: no call, no byte, until the test gives up on it.
    let found = failure_after(&stream, last_byte, limit + Duration::from_secs(10))
        .expect("a hung producer is given up on");
    assert!(
        found + Duration::from_millis(150) >= limit,
        "given up on after {found:?}, before the ceiling plus the stall limit {limit:?}"
    );
    assert!(
        found <= limit + Duration::from_secs(3),
        "given up on after {found:?}, long after the ceiling plus the stall limit {limit:?}"
    );
    let endpoint = rig
        .runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    let counts = loop {
        let counts = endpoint.ssh_counts_for_test().unwrap();
        if counts.leased == 0 || Instant::now() >= deadline {
            break counts;
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(counts.leased, 0, "the connection is released: {counts:?}");
    stream.cancel();
    rig.finish();
}

#[test]
fn a_push_open_on_a_runtime_nothing_configured_carries_the_shipped_ceiling() {
    // No setter: the ceiling is the product's, ten minutes (decision D45).
    let rig = Rig::new("producer-busy-default-ceiling", false, None);
    let session = rig
        .runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local
        .clone();
    let material = Material::new(&rig.fixture.temp.path().join("local"));
    let push = rig.push_started(&material, 0);
    assert_eq!(session.producer_ceilings_for_test(), vec![600_000]);
    assert_eq!(
        crate::git::endpoint::stream_io::PRODUCER_CEILING_MS,
        600_000
    );
    push.cancel();
    rig.finish();
}

// The plumbing a push open takes (§2): the gate is the driver's, the endpoint's pump has the same one, a fetch
// has none, and pool identity is as it was.

#[test]
fn only_a_push_open_carries_a_producer_gate() {
    let rig = Rig::new("producer-busy-plumbing", false, Some(600_000));
    let session = rig
        .runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local
        .clone();
    let material = Material::new(&rig.fixture.temp.path().join("local"));
    let mut fetch = rig.open(GitService::UploadPack);
    read_advertisement(&mut fetch);
    assert_eq!(session.producer_gates_for_test(), 0, "a fetch has no gate");
    let push = rig.push_started(&material, 0);
    assert_eq!(session.producer_gates_for_test(), 1, "a push has one");
    push.cancel();
    fetch.cancel();
    rig.finish();
}

#[test]
fn a_push_open_reuses_the_connection_a_fetch_left_and_is_pooled_as_it_was() {
    let rig = Rig::new("producer-busy-identity", false, Some(600_000));
    let mut fetch = rig.open(GitService::UploadPack);
    read_advertisement(&mut fetch);
    fetch.write_all(b"0000").unwrap();
    fetch.close().unwrap();
    let endpoint = rig
        .runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    while endpoint.ssh_counts_for_test().unwrap().idle != 1 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let reused = Arc::new(Mutex::new(None));
    let seen = reused.clone();
    let push = rig.open_observing(
        GitService::ReceivePack,
        Arc::new(move |_, opened| {
            *seen.lock().unwrap() = Some(opened.reused);
        }),
    );
    assert_eq!(
        *reused.lock().unwrap(),
        Some(true),
        "a push open, gate and all, keys and partitions its connection as a fetch does"
    );
    push.cancel();
    rig.finish();
}
