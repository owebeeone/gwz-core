//! Full backend tests, compiled only by the isolated candidate harness.
use super::*;
use crate::git::endpoint;
#[allow(unused_imports)]
#[path = "../../../tests/transport_ssh/tests/common/mod.rs"]
mod common;
mod drivers;

fn fixture() -> (
    common::SshdFixture,
    Git2Backend,
    endpoint::ssh_worker::Endpoint,
) {
    let f = common::SshdFixture::new();
    let endpoint = endpoint::ssh_local::connect(
        gwz_transport::pool::Config {
            total: 1,
            per_host: 1,
            per_user_host: 1,
            ..Default::default()
        },
        f.known_hosts.clone(),
        None,
        3000,
    )
    .unwrap();
    let mut backend = Git2Backend::new();
    backend.ssh = transport_binding::Runtime::from_endpoint(endpoint.clone());
    let options = crate::TransportOptions {
        default_identity: Some(
            f.temp
                .path()
                .join("client_ed25519")
                .to_string_lossy()
                .into_owned(),
        ),
        remote_identities: Vec::new(),
        url_scheme: None,
        ..Default::default()
    };
    let backend = backend
        .with_transport(f.temp.path(), Some(&options))
        .unwrap()
        .unwrap();
    (f, backend, endpoint)
}
fn url(f: &common::SshdFixture) -> String {
    format!(
        "ssh://{}@127.0.0.1:{}{}",
        f.user,
        f.port,
        f.repository
            .to_str()
            .unwrap()
            .replace('%', "%25")
            .replace(' ', "%20")
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
    let (f, b, e) = fixture();
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
    e.shutdown();
}
#[test]
fn candidate_failed_key_and_trust_have_distinct_facts() {
    let (f, b, e) = fixture();
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
    std::fs::write(&f.known_hosts, "").unwrap();
    let scoped = b
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
    e.shutdown();
}

#[test]
fn candidate_push_rejection_pushurl_and_native_local_route_are_preserved() {
    let (f, b, e) = fixture();
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
    // A stopped SSH endpoint must have no effect on native local transport.
    e.shutdown();
    let local = f.temp.path().join("local");
    b.clone_repo(f.repository.to_str().unwrap(), &local)
        .unwrap();
    assert_eq!(b.head(&local).unwrap().commit, Some(first.to_string()));
    let rows = b.transport_observations().unwrap().snapshot();
    assert_eq!(rows.iter().filter(|r| r.credential_offered).count(), 1);
    assert_eq!(rows.last().unwrap().authenticated, None);
}

#[test]
fn candidate_backend_clones_and_nested_scopes_share_pool_not_observation_rows() {
    let (f, b, e) = fixture();
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
    e.shutdown();
}

#[test]
fn candidate_runtime_retries_transient_failure_and_serializes_family_initialization() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (f, mut b, e) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let owned = e.clone();
    b.ssh = transport_binding::Runtime::with_factory(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(owned.clone())
    });
    assert_eq!(
        b.ssh.endpoint().err().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let options = crate::TransportOptions {
        default_identity: Some(
            f.temp
                .path()
                .join("client_ed25519")
                .to_string_lossy()
                .into(),
        ),
        ..Default::default()
    };
    let scoped = b
        .with_transport(f.temp.path(), Some(&options))
        .unwrap()
        .unwrap();
    std::thread::scope(|scope| {
        for runtime in [b.ssh.clone(), b.clone().ssh, scoped.ssh.clone()] {
            scope.spawn(move || {
                runtime.endpoint().unwrap();
            });
        }
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one failed construction, one shared success"
    );
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    commit(&server, "retry");
    b.clone_repo(&url(&f), &f.temp.path().join("retry-a"))
        .unwrap();
    scoped
        .clone_repo(&url(&f), &f.temp.path().join("retry-b"))
        .unwrap();
    assert_eq!(b.transport_observations().unwrap().snapshot().len(), 1);
    let rows = scoped.transport_observations().unwrap().snapshot();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].credential_offered);
    assert_eq!(rows[0].authenticated, Some(true));
    e.shutdown();
}
