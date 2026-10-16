//! End-to-end host-scoped driver coverage.
//!
//! These tests deliberately keep one request alive while several git drivers
//! run.  That is the boundary which catches a driver accidentally rebuilding
//! native transport state instead of using the request's host session.

use super::*;
use crate::git::GitBackend;
use crate::operation::NullSink;
use crate::workspace_ops::{handle_fetch, handle_init_from_sources};
use crate::{RequestMeta, TransportOptions};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

pub(super) use crate::git::endpoint::ssh_fixture as common;

pub(super) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                assert!(
                    Instant::now() < deadline,
                    "host driver future exceeded test deadline"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

pub(super) fn endpoint_home(fixture: &common::SshdFixture) -> PathBuf {
    let home = fixture.temp.path().join("endpoint-home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::copy(&fixture.known_hosts, home.join(".ssh/known_hosts")).unwrap();
    std::fs::copy(
        fixture.temp.path().join("client_ed25519"),
        home.join("client_ed25519"),
    )
    .unwrap();
    home
}

/// The fixture repository's URL. Its path passes as written, as in libgit2.
pub(super) fn fixture_url(fixture: &common::SshdFixture) -> String {
    format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        common::server_path(&fixture.url_repository)
    )
}

pub(super) fn local_meta(id: &str, home: &Path) -> RequestMeta {
    RequestMeta {
        request_id: id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            default_identity: Some(home.join("client_ed25519").to_string_lossy().into_owned()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn sequential_local_requests_reuse_an_idle_ssh_connection() {
    let fixture = common::SshdFixture::new();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "first");
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let url = fixture_url(&fixture);
    let mut connection = None;
    for index in 0..2 {
        let meta = local_meta(&format!("local-reuse-{index}"), &home);
        let request = block_on(runtime.request(meta.clone(), "clone".into())).unwrap();
        let backend = request
            .backend()
            .with_transport(fixture.temp.path(), meta.transport.as_ref())
            .unwrap()
            .unwrap();
        backend
            .clone_repo(
                &url,
                &fixture.temp.path().join(format!("local-reuse-{index}")),
            )
            .unwrap();
        let row = backend
            .transport_observations()
            .unwrap()
            .snapshot()
            .pop()
            .unwrap();
        if let Some(first) = &connection {
            assert_eq!(row.connection_id.as_ref(), Some(first));
            assert_eq!(row.reused, Some(true));
        } else {
            connection = row.connection_id;
        }
        assert_eq!(block_on(request.finish()).pending_local_work, 0);
        // The member is done at libssh2's close, and its connection is leased, and counted, until the server's close
        // ends and the worker returns it: a second request that came sooner would open a connection of its own.
        // How soon that is depends on how fast the server's session ends (a Windows `sshd.exe` takes longer).
        wait_for_an_idle_connection(&runtime);
    }
    block_on(runtime.shutdown());
}

/// Waits until the runtime's local SSH pool holds one idle connection and no leased one.
fn wait_for_an_idle_connection(runtime: &TransportRuntime) {
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let counts = endpoint.ssh_counts_for_test().unwrap();
        if (counts.idle, counts.leased) == (1, 0) {
            return;
        }
        assert!(Instant::now() < deadline, "no idle connection: {counts:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_live_local_lease_admits_an_overlapping_request_with_the_same_capacity() {
    let fixture = common::SshdFixture::new();
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let first =
        block_on(runtime.request(local_meta("lease-first", &home), "fetch".into())).unwrap();
    let stream = first
        .context
        .open(
            &fixture_url(&fixture),
            crate::git::endpoint::ssh_channel::GitService::UploadPack,
            Some(home.join("client_ed25519").to_string_lossy().into_owned()),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
        )
        .unwrap();
    let second = block_on(runtime.request(local_meta("lease-second", &home), "fetch".into()))
        .expect("equal capacity joins without touching the held physical lease");
    assert_eq!(block_on(second.finish()).pending_local_work, 0);
    drop(stream);
    block_on(first.finish());
    block_on(runtime.shutdown());
}

pub(super) fn commit(repository: &git2::Repository, text: &str) -> git2::Oid {
    let blob = repository.blob(text.as_bytes()).unwrap();
    let mut builder = repository.treebuilder(None).unwrap();
    builder.insert("payload", blob, 0o100644).unwrap();
    let tree = repository.find_tree(builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok());
    repository.set_head("refs/heads/main").unwrap();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            text,
            &tree,
            &parent.iter().collect::<Vec<_>>(),
        )
        .unwrap()
}

/// The host-scope guard on the Local route: a backend of a request refuses metadata or an operation that is not the
/// request's, before any transport is touched. The Cli twin is `cli_driver_tests`.
#[test]
fn host_scope_rejects_metadata_or_operation_changes_before_transport() {
    let runtime = TransportRuntime::new(super::tests::config()).unwrap();
    let request_meta = RequestMeta {
        request_id: "scope-check".into(),
        schema_version: "gwz.protocol/v0".into(),
        ..Default::default()
    };
    let request = block_on(runtime.request(request_meta.clone(), "fetch".into())).unwrap();
    let backend = request.backend();
    backend
        .validate_transport_scope(&request_meta, "fetch")
        .unwrap();
    let mut changed = request_meta.clone();
    changed.request_id = "other-request".into();
    assert!(backend.validate_transport_scope(&changed, "fetch").is_err());
    assert!(
        backend
            .validate_transport_scope(&request_meta, "other-operation")
            .is_err()
    );
    let _ = block_on(request.finish());
    block_on(runtime.shutdown());
}

/// A setup the server refuses leaves the attempt's credential facts and no receipt: the four receipts say "an open
/// was reported for this row", and none was (S7.1 C2's absence rule). The Cli twin is `cli_driver_tests`.
#[test]
fn local_authentication_failure_retains_attempt_facts_without_claiming_an_open_stream() {
    let fixture = common::SshdFixture::new();
    std::fs::write(fixture.temp.path().join("authorized_keys"), "").unwrap();
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta("authentication", &home);
    let request = block_on(runtime.request(meta.clone(), "clone".into())).unwrap();
    let backend = request
        .backend()
        .with_transport(fixture.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let error = backend
        .clone_repo(&fixture_url(&fixture), &fixture.temp.path().join("denied"))
        .unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::RemoteRejected);
    let row = backend
        .transport_observations()
        .unwrap()
        .snapshot()
        .pop()
        .unwrap();
    assert!(row.credential_offered);
    assert_eq!(row.authenticated, Some(false));
    assert_eq!(
        (
            row.endpoint_id,
            row.connection_id,
            row.stream_id,
            row.reused
        ),
        (None, None, None, None)
    );
    block_on(request.finish());
    block_on(runtime.shutdown());
}

/// A raw upload-pack stream over the Local route, read past the server's advertisement: the runtime, the request, the
/// stream, and the commit the server advertised.
fn local_upload_pack_past_advertisement(
    fixture: &common::SshdFixture,
    id: &str,
) -> (
    TransportRuntime,
    TransportRequest,
    crate::git::endpoint::stream_io::BlockingStream,
    String,
) {
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    let head = commit(&server, "advertised").to_string();
    let home = endpoint_home(fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta(id, &home);
    let request = block_on(runtime.request(meta, "fetch".into())).unwrap();
    let mut stream = request
        .context
        .open(
            &fixture_url(fixture),
            crate::git::endpoint::ssh_channel::GitService::UploadPack,
            Some(home.join("client_ed25519").to_string_lossy().into_owned()),
            std::sync::Arc::new(|_, _| {}),
            std::sync::Arc::new(|_| {}),
        )
        .unwrap();
    let mut advertisement = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !advertisement.ends_with(b"0000") {
        let read = std::io::Read::read(&mut stream, &mut buffer).unwrap();
        assert_ne!(read, 0, "the advertisement ended early");
        advertisement.extend_from_slice(&buffer[..read]);
    }
    assert!(
        advertisement
            .windows(head.len())
            .any(|window| window == head.as_bytes())
    );
    (runtime, request, stream, head)
}

/// A clone's whole request: one want, its flush, and done.
fn clone_request(head: &str) -> Vec<u8> {
    let want = format!("want {head}\n");
    format!("{:04x}{want}00000009done\n", want.len() + 4).into_bytes()
}

#[test]
fn local_client_think_time_after_the_advertisement_never_times_out() {
    // GwzRemoteTransportDesign §10.1: application think time does not start
    // the endpoint's stall clock. Think past the fixture's 3 s, then fetch.
    let fixture = common::SshdFixture::new();
    let (runtime, request, mut stream, head) =
        local_upload_pack_past_advertisement(&fixture, "thinking");
    let (done, outcome) = std::sync::mpsc::channel();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(4));
        let fetched = std::io::Write::write_all(&mut stream, &clone_request(&head))
            .and_then(|()| std::io::Write::flush(&mut stream))
            .and_then(|()| {
                let mut response = Vec::new();
                std::io::Read::read_to_end(&mut stream, &mut response).map(|_| response)
            });
        let _ = done.send(fetched.map_err(|error| error.kind()));
    });
    let response = outcome
        .recv_timeout(Duration::from_secs(20))
        .expect("the fetch never finished")
        .expect("thinking ended the exchange");
    assert!(response.starts_with(b"0008NAK\nPACK"));
    block_on(request.finish());
    block_on(runtime.shutdown());
}

#[test]
fn local_server_stall_after_done_reaches_the_client_as_a_timeout() {
    // After done the server owes the pack, so its stall runs the endpoint's
    // 3 s clock. The client's reads have no deadline of their own: the
    // endpoint's Timeout must reach them, or they wait forever.
    let fixture = common::SshdFixture::new();
    let (runtime, request, mut stream, head) =
        local_upload_pack_past_advertisement(&fixture, "stalled");
    let mut paused = common::pause_process_tree(fixture.child.id());
    let (done, outcome) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let read = std::io::Write::write_all(&mut stream, &clone_request(&head))
            .and_then(|()| std::io::Write::flush(&mut stream))
            .and_then(|()| std::io::Read::read(&mut stream, &mut [0_u8; 1]));
        let _ = done.send(read.map_err(|error| (error.kind(), error.to_string())));
    });
    let outcome = outcome
        .recv_timeout(Duration::from_secs(20))
        .expect("the endpoint's timeout never reached the client");
    paused.resume();
    // The body-read stall names the flag and the fixture's 3 s bound (T7).
    assert_eq!(
        outcome,
        Err((
            std::io::ErrorKind::TimedOut,
            "no progress for 3 seconds (--ssh-timeout 3)".to_string()
        ))
    );
    block_on(request.finish());
    block_on(runtime.shutdown());
}

