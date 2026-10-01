//! TR2.13: the transport moves a large pack about as fast as libgit2's own
//! transport does, on the same fixture.
//!
//! Each test clones one repository whose pack holds `PACK_MIB` MiB of
//! incompressible data, alternately through libgit2's own SSH or HTTPS
//! transport, the baseline, and through the transport runtime, the way a
//! command's local transport runs it. The baseline runs in a child process
//! whose HOME is the fixture's, so libgit2 reads the fixture's known_hosts and
//! no user configuration.
//!
//! A transport clone keeps pace when it takes at most `FACTOR` times the
//! fastest baseline clone so far, plus `SLACK` for setup that does not scale
//! with the pack. On a loaded host either clone can lose seconds to its
//! neighbours, so the test tries up to `ATTEMPTS` times; a cap on the
//! transport's rate slows every attempt alike.
use super::driver_tests::{block_on, common, endpoint_home, local_meta};
use super::https_tests::{fixture, git_http_backend, meta as https_meta};
use super::*;
use crate::git::GitBackend;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PACK_MIB: usize = 16;
const FACTOR: f64 = 1.5;
const SLACK: Duration = Duration::from_secs(1);
const ATTEMPTS: usize = 3;
const NATIVE_DEADLINE: Duration = Duration::from_secs(60);
const PROBE: &str = "transport_host::throughput_tests::native_clone_probe";
const URL: &str = "GWZ_TEST_NATIVE_CLONE_URL";
const TARGET: &str = "GWZ_TEST_NATIVE_CLONE_TARGET";
const KEY: &str = "GWZ_TEST_NATIVE_CLONE_KEY";
const TRUSTED_HOST: &str = "GWZ_TEST_NATIVE_CLONE_TRUSTED_HOST";
const ELAPSED: &str = "NATIVE-CLONE-MS=";

/// Bytes that neither zlib nor delta compression can shrink, so the pack is
/// as large as the payload. A fixed seed keeps every run's pack the same.
fn incompressible(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut bytes = Vec::with_capacity(length + 8);
    while bytes.len() < length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(length);
    bytes
}

/// Creates a bare repository whose one commit holds one large file, then packs
/// it, so the server sends stored pack data instead of compressing it for each
/// clone. Its plain path needs no escaping in either transport's URL.
fn large_repository(repository: &Path) {
    let repo = git2::Repository::init_bare(repository).unwrap();
    let blob = repo.blob(&incompressible(PACK_MIB << 20)).unwrap();
    let mut tree = repo.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(tree.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    repo.commit(
        Some("refs/heads/main"),
        &signature,
        &signature,
        "large",
        &tree,
        &[],
    )
    .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    common::run(
        Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["repack", "-a", "-d", "-q"]),
    );
}

