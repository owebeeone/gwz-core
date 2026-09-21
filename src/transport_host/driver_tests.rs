//! End-to-end host-scoped driver coverage.
//!
//! These tests deliberately keep one request alive while several git drivers
//! run.  That is the boundary which catches a driver accidentally rebuilding
//! native transport state instead of using the request's host session.

use super::*;
use crate::git::GitBackend;
use crate::operation::NullSink;
use crate::workspace_ops::{
    handle_fetch, handle_init_from_sources, handle_materialize, handle_repo_sync,
};
use crate::{RequestMeta, TransportOptions, TransportPlacement};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

#[allow(unused_imports)]
#[path = "../../tests/transport_ssh/tests/common/mod.rs"]
mod common;

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

fn endpoint_home(fixture: &common::SshdFixture) -> PathBuf {
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

fn url(fixture: &common::SshdFixture) -> String {
    format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        fixture
            .repository
            .to_string_lossy()
            .replace('%', "%25")
            .replace(' ', "%20")
    )
}

fn commit(repository: &git2::Repository, text: &str) -> git2::Oid {
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

pub(super) struct CliHarness {
    pub(super) fixture: common::SshdFixture,
    pub(super) runtime: TransportRuntime,
    pub(super) endpoint: CliEndpoint,
    _link: session::LocalLink,
    home: PathBuf,
}

impl CliHarness {
    pub(super) fn new() -> Self {
        let fixture = common::SshdFixture::new();
        let home = endpoint_home(&fixture);
        let core_home = fixture.temp.path().join("core-home");
        std::fs::create_dir_all(core_home.join(".ssh")).unwrap();
        std::fs::write(core_home.join(".ssh/known_hosts"), b"").unwrap();
        let runtime = TransportRuntime::new(SshEndpointConfig::fixture(core_home, None)).unwrap();
        let driver_port = runtime.install_cli().unwrap();
        let (endpoint, endpoint_port) =
            CliEndpoint::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
        let link = session::LocalLink::new(driver_port, endpoint_port).unwrap();
        Self {
            fixture,
            runtime,
            endpoint,
            _link: link,
            home,
        }
    }

    pub(super) fn meta(&self, request_id: &str) -> RequestMeta {
        RequestMeta {
            request_id: request_id.into(),
            schema_version: "gwz.protocol/v0".into(),
            transport: Some(TransportOptions {
                placement: Some(TransportPlacement::Cli),
                default_identity: Some("client_ed25519".into()),
                endpoint_path_base: Some(self.home.to_string_lossy().into_owned()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

#[test]
fn cli_driver_operations_share_one_host_scoped_request() {
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    let first = commit(&server, "first");
    let request_meta = harness.meta("host-driver");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "clone".into()),
    )
    .unwrap();
    let target = harness.fixture.temp.path().join("clone");
    let backend = request
        .backend()
        .with_transport(harness.fixture.temp.path(), request_meta.transport.as_ref())
        .unwrap()
        .unwrap();

    backend.clone_repo(&url(&harness.fixture), &target).unwrap();
    assert!(
        backend
            .ls_remote(&target, "origin")
            .unwrap()
            .iter()
            .any(|item| item.target == first.to_string())
    );
    backend.fetch(&target, "origin").unwrap();
    server
        .tag_lightweight("fixture", &server.find_object(first, None).unwrap(), false)
        .unwrap();
    backend.tag_fetch(&target, "origin").unwrap();
    assert_eq!(
        backend
            .read_remote_file(&url(&harness.fixture), "origin", "payload")
            .unwrap(),
        Some(b"first".to_vec())
    );
    let local = git2::Repository::open(&target).unwrap();
    let pushed = commit(&local, "local");
    backend
        .push(&target, "origin", "refs/heads/main:refs/heads/published")
        .unwrap();
    assert_eq!(
        server
            .find_reference("refs/heads/published")
            .unwrap()
            .target(),
        Some(pushed)
    );
    let rows = backend.transport_observations().unwrap().snapshot();
    assert!(rows.len() >= 6);
    assert!(rows.iter().all(|row| row.authenticated == Some(true)));
    assert!(
        rows.iter()
            .all(|row| row.endpoint_id.is_some() && row.stream_id.is_some())
    );
    assert_eq!(rows.iter().filter(|row| row.credential_offered).count(), 1);
    let connection = rows[0]
        .connection_id
        .clone()
        .expect("endpoint connection ID");
    assert!(
        rows.iter()
            .all(|row| row.connection_id.as_ref() == Some(&connection))
    );
    assert!(rows.iter().skip(1).all(|row| row.reused == Some(true)));
    assert!(!harness.fixture.marker.exists());

    let _ = block_on(request.finish());
    let _ = block_on(client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}

#[test]
fn host_scope_rejects_metadata_or_operation_changes_before_transport() {
    let harness = CliHarness::new();
    let request_meta = harness.meta("scope-check");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "fetch".into()),
    )
    .unwrap();
    let backend = request.backend();
    let mut changed = request_meta.clone();
    changed.request_id = "other-request".into();
    assert!(backend.validate_transport_scope(&changed, "fetch").is_err());
    assert!(
        backend
            .validate_transport_scope(&request_meta, "other-operation")
            .is_err()
    );
    let _ = block_on(request.finish());
    let _ = block_on(client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}

#[test]
fn cli_preflight_checks_every_selected_identity_before_any_target_runs() {
    let harness = CliHarness::new();
    let mut request_meta = harness.meta("identity-preflight");
    request_meta.transport.as_mut().unwrap().remote_identities = vec![
        crate::RemoteSshIdentity {
            remote: "origin".into(),
            private_key_path: "client_ed25519".into(),
        },
        crate::RemoteSshIdentity {
            remote: "last-target".into(),
            private_key_path: "missing-last-target".into(),
        },
    ];
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "identity-check".into()),
    )
    .unwrap();
    let result = request
        .backend()
        .with_transport(Path::new("/tmp"), request_meta.transport.as_ref());
    assert!(result.is_err());
    assert!(!harness.fixture.temp.path().join("last-target").exists());
    let _ = block_on(request.finish());
    let _ = block_on(client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}

#[test]
fn explicit_cli_rejects_non_ssh_without_native_fallback() {
    let harness = CliHarness::new();
    let request_meta = harness.meta("scheme-check");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "clone".into()),
    )
    .unwrap();
    let options = request_meta.transport.as_ref().unwrap();
    let scoped = request
        .backend()
        .with_transport(Path::new("/tmp"), Some(options))
        .unwrap()
        .unwrap();
    let result = scoped.clone_repo(
        "file:///tmp/not-a-remote",
        &harness.fixture.temp.path().join("refused"),
    );
    assert!(
        matches!(result, Err(error) if error.code == crate::model::ErrorCode::UnsupportedOperation)
    );
    let _ = block_on(request.finish());
    let _ = block_on(client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}

#[test]
fn host_scoped_workspace_drivers_keep_request_identity_across_init_and_fetch() {
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    commit(&server, "first");
    let root = harness.fixture.temp.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let source = crate::SourceUrl {
        url: url(&harness.fixture),
        path: Some("member".into()),
        remote_name: None,
        branch: None,
    };

    let init_meta = harness.meta("workspace-init");
    let init_client = harness
        .endpoint
        .register_request(&init_meta.request_id)
        .unwrap();
    let init_request = block_on(harness.runtime.request(init_meta.clone(), "init".into())).unwrap();
    let init = handle_init_from_sources(
        init_request.backend(),
        &root,
        crate::InitFromSourcesRequest {
            meta: init_meta,
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![source],
            ..Default::default()
        },
        "init",
        &NullSink,
    )
    .unwrap();
    assert_eq!(
        init.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let _ = block_on(init_request.finish());
    let _ = block_on(init_client.finish());

    let fetch_meta = harness.meta("workspace-fetch");
    let fetch_client = harness
        .endpoint
        .register_request(&fetch_meta.request_id)
        .unwrap();
    let fetch_request =
        block_on(harness.runtime.request(fetch_meta.clone(), "fetch".into())).unwrap();
    let fetch = handle_fetch(
        fetch_request.backend(),
        &root,
        crate::FetchRequest { meta: fetch_meta },
        "fetch",
    )
    .unwrap();
    assert!(matches!(
        fetch.response.meta.aggregate_status,
        crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
    ));
    let _ = block_on(fetch_request.finish());
    let _ = block_on(fetch_client.finish());

    std::fs::remove_dir_all(root.join("member")).unwrap();
    let materialize_meta = harness.meta("workspace-materialize");
    let materialize_client = harness
        .endpoint
        .register_request(&materialize_meta.request_id)
        .unwrap();
    let materialize_request = block_on(
        harness
            .runtime
            .request(materialize_meta.clone(), "materialize".into()),
    )
    .unwrap();
    let materialize = handle_materialize(
        materialize_request.backend(),
        &root,
        crate::MaterializeRequest {
            meta: materialize_meta,
            target: crate::MaterializeTarget {
                kind: crate::MaterializeTargetKind::Lock,
                ..Default::default()
            },
        },
        "materialize",
        &NullSink,
    )
    .unwrap();
    assert!(matches!(
        materialize.response.meta.aggregate_status,
        crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
    ));
    let _ = block_on(materialize_request.finish());
    let _ = block_on(materialize_client.finish());

    let mut sync_meta = harness.meta("workspace-repo-sync");
    sync_meta.transport = None;
    let sync_client = harness
        .endpoint
        .register_request(&sync_meta.request_id)
        .unwrap();
    let sync_request = block_on(
        harness
            .runtime
            .request(sync_meta.clone(), "repo-sync".into()),
    )
    .unwrap();
    handle_repo_sync(
        sync_request.backend(),
        &root,
        crate::RepoSyncRequest {
            meta: sync_meta,
            private: Some(false),
        },
        "repo-sync",
    )
    .unwrap();
    let _ = block_on(sync_request.finish());
    let _ = block_on(sync_client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}

#[test]
fn cli_endpoint_preserves_repository_refusal_and_reuses_binding_after_failure() {
    let harness = CliHarness::new();
    use std::os::unix::fs::PermissionsExt;
    let script = harness.fixture.temp.path().join("refuse-missing.sh");
    std::fs::write(&script, "#!/bin/sh\ncase \"$SSH_ORIGINAL_COMMAND\" in\n *-missing*) echo 'ERROR: Repository not found.' >&2; exit 1 ;;\n *) eval \"$SSH_ORIGINAL_COMMAND\" ;;\nesac\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let public =
        std::fs::read_to_string(harness.fixture.temp.path().join("client_ed25519.pub")).unwrap();
    std::fs::write(
        harness.fixture.temp.path().join("authorized_keys"),
        format!("command=\"{}\" {}", script.display(), public),
    )
    .unwrap();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    commit(&server, "first");
    let request_meta = harness.meta("refusal");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "clone".into()),
    )
    .unwrap();
    let backend = request
        .backend()
        .with_transport(harness.fixture.temp.path(), request_meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let error = backend
        .clone_repo(
            &format!("{}-missing", url(&harness.fixture)),
            &harness.fixture.temp.path().join("absent"),
        )
        .unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::RemoteRejected);
    let denied = backend
        .transport_observations()
        .unwrap()
        .snapshot()
        .pop()
        .unwrap();
    assert_eq!(denied.authenticated, Some(true));
    assert!(denied.endpoint_id.is_some());
    backend
        .clone_repo(
            &url(&harness.fixture),
            &harness.fixture.temp.path().join("present"),
        )
        .unwrap();
    assert_eq!(block_on(request.finish()).pending_local_work, 0);
    block_on(client.finish());
    block_on(harness.endpoint.shutdown());
}

#[test]
fn cli_authentication_failure_retains_attempt_facts_without_claiming_an_open_stream() {
    let harness = CliHarness::new();
    std::fs::write(harness.fixture.temp.path().join("authorized_keys"), "").unwrap();
    let request_meta = harness.meta("authentication");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "clone".into()),
    )
    .unwrap();
    let backend = request
        .backend()
        .with_transport(harness.fixture.temp.path(), request_meta.transport.as_ref())
        .unwrap()
        .unwrap();
    let error = backend
        .clone_repo(
            &url(&harness.fixture),
            &harness.fixture.temp.path().join("denied"),
        )
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
    assert!(row.connection_id.is_none());
    block_on(request.finish());
    block_on(client.finish());
    block_on(harness.endpoint.shutdown());
}

#[test]
fn cli_one_request_supports_concurrent_git_streams() {
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    commit(&server, "concurrent");
    let request_meta = harness.meta("fanout");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(
        harness
            .runtime
            .request(request_meta.clone(), "clone".into()),
    )
    .unwrap();
    let backend = request
        .backend()
        .with_transport(harness.fixture.temp.path(), request_meta.transport.as_ref())
        .unwrap()
        .unwrap();
    thread::scope(|threads| {
        let jobs: Vec<_> = (0..crate::operation::resolve_jobs(None))
            .map(|index| {
                let backend = backend.clone();
                let url = url(&harness.fixture);
                let target = harness.fixture.temp.path().join(format!("fanout-{index}"));
                threads.spawn(move || backend.clone_repo(&url, &target))
            })
            .collect();
        for job in jobs {
            job.join().unwrap().unwrap();
        }
    });
    let rows = backend.transport_observations().unwrap().snapshot();
    assert_eq!(rows.len(), crate::operation::resolve_jobs(None));
    assert!(rows.iter().all(|row| row.authenticated == Some(true)));
    let streams: std::collections::BTreeSet<_> =
        rows.iter().map(|row| row.stream_id.unwrap()).collect();
    assert_eq!(streams.len(), crate::operation::resolve_jobs(None));
    block_on(request.finish());
    block_on(client.finish());
    block_on(harness.endpoint.shutdown());
}

#[test]
fn cli_open_rechecks_a_selected_file_after_successful_preflight() {
    let harness = CliHarness::new();
    let request_meta = harness.meta("changed-file");
    let client = harness
        .endpoint
        .register_request(&request_meta.request_id)
        .unwrap();
    let request = block_on(harness.runtime.request(request_meta, "clone".into())).unwrap();
    request.context.check_identity("client_ed25519").unwrap();
    std::fs::remove_file(harness.home.join("client_ed25519")).unwrap();
    let result = request.context.open(
        &url(&harness.fixture),
        crate::git::endpoint::ssh_channel::GitService::UploadPack,
        Some("client_ed25519".into()),
        std::sync::Arc::new(|_, _| panic!("missing key must never open a stream")),
        std::sync::Arc::new(|facts| assert!(!facts.credential_offered)),
    );
    let error = match result {
        Ok(_) => panic!("changed key incorrectly admitted"),
        Err(error) => error,
    };
    assert!(matches!(
        error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<gwz_transport::stream::Error>()),
        Some(gwz_transport::stream::Error::PeerFailed {
            code: gwz_transport::protocol::ErrorCode::Unavailable,
            ..
        })
    ));
    assert!(!harness.fixture.marker.exists());
    block_on(request.finish());
    block_on(client.finish());
    block_on(harness.endpoint.shutdown());
}