/// A fetch whose later member names an identity file that is missing is refused whole, before the first member's
/// tracking ref moves (push and pull do the same). The Cli twin is `fetch_preflight_tests`.
#[test]
fn local_fetch_checks_later_repo_identity_before_any_tracking_ref_moves() {
    let fixture = common::SshdFixture::new();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    let first = commit(&server, "first");
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let root = fixture.temp.path().join("fetch-preflight");
    std::fs::create_dir(&root).unwrap();
    let init_meta = local_meta("fetch-preflight-init", &home);
    let init_request = block_on(runtime.request(init_meta.clone(), "init".into())).unwrap();
    handle_init_from_sources(
        init_request.backend(),
        &root,
        crate::InitFromSourcesRequest {
            meta: init_meta,
            workspace_root: root.to_string_lossy().into_owned(),
            sources: ["first", "last"]
                .map(|path| crate::SourceUrl {
                    url: fixture_url(&fixture),
                    path: Some(path.into()),
                    ..Default::default()
                })
                .to_vec(),
            ..Default::default()
        },
        "init",
        &NullSink,
    )
    .unwrap();
    let _ = block_on(init_request.finish());

    for (member, key) in [("first", "client_ed25519"), ("last", "missing-last-key")] {
        git2::Repository::open(root.join(member))
            .unwrap()
            .config()
            .unwrap()
            .set_str(
                "remote.origin.gwzSshIdentity",
                &home.join(key).to_string_lossy(),
            )
            .unwrap();
    }
    let tracking = |member: &str| {
        git2::Repository::open(root.join(member))
            .unwrap()
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target()
    };
    assert_eq!(
        (tracking("first"), tracking("last")),
        (Some(first), Some(first))
    );
    let second = commit(&server, "second");
    assert_ne!(second, first);

    let fetch_meta = RequestMeta {
        request_id: "fetch-preflight".into(),
        schema_version: "gwz.protocol/v0".into(),
        ..Default::default()
    };
    let fetch_request = block_on(runtime.request(fetch_meta.clone(), "fetch".into())).unwrap();
    let error = handle_fetch(
        fetch_request.backend(),
        &root,
        crate::FetchRequest { meta: fetch_meta },
        "fetch",
    )
    .expect_err("missing later identity must reject before fanout");
    assert_eq!(error.code, crate::model::ErrorCode::PermissionDenied);
    assert_eq!(
        (tracking("first"), tracking("last")),
        (Some(first), Some(first))
    );
    assert!(
        error
            .response_meta
            .as_ref()
            .and_then(|meta| meta.transport.as_ref())
            .is_none_or(Vec::is_empty)
    );
    let _ = block_on(fetch_request.finish());
    block_on(runtime.shutdown());
}