/// Clones through libgit2's own transport in a child process with a clean
/// environment, and returns the clone's own time as the child measured it.
fn native_clone(
    url: &str,
    target: &Path,
    home: &Path,
    key: Option<&Path>,
    trusted: &str,
) -> Duration {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env("HOME", home)
        .env(URL, url)
        .env(TARGET, target)
        .env(TRUSTED_HOST, trusted)
        .args([
            "--ignored",
            "--exact",
            PROBE,
            "--nocapture",
            "--test-threads=1",
        ]);
    if let Some(key) = key {
        command.env(KEY, key);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + NATIVE_DEADLINE;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("libgit2's own clone did not finish within {NATIVE_DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "native clone failed: {stdout} {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The harness prints the probe's output after its test's name, mid-line.
    let elapsed = stdout
        .split(ELAPSED)
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("the native clone reported no time: {stdout}"));
    Duration::from_millis(elapsed.parse().unwrap())
}

#[test]
#[ignore = "the child process of the throughput tests' native baseline"]
fn native_clone_probe() {
    let (Some(url), Some(target)) = (std::env::var_os(URL), std::env::var_os(TARGET)) else {
        return;
    };
    let url = url.into_string().unwrap();
    let key = std::env::var_os(KEY);
    let trusted = std::env::var(TRUSTED_HOST).unwrap_or_default();
    let mut offered = false;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |_, user, _| {
        if offered {
            return Err(git2::Error::from_str("the fixture key was refused"));
        }
        offered = true;
        let key = key
            .as_ref()
            .ok_or_else(|| git2::Error::from_str("no key"))?;
        git2::Cred::ssh_key(user.unwrap_or_default(), None, Path::new(key), None)
    });
    // SSH trusts the fixture's known_hosts through HOME. HTTPS trusts only
    // the fixture server's name, whose certificate a temporary CA signed.
    callbacks.certificate_check(move |_, host| {
        if host == trusted {
            Ok(git2::CertificateCheckStatus::CertificateOk)
        } else {
            Ok(git2::CertificateCheckStatus::CertificatePassthrough)
        }
    });
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(callbacks);
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.disable_filters(true);
    let began = Instant::now();
    git2::build::RepoBuilder::new()
        .fetch_options(fetch)
        .with_checkout(checkout)
        .clone(&url, Path::new(&target))
        .unwrap();
    println!("{ELAPSED}{}", began.elapsed().as_millis());
}

/// Alternates a baseline clone and a transport clone, each into a new
/// directory named for the attempt, until a transport clone keeps pace.
fn assert_keeps_pace(
    scheme: &str,
    mut native: impl FnMut(usize) -> Duration,
    mut transport: impl FnMut(usize) -> Duration,
) {
    let rate = |elapsed: Duration| PACK_MIB as f64 / elapsed.as_secs_f64();
    let mut fastest = Duration::MAX;
    let mut runs = Vec::new();
    for attempt in 0..ATTEMPTS {
        fastest = fastest.min(native(attempt));
        let took = transport(attempt);
        let bound = fastest.mul_f64(FACTOR) + SLACK;
        eprintln!(
            "{scheme} clone of {PACK_MIB} MiB, attempt {attempt}: libgit2's fastest {fastest:?} \
             ({:.1} MiB/s), transport {took:?} ({:.1} MiB/s), bound {bound:?}",
            rate(fastest),
            rate(took)
        );
        runs.push(format!("{took:?} ({:.1} MiB/s)", rate(took)));
        if took <= bound {
            return;
        }
    }
    panic!(
        "every {scheme} transport clone of the {PACK_MIB} MiB pack fell behind libgit2's own \
         transport, whose fastest clone took {fastest:?} ({:.1} MiB/s): {runs:?}",
        rate(fastest)
    );
}

#[test]
fn an_ssh_clone_of_a_large_pack_keeps_pace_with_libgit2() {
    let fixture = common::SshdFixture::new();
    let repository = fixture.temp.path().join("large.git");
    large_repository(&repository);
    let home = endpoint_home(&fixture);
    let key = home.join("client_ed25519");
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        repository.display()
    );
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta("throughput-ssh", &home);
    let request = block_on(runtime.request(meta.clone(), "clone".into())).unwrap();
    let backend = request
        .backend()
        .with_transport(fixture.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let directory =
        |name: &str, attempt: usize| fixture.temp.path().join(format!("{name}-{attempt}"));
    assert_keeps_pace(
        "SSH",
        |attempt| native_clone(&url, &directory("native", attempt), &home, Some(&key), ""),
        |attempt| {
            let began = Instant::now();
            backend
                .clone_repo(&url, &directory("transport", attempt))
                .unwrap();
            began.elapsed()
        },
    );
    assert_eq!(block_on(request.finish()).pending_local_work, 0);
    block_on(runtime.shutdown());
}

#[test]
fn an_https_clone_of_a_large_pack_keeps_pace_with_libgit2() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repo");
        large_repository(&repository);
        let repository = Arc::new(repository);
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move {
                // The fixture's macOS TLS keeps a record it could not send
                // while the socket was full until its next read or write; a
                // closing connection sends it. Without this a large response
                // can stall a client, libgit2's above all.
                let mut response = git_http_backend(repository, request).await;
                response.headers_mut().insert(
                    hyper::header::CONNECTION,
                    hyper::header::HeaderValue::from_static("close"),
                );
                response
            })
        }))
        .await;
        let home = super::https_tests::endpoint_home(root.path());
        let runtime = TransportRuntime::with_https(
            SshEndpointConfig::fixture(home.clone(), None),
            HttpsEndpointConfig {
                tls: server.config(),
                auth: None,
            },
            HelperSlots::new(),
        )
        .unwrap();
        let request = runtime
            .request(https_meta("throughput-https"), "clone".into())
            .await
            .unwrap();
        let backend = request.backend().clone();
        let (url, directory) = (server.url.clone(), root.path().to_path_buf());
        // The clones block; this executor keeps serving them meanwhile.
        let compared = tokio::task::spawn_blocking(move || {
            let directory =
                |name: &str, attempt: usize| directory.join(format!("{name}-{attempt}"));
            assert_keeps_pace(
                "HTTPS",
                |attempt| {
                    native_clone(
                        &url,
                        &directory("native", attempt),
                        &home,
                        None,
                        "localhost",
                    )
                },
                |attempt| {
                    let began = Instant::now();
                    backend
                        .clone_repo(&url, &directory("transport", attempt))
                        .unwrap();
                    began.elapsed()
                },
            );
        })
        .await;
        if let Err(error) = compared {
            std::panic::resume_unwind(error.into_panic());
        }
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
