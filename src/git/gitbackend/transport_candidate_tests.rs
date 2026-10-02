//! Full backend tests, compiled only by the isolated candidate harness. Only a
//! host context reaches the transport (TR2.11), so every backend here that
//! takes it carries one, from a request of the fixture's `Host`.
use super::*;
use crate::git::endpoint::ssh_fixture as common;
use crate::transport_host::{SshEndpointConfig, TransportRequest, TransportRuntime};
mod drivers;
mod host_context;
mod known_hosts_case;
mod url_parity;
mod url_password;

/// One transport runtime for a test, as `with_local_transport` builds one per
/// command. Its endpoint trusts the fixture's host key.
struct Host {
    executor: tokio::runtime::Runtime,
    runtime: TransportRuntime,
    /// The endpoint's `known_hosts`, which a test may empty.
    known_hosts: PathBuf,
}
impl Host {
    fn new(f: &common::SshdFixture) -> Self {
        let home = f.temp.path().join("endpoint-home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let known_hosts = home.join(".ssh/known_hosts");
        std::fs::copy(&f.known_hosts, &known_hosts).unwrap();
        // Production's per-step SSH budget. At 3 s, a clone under the full
        // suite's load stalled past it and failed as a Timeout.
        let config = SshEndpointConfig::fixture(home, None)
            .with_io_timeout_ms(super::transport_support::DEFAULT_SERVER_TIMEOUT_MS as u64);
        Self {
            executor: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            runtime: TransportRuntime::new(config).unwrap(),
            known_hosts,
        }
    }
    /// One operation's request. Its backend carries the host context, which
    /// admits exactly this metadata and operation.
    fn request(&self, meta: crate::RequestMeta, operation: &str) -> TransportRequest {
        self.executor
            .block_on(self.runtime.request(meta, operation.into()))
            .unwrap()
    }
    fn finish(&self, request: TransportRequest) {
        self.executor.block_on(request.finish());
    }
    fn shutdown(&self) {
        self.executor.block_on(self.runtime.shutdown());
    }
}
/// A request's metadata: the fixture's key, and one connection per host, the
/// capacity these tests' endpoint has always had.
fn request_meta(f: &common::SshdFixture, request_id: &str) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: request_id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(crate::TransportOptions {
            default_identity: Some(
                f.temp
                    .path()
                    .join("client_ed25519")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        }),
        policy: Some(crate::OperationPolicy {
            max_connections_per_host: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    }
}
/// The fixture, its host, an open request, and that request's backend scoped
/// to the fixture's key. The request must outlive every use of the backend.
fn fixture() -> (common::SshdFixture, Host, TransportRequest, Git2Backend) {
    let f = common::SshdFixture::new();
    let host = Host::new(&f);
    let meta = request_meta(&f, "candidate");
    let request = host.request(meta.clone(), "candidate");
    let backend = request
        .backend()
        .with_transport(f.temp.path(), meta.transport.as_ref())
        .unwrap()
        .unwrap();
    (f, host, request, backend)
}
/// The fixture repository's URL. Its path passes as written, as in libgit2.
fn url(f: &common::SshdFixture) -> String {
    format!(
        "ssh://{}@127.0.0.1:{}{}",
        f.user,
        f.port,
        f.repository.display()
    )
}
fn commit(repo: &git2::Repository, text: &str) -> git2::Oid {
    let blob = repo.blob(text.as_bytes()).unwrap();
    let mut b = repo.treebuilder(None).unwrap();
    b.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(b.write().unwrap()).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    repo.set_head("refs/heads/main").unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        text,
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
}
#[test]
fn candidate_backend_clone_fetch_tags_advertisement_manifest_and_push() {
    let (f, host, request, b) = fixture();
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    let first = commit(&server, "first");
    let target = f.temp.path().join("clone");
    let progress = std::sync::atomic::AtomicUsize::new(0);
    b.clone_repo_named(&url(&f), &target, "origin", &|_| {
        progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    })
    .unwrap();
    assert!(progress.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert!(
        b.ls_remote(&target, "origin")
            .unwrap()
            .iter()
            .any(|r| r.target == first.to_string())
    );
    let next = commit(&server, "next");
    b.fetch(&target, "origin").unwrap();
    server
        .tag_lightweight("fixture", &server.find_object(next, None).unwrap(), false)
        .unwrap();
    b.tag_fetch(&target, "origin").unwrap();
    assert_eq!(
        b.read_remote_file(&url(&f), "origin", "payload").unwrap(),
        Some(b"next".to_vec())
    );
    let repo = git2::Repository::open(&target).unwrap();
    assert_eq!(
        repo.find_reference("refs/tags/fixture").unwrap().target(),
        Some(next)
    );
    let local = commit(&repo, "local");
    b.push(&target, "origin", "refs/heads/main:refs/heads/other")
        .unwrap();
    assert_eq!(
        server.find_reference("refs/heads/other").unwrap().target(),
        Some(local)
    );
    let rows = b.transport_observations().unwrap().snapshot();
    assert!(rows.iter().all(|r| r.authenticated == Some(true)));
    assert_eq!(rows.iter().filter(|r| r.credential_offered).count(), 1);
    assert!(
        rows.iter()
            .all(|r| r.credential_method == crate::TransportCredentialMethod::File)
    );
    assert!(!f.marker.exists());
    host.finish(request);
    host.shutdown();
}
#[test]
fn candidate_failed_key_and_trust_have_distinct_facts() {
    let (f, host, request, b) = fixture();
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    commit(&server, "first");
    std::fs::write(f.temp.path().join("authorized_keys"), "").unwrap();
    assert_eq!(
        b.clone_repo(&url(&f), &f.temp.path().join("denied"))
            .unwrap_err()
            .code,
        crate::model::ErrorCode::RemoteRejected
    );
    let row = b
        .transport_observations()
        .unwrap()
        .snapshot()
        .pop()
        .unwrap();
    assert!(row.credential_offered);
    assert_eq!(row.authenticated, Some(false));
    // The rejection closes the key for the rest of the operation (the retry
    // plan's §4): a later clone in it finishes with that rejection at once,
    // and its row reports no offer, since it set nothing up.
    assert_eq!(
        b.clone_repo(&url(&f), &f.temp.path().join("closed"))
            .unwrap_err()
            .code,
        crate::model::ErrorCode::RemoteRejected
    );
    let row = b
        .transport_observations()
        .unwrap()
        .snapshot()
        .pop()
        .unwrap();
    assert!(!row.credential_offered);
    assert_eq!(row.authenticated, None);
    host.finish(request);
    // The next operation starts its keys Cold, and there an untrusted host
    // refuses before any credential is offered.
    std::fs::write(&host.known_hosts, "").unwrap();
    let meta = request_meta(&f, "candidate-untrusted");
    let request = host.request(meta, "candidate-untrusted");
    let scoped = request
        .backend()
        .with_transport(
            f.temp.path(),
            Some(&crate::TransportOptions {
                default_identity: Some(
                    f.temp
                        .path()
                        .join("client_ed25519")
                        .to_string_lossy()
                        .into_owned(),
                ),
                remote_identities: vec![],
                url_scheme: None,
                ..Default::default()
            }),
        )
        .unwrap()
        .unwrap();
    assert!(
        scoped
            .clone_repo(&url(&f), &f.temp.path().join("untrusted"))
            .is_err()
    );
    let row = scoped
        .transport_observations()
        .unwrap()
        .snapshot()
        .pop()
        .unwrap();
    assert!(!row.credential_offered);
    assert_eq!(row.authenticated, None);
    host.finish(request);
    host.shutdown();
}

#[test]
fn candidate_push_rejection_pushurl_and_native_local_route_are_preserved() {
    let (f, host, request, b) = fixture();
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    let first = commit(&server, "first");
    let target = f.temp.path().join("client");
    b.clone_repo(&url(&f), &target).unwrap();
    server
        .config()
        .unwrap()
        .set_bool("receive.denyDeletes", true)
        .unwrap();
    let denied = b.push(&target, "origin", ":refs/heads/main").unwrap_err();
    assert_eq!(denied.code, crate::model::ErrorCode::RemoteRejected);
    assert_eq!(server.head().unwrap().target(), Some(first));
    let alternate = f.temp.path().join("alternate.git");
    let other = git2::Repository::init_bare(&alternate).unwrap();
    let repo = git2::Repository::open(&target).unwrap();
    repo.remote_set_pushurl(
        "origin",
        Some(&format!(
            "ssh://{}@127.0.0.1:{}{}",
            f.user,
            f.port,
            alternate.display()
        )),
    )
    .unwrap();
    b.push(&target, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    assert_eq!(
        other.find_reference("refs/heads/main").unwrap().target(),
        Some(first)
    );
    for scheme in ["git+ssh", "ssh+git"] {
        let alias = url(&f).replacen("ssh://", &format!("{scheme}://"), 1);
        assert!(
            b.ls_remote_url(&target, &alias, "origin", Some(&target))
                .unwrap()
                .iter()
                .any(|r| r.target == first.to_string())
        );
    }
    // A stopped transport host must have no effect on native local transport.
    host.shutdown();
    let local = f.temp.path().join("local");
    b.clone_repo(f.repository.to_str().unwrap(), &local)
        .unwrap();
    assert_eq!(b.head(&local).unwrap().commit, Some(first.to_string()));
    let rows = b.transport_observations().unwrap().snapshot();
    assert_eq!(rows.iter().filter(|r| r.credential_offered).count(), 1);
    assert_eq!(rows.last().unwrap().authenticated, None);
    drop(request);
}

#[test]
fn candidate_backend_clones_and_nested_scopes_share_pool_not_observation_rows() {
    let (f, host, request, b) = fixture();
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    commit(&server, "first");
    b.clone_repo(&url(&f), &f.temp.path().join("one")).unwrap();
    let cloned = b.clone();
    cloned
        .clone_repo(&url(&f), &f.temp.path().join("two"))
        .unwrap();
    assert_eq!(b.transport_observations().unwrap().snapshot().len(), 2);
    let options = crate::TransportOptions {
        default_identity: Some(
            f.temp
                .path()
                .join("client_ed25519")
                .to_string_lossy()
                .into_owned(),
        ),
        ..Default::default()
    };
    let nested = b
        .with_transport(f.temp.path(), Some(&options))
        .unwrap()
        .unwrap();
    nested
        .clone_repo(&url(&f), &f.temp.path().join("three"))
        .unwrap();
    let rows = nested.transport_observations().unwrap().snapshot();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].authenticated, Some(true));
    assert!(!rows[0].credential_offered);
    assert_eq!(b.transport_observations().unwrap().snapshot().len(), 2);
    std::fs::remove_file(f.temp.path().join("client_ed25519")).unwrap();
    assert!(
        nested
            .clone_repo(&url(&f), &f.temp.path().join("missing"))
            .is_err()
    );
    let rows = nested.transport_observations().unwrap().snapshot();
    assert_eq!(
        rows.len(),
        1,
        "existing preflight refuses before a new network attempt"
    );
    host.finish(request);
    host.shutdown();
}
