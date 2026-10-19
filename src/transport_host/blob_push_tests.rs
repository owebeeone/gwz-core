//! Trigger test for the SSH pump stall review's Follow-up 1
//! (`GwzTransportSshPumpStall-ReviewState.md` section 3): does a push whose
//! producer pauses after the commands, with one very large object to write,
//! fail with a stall at the default limit?
//!
//! Each row pushes one random (incompressible) blob of `GWZ_BLOBPUSH_MIB` MiB,
//! read from a local pack, to a bare repository through the transport runtime
//! at the product's default stall limit (`GWZ_BLOBPUSH_STALL_MS` sets another).
//! The default of 3072 MiB is where the SSH row failed on a 12-core Apple-silicon
//! Mac before D42 (GwzPushProducerBusyDesign): libgit2 reads the whole object
//! into memory before it writes the object's first byte (it then deflates in
//! 1 MiB chunks), and that read is a silent pause the pump then charged as
//! `Network`. The pump now charges it to the producer, up to the ceiling. A tap in front of each server
//! stamps the client's writes, so the pause shows on the wire: the gap before
//! the first pack data. The HTTPS row
//! is the contrast: it reports `Backpressure` while its producer is paused.
//! The git server of either row is told to index the pack as it arrives
//! (`receive.unpackLimit=1`), so that it does not deflate the blob again after
//! the last byte, which would keep it silent for the stall limit by itself.
//!
//! Both rows are `#[ignore]`d: they write several times the blob's size to
//! disk and take minutes. `TMPDIR` names the directory the fixtures use, so
//! point it at one with room for them (4 x the blob), and run with
//! `--nocapture` to read the `BLOBPUSH` lines.
use super::driver_tests::{block_on, common, endpoint_home, local_meta};
use super::https_tests::{self, fixture, git_http_backend};
use super::*;
use crate::git::GitBackend;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

/// An uplink write at least this large carries pack data. The pump's keepalives
/// and window adjustments are smaller.
const DATA_BYTES: usize = 1024;
const MIB: usize = 1 << 20;

/// One read of the client's side: ms since the tap started, and bytes.
type Stamp = (u128, usize);

fn blob_mib() -> usize {
    std::env::var("GWZ_BLOBPUSH_MIB")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(3072)
}

/// The stall limit in ms: the product's default, unless `GWZ_BLOBPUSH_STALL_MS`
/// names another (the `--ssh-timeout` workaround, 0 for none).
fn stall_limit_ms() -> u64 {
    std::env::var("GWZ_BLOBPUSH_STALL_MS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(crate::git::DEFAULT_SERVER_TIMEOUT_MS as u64)
}

/// Writes `mib` MiB of xorshift output, which zlib cannot shrink, to `path`.
fn write_random_file(path: &Path, mib: usize) {
    let mut out = std::io::BufWriter::with_capacity(MIB, std::fs::File::create(path).unwrap());
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut chunk = vec![0u8; MIB];
    for _ in 0..mib {
        for word in chunk.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            word.copy_from_slice(&state.to_le_bytes());
        }
        out.write_all(&chunk).unwrap();
    }
    out.flush().unwrap();
}

/// Makes the bare repository `path` index the pack as it arrives instead of
/// unpacking one loose object, which would deflate the whole blob again after
/// the last byte and keep the server silent for longer than the stall limit.
/// That silence is the server's, not the client's producer's, so it would hide
/// what these rows measure. `GWZ_BLOBPUSH_SERVER_DEFAULTS` leaves the server as
/// git ships it.
fn quiet_server(path: &Path) {
    if std::env::var_os("GWZ_BLOBPUSH_SERVER_DEFAULTS").is_some() {
        return;
    }
    git2::Repository::open_bare(path)
        .unwrap()
        .config()
        .unwrap()
        .set_i32("receive.unpackLimit", 1)
        .unwrap();
}

/// Packs the repository's objects, so the push reads the blob from a pack and
/// not from one loose file. libgit2 reads a loose object's file with a single
/// `read`, which macOS refuses above 2 GiB ("failed to read descriptor: Invalid
/// argument"), so a blob of that size can only be pushed from a pack. Packed is
/// also how a repository that has been around holds a blob this large.
fn pack_locally(dir: &Path) {
    let started = Instant::now();
    common::run(
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["repack", "-a", "-d", "-q"]),
    );
    eprintln!("BLOBPUSH repack_ms={}", started.elapsed().as_millis());
}

