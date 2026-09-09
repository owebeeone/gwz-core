use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

struct RefusingServer {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl RefusingServer {
    fn new(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/private.git", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let _ = stream.read(&mut [0; 4096]);
                        let challenge = if status == 401 {
                            "WWW-Authenticate: Basic realm=\"fixture\"\r\n"
                        } else {
                            ""
                        };
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status} Refused\r\n{challenge}Content-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture listener failed: {error}"),
                }
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for RefusingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn private_member_clone_access_refusals_are_quiet_and_preserve_public_clones() {
    for status in [401, 403, 404] {
        clone_case(status, true, true);
    }
}

#[test]
fn public_member_access_refusal_and_private_server_failure_remain_errors() {
    clone_case(404, false, false);
    clone_case(500, true, false);
}

fn clone_case(status: u16, private: bool, succeeds: bool) {
    let temp = TempDir::new("private-member-clone");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    handle_create_workspace(create_workspace_request(&source), "create").unwrap();
    let remote = RemoteFixture::new("private-member-public");
    let commit = remote.commit_and_push("README.md", "public", "initial", &backend);
    let server = RefusingServer::new(status);
    write_pull_fixture(
        &source,
        vec![
            ("mem_app", "repos/app", remote.remote_url(), &commit),
            ("mem_secret", "repos/secret", &server.url, &commit),
        ],
    );
    // Inject YAML so the pre-feature code can execute this regression test.
    let path = source.join("gwz.conf/gwz.yml");
    let mut manifest: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["members"][1]["private"] = serde_yaml::Value::Bool(private);
    fs::write(&path, serde_yaml::to_string(&manifest).unwrap()).unwrap();
    commit_workspace_root(&source);
    let original_lock = fs::read(source.join("gwz.conf/gwz.lock.yml")).unwrap();
    let target = temp.path().join("target");
    let events = CollectingSink::default();
    let result = handle_clone_workspace(
        &backend,
        request_meta(),
        source.to_str().unwrap(),
        target.to_str().unwrap(),
        "clone",
        &events,
    );
    if succeeds {
        let response = result.unwrap();
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert!(target.join("repos/app/README.md").exists());
        assert!(!target.join("repos/secret").exists());
        assert_eq!(
            fs::read(target.join("gwz.conf/gwz.lock.yml")).unwrap(),
            original_lock
        );
        assert_eq!(read_manifest(&target).unwrap().members.len(), 2);
        assert!(
            !response
                .response
                .meta
                .transport
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|row| Path::new(&row.repository_path) == target.join("repos/secret"))
        );
        let rendered = format!("{response:?} {:?}", events.take());
        assert!(!rendered.contains("mem_secret"), "{rendered}");
        assert!(!rendered.contains("repos/secret"), "{rendered}");
    } else {
        assert!(
            result.is_err(),
            "public access and non-access failures must remain errors"
        );
        assert!(
            !target.join("repos/app").exists(),
            "ordinary rollback remains unchanged"
        );
    }
}

#[test]
fn private_member_policy_round_trips_and_sync_preserves_or_clears_it() {
    let temp = TempDir::new("private-member-policy");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    handle_create_repo(
        &backend,
        temp.path(),
        create_repo_request("repos/app", None, None),
        "member",
    )
    .unwrap();
    assert!(!read_manifest(temp.path()).unwrap().members[0].private);
    for (private, expected, dry_run) in [
        (Some(true), false, true),
        (Some(true), true, false),
        (None, true, false),
        (Some(false), false, false),
    ] {
        let before = fs::read(temp.path().join("gwz.conf/gwz.yml")).unwrap();
        let request = crate::RepoSyncRequest {
            private,
            meta: crate::RequestMeta {
                dry_run: Some(dry_run),
                ..request_meta()
            },
        };
        handle_repo_sync(&backend, temp.path(), request, "sync").unwrap();
        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(manifest.members[0].private, expected);
        let bytes = fs::read(temp.path().join("gwz.conf/gwz.yml")).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&bytes).contains("private: true"),
            expected
        );
        if dry_run {
            assert_eq!(bytes, before);
        }
    }
}

#[test]
fn private_accessible_members_are_materialized_and_reported() {
    let temp = TempDir::new("private-member-accessible");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let remote = RemoteFixture::new("private-member-accessible-source");
    let commit = remote.commit_and_push("README.md", "available", "initial", &backend);
    write_materialize_fixture(temp.path(), remote.remote_url(), &commit);
    let mut manifest = read_manifest(temp.path()).unwrap();
    manifest.members[0].private = true;
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
    let response = handle_materialize(
        &backend,
        temp.path(),
        materialize_lock_request(false),
        "materialize",
        &NullSink,
    )
    .unwrap();
    assert_eq!(response.response.members.len(), 1);
    assert!(temp.path().join("repos/app/README.md").exists());
}

#[test]
fn private_policy_does_not_hide_root_access_failure() {
    let temp = TempDir::new("private-root-refusal");
    let server = RefusingServer::new(404);
    let result = handle_clone_workspace(
        &Git2Backend::new(),
        request_meta(),
        &server.url,
        temp.path().join("target").to_str().unwrap(),
        "clone",
        &NullSink,
    );
    assert!(result.is_err());
}

#[test]
fn private_policy_does_not_hide_or_delete_a_preexisting_nonrepo_directory() {
    let temp = TempDir::new("private-member-destination");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let server = RefusingServer::new(404);
    write_materialize_fixture(temp.path(), &server.url, "deadbeef");
    let mut manifest = read_manifest(temp.path()).unwrap();
    manifest.members[0].private = true;
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
    fs::create_dir_all(temp.path().join("repos/app")).unwrap();
    let sentinel = temp.path().join("repos/app/keep.txt");
    fs::write(&sentinel, "existing work").unwrap();
    let result = handle_materialize(
        &backend,
        temp.path(),
        materialize_lock_request(false),
        "materialize",
        &NullSink,
    );
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "existing work");
}

#[test]
fn workspace_clone_with_only_inaccessible_private_members_succeeds_quietly() {
    let temp = TempDir::new("private-only-workspace");
    let backend = Git2Backend::new();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    handle_create_workspace(create_workspace_request(&source), "create").unwrap();
    let server = RefusingServer::new(404);
    write_materialize_fixture(&source, &server.url, "deadbeef");
    let mut manifest = read_manifest(&source).unwrap();
    manifest.members[0].private = true;
    crate::artifact::write_manifest(&source, &manifest).unwrap();
    commit_workspace_root(&source);
    let target = temp.path().join("target");
    let events = CollectingSink::default();
    let response = handle_clone_workspace(
        &backend,
        request_meta(),
        source.to_str().unwrap(),
        target.to_str().unwrap(),
        "clone",
        &events,
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(response.response.members.is_empty());
    assert_eq!(read_manifest(&target).unwrap(), manifest);
    assert!(!target.join("repos/app").exists());
    assert!(!format!("{response:?} {:?}", events.take()).contains("mem_app"));
}
