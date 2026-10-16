//! End-to-end host-scoped driver coverage.
//!
//! These tests deliberately keep one request alive while several git drivers
//! run.  That is the boundary which catches a driver accidentally rebuilding
//! native transport state instead of using the request's host session.

use super::*;
use crate::git::GitBackend;
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
    let deadline = Instant::now() + Duration::from_secs(5);
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