/// A repository at `dir` whose one commit holds one blob of `mib` MiB of random
/// bytes, with `url` as `origin`. The blob is hashed from a file, so it is never
/// whole in memory here, and the file is gone before the push starts.
fn blob_repository(dir: &Path, url: &str, mib: usize) -> git2::Oid {
    let started = Instant::now();
    let repo = git2::Repository::init(dir).unwrap();
    repo.remote("origin", url).unwrap();
    let file = dir.join("blob.bin");
    write_random_file(&file, mib);
    let blob = repo.blob_path(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    let mut tree = repo.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(tree.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let commit = repo
        .commit(None, &signature, &signature, "blob", &tree, &[])
        .unwrap();
    repo.reference("refs/heads/blob", commit, true, "blob")
        .unwrap();
    pack_locally(dir);
    eprintln!(
        "BLOBPUSH setup mib={mib} blob_ms={}",
        started.elapsed().as_millis()
    );
    commit
}

/// A TCP forwarder in front of the fixture's sshd that stamps every read of the
/// client's side. What the client's pump writes shows up here as it is sent.
struct Tap {
    port: u16,
    origin: Instant,
    uplink: std::sync::Arc<Mutex<Vec<Stamp>>>,
    stop: std::sync::Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Tap {
    fn start(upstream: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = Instant::now();
        let uplink = std::sync::Arc::new(Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (events, halted) = (uplink.clone(), stop.clone());
        let accept = std::thread::spawn(move || {
            while !halted.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        client.set_nonblocking(false).unwrap();
                        let server = TcpStream::connect(("127.0.0.1", upstream)).unwrap();
                        let _ = client.set_nodelay(true);
                        let _ = server.set_nodelay(true);
                        let (down_from, down_to) =
                            (server.try_clone().unwrap(), client.try_clone().unwrap());
                        std::thread::spawn(move || copy(down_from, down_to, None, origin));
                        let events = events.clone();
                        std::thread::spawn(move || copy(client, server, Some(events), origin));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            port,
            origin,
            uplink,
            stop,
            accept: Some(accept),
        }
    }

    /// Stops accepting and returns the client's writes as (ms since the tap
    /// started, bytes), in order.
    fn finish(mut self) -> Vec<Stamp> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            accept.join().unwrap();
        }
        let _ = self.origin;
        self.uplink.lock().unwrap().clone()
    }
}

/// Copies `from` to `to` until either ends, recording each read of an uplink.
fn copy(
    mut from: TcpStream,
    mut to: TcpStream,
    events: Option<std::sync::Arc<Mutex<Vec<Stamp>>>>,
    origin: Instant,
) {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if let Some(events) = &events {
                    events
                        .lock()
                        .unwrap()
                        .push((origin.elapsed().as_millis(), count));
                }
                if to.write_all(&buffer[..count]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(Shutdown::Write);
    let _ = from.shutdown(Shutdown::Read);
}

/// The pack-data writes of an uplink, the largest pauses between two of them,
/// and the bytes sent, as `BLOBPUSH` lines. Returns the largest pause in ms.
fn report_uplink(events: &[Stamp], push_started_ms: u128, push_ended_ms: u128) -> u128 {
    let data: Vec<Stamp> = events
        .iter()
        .copied()
        .filter(|(_, count)| *count >= DATA_BYTES)
        .collect();
    let total: usize = events.iter().map(|(_, count)| count).sum();
    let mut sent = 0usize;
    let mut gaps = Vec::new();
    for pair in data.windows(2) {
        sent += pair[0].1;
        gaps.push((pair[1].0 - pair[0].0, pair[0].0, sent));
    }
    gaps.sort_by(|a, b| b.cmp(a));
    eprintln!(
        "BLOBPUSH uplink bytes={total} data_writes={} first_data_ms={:?} last_data_ms={:?} push_started_ms={push_started_ms} push_ended_ms={push_ended_ms}",
        data.len(),
        data.first().map(|e| e.0),
        data.last().map(|e| e.0),
    );
    // Every write inside the largest pause's window, small ones too: the
    // command line that starts the receive-pack exchange is among them, and the
    // pump's stall clock runs from there.
    if data.len() < 8 {
        // No pack data went out at all (the push failed first): every write.
        for (when, count) in events.iter().take(60) {
            eprintln!("BLOBPUSH uplink_event at_ms={when} bytes={count}");
        }
    }
    if let Some((gap, at, _)) = gaps.first() {
        for (when, count) in events
            .iter()
            .filter(|e| e.0 + 100 >= *at && e.0 <= at + gap + 5)
            .take(60)
        {
            eprintln!("BLOBPUSH uplink_event at_ms={when} bytes={count}");
        }
    }
    eprintln!(
        "BLOBPUSH pause_before_first_data_ms={:?}",
        data.first()
            .map(|first| first.0.saturating_sub(push_started_ms))
    );
    for (gap, at, after) in gaps.iter().take(5) {
        eprintln!("BLOBPUSH gap_ms={gap} began_at_ms={at} after_uplink_bytes={after}");
    }
    gaps.first().map_or(0, |gap| gap.0)
}

#[test]
#[ignore = "Follow-up 1 trigger, green since D42's producer-busy signal: one random blob over SSH at the default stall limit, to a server that indexes the pack as it arrives (receive.unpackLimit=1); minutes and 4x the blob on disk, set TMPDIR and run with --nocapture"]
fn a_large_random_blob_push_over_ssh_survives_the_default_stall_limit() {
    let mib = blob_mib();
    let fixture = common::SshdFixture::new();
    quiet_server(&fixture.repository);
    let tap = Tap::start(fixture.port);
    let home = endpoint_home(&fixture);
    std::fs::write(
        home.join(".ssh/known_hosts"),
        format!("{}\n", fixture.known_host("127.0.0.1", tap.port, false)),
    )
    .unwrap();
    let limit = stall_limit_ms();
    let runtime = TransportRuntime::new(
        SshEndpointConfig::fixture(home.clone(), None).with_io_timeout_ms(limit),
    )
    .unwrap();
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        tap.port,
        common::server_path(&fixture.url_repository)
    );
    let local = fixture.temp.path().join("local");
    let commit = blob_repository(&local, &url, mib);
    let meta = local_meta("blob-push", &home);
    let request = block_on(runtime.request(meta.clone(), "push".into())).unwrap();
    let backend = request
        .backend()
        .with_transport(fixture.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let started = Instant::now();
    let started_ms = tap.origin.elapsed().as_millis();
    let result = backend.push(&local, "origin", "refs/heads/blob:refs/heads/blob");
    let took = started.elapsed();
    let ended_ms = tap.origin.elapsed().as_millis();
    let _ = block_on(request.finish());
    block_on(runtime.shutdown());
    let events = tap.finish();
    let longest = report_uplink(&events, started_ms, ended_ms);
    eprintln!(
        "BLOBPUSH ssh mib={mib} limit_ms={limit} push_ms={} longest_uplink_pause_ms={longest} result={}",
        took.as_millis(),
        match &result {
            Ok(_) => "ok".to_owned(),
            Err(error) => format!("{error:?}"),
        }
    );
    result.expect("the push succeeds");
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    assert_eq!(
        server.find_reference("refs/heads/blob").unwrap().target(),
        Some(commit)
    );
}

#[test]
#[ignore = "Follow-up 1 contrast: the same blob over HTTPS survives the producer pause but the fixture server (it indexes after the whole body) is silent for the limit afterwards; minutes and 4x the blob on disk, set TMPDIR and run with --nocapture"]
fn a_large_random_blob_push_over_https_survives_the_default_stall_limit() {
    let mib = blob_mib();
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (bare, _) = https_tests::repository_with_payload(root.path(), b"seed\n");
        quiet_server(&bare);
        let repository = Arc::new(bare.clone());
        let origin = Instant::now();
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            let what = format!("{} {}", request.method(), request.uri().path());
            Box::pin(async move {
                eprintln!(
                    "BLOBPUSH https server begins {what} at_ms={}",
                    origin.elapsed().as_millis()
                );
                let response = git_http_backend(repository, request).await;
                eprintln!(
                    "BLOBPUSH https server answers {what} at_ms={}",
                    origin.elapsed().as_millis()
                );
                response
            })
        }))
        .await;
        let limit = stall_limit_ms();
        let transport = TransportRuntime::with_https(
            SshEndpointConfig::fixture(https_tests::endpoint_home(root.path()), None)
                .with_io_timeout_ms(limit),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )
        .unwrap();
        // The same tap as the SSH row's, in front of the TLS server: it sees
        // when the client's bytes go out, not what they are.
        let port: u16 = server
            .url
            .rsplit_once(':')
            .and_then(|(_, rest)| rest.split('/').next())
            .and_then(|port| port.parse().ok())
            .expect("the fixture URL names a port");
        let tap = Tap::start(port);
        let url = format!("https://localhost:{}/repo", tap.port);
        let local = root.path().join("local");
        let local_for_setup = local.clone();
        let commit = tokio::task::spawn_blocking(move || {
            blob_repository(&local_for_setup, &url, mib)
        })
        .await
        .unwrap();
        let request = transport
            .request(https_tests::meta("blob-push-https"), "push".into())
            .await
            .unwrap();
        let backend = request.backend().clone();
        let started = Instant::now();
        let started_ms = tap.origin.elapsed().as_millis();
        eprintln!("BLOBPUSH https push begins at_ms={}", origin.elapsed().as_millis());
        let result = tokio::task::spawn_blocking(move || {
            backend.push(&local, "origin", "refs/heads/blob:refs/heads/blob")
        })
        .await
        .unwrap();
        let ended_ms = tap.origin.elapsed().as_millis();
        let _ = request.finish().await;
        let _ = transport.shutdown().await;
        let longest = report_uplink(&tap.finish(), started_ms, ended_ms);
        eprintln!(
            "BLOBPUSH https longest_uplink_pause_ms={longest} mib={mib} limit_ms={limit} push_ms={} result={}",
            started.elapsed().as_millis(),
            match &result {
                Ok(_) => "ok".to_owned(),
                Err(error) => format!("{error:?}"),
            }
        );
        result.expect("the push succeeds");
        let remote = git2::Repository::open_bare(&bare).unwrap();
        assert_eq!(
            remote.find_reference("refs/heads/blob").unwrap().target(),
            Some(commit)
        );
    });
}
