use std::fs;
use std::path::Path;

use crate::artifact::read_lock;
use crate::git::{Git2Backend, GitBackend};

use super::*;

// WS6: `gwz commit` fans out git commit across members + root (root last).

pub(crate) fn set_identity(repo: &Path) {
    let repo = git2::Repository::open(repo).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.name", "GWZ Test").unwrap();
    cfg.set_str("user.email", "gwz@example.invalid").unwrap();
}

pub(crate) fn init_one_member_workspace(
    temp: &Path,
    backend: &Git2Backend,
    source: &str,
) -> RemoteFixture {
    let fixture = RemoteFixture::new(source);
    fixture.commit_and_push("README.md", "one", "initial", backend);
    let events = CollectingSink::default();
    handle_init_from_sources(
        backend,
        temp,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: temp.to_string_lossy().into_owned(),
            sources: vec![crate::SourceUrl {
                url: fixture.remote_url().to_owned(),
                path: None,
                remote_name: None,
                branch: None,
            }],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &events,
    )
    .unwrap();
    fixture
}

fn commit_request() -> crate::CommitRequest {
    crate::CommitRequest {
        meta: request_meta(),
        message: "do the work".to_owned(),
        all: None,
    }
}

#[test]
fn commit_fans_out_to_members_then_commits_root_last() {
    let temp = TempDir::new("commit-ws");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-ws-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    // A commit-able change staged in the member.
    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();
    let before = backend.head(&member_root).unwrap().commit;

    let response = handle_commit(&backend, temp.path(), commit_request(), "op_commit").unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );

    // The member HEAD advanced and the lock records the new commit.
    let after = backend.head(&member_root).unwrap().commit;
    assert_ne!(before, after);
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"].commit,
        after
    );

    // The root was committed last (the lock update): HEAD has a commit and the working
    // tree is clean — gwz.conf committed and the member hidden via .git/info/exclude.
    assert!(backend.head(temp.path()).unwrap().commit.is_some());
    assert!(
        !backend.status(temp.path()).unwrap().is_dirty,
        "root is clean after commit"
    );
}

#[test]
fn commit_commits_root_only_staged_changes() {
    let temp = TempDir::new("commit-root-only");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-root-only-source");
    set_identity(temp.path());

    fs::create_dir_all(temp.path().join("dev-docs")).unwrap();
    fs::write(
        temp.path().join("dev-docs/ContractManifest.md"),
        "# Contract Manifest\n",
    )
    .unwrap();
    backend
        .stage_paths(temp.path(), &["dev-docs/ContractManifest.md"])
        .unwrap();
    let before = backend.head(temp.path()).unwrap().commit;

    let response = handle_commit(
        &backend,
        temp.path(),
        commit_request(),
        "op_commit_root_only",
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(
        response.response.members.is_empty(),
        "root-only commit should not report member commits"
    );

    let after = backend.head(temp.path()).unwrap().commit;
    assert_ne!(before, after, "root HEAD advanced");
    assert!(
        !backend.status(temp.path()).unwrap().is_dirty,
        "root is clean after root-only commit"
    );

    let repo = git2::Repository::open(temp.path()).unwrap();
    assert!(
        repo.revparse_single("HEAD:dev-docs/ContractManifest.md")
            .is_ok(),
        "root-only staged file was committed"
    );
}

#[test]
fn commit_with_nothing_to_commit_is_a_success_noop() {
    let temp = TempDir::new("commit-noop");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-noop-source");
    set_identity(temp.path());

    // First commit the workspace metadata staged by init so the root is clean.
    handle_commit(&backend, temp.path(), commit_request(), "op_commit_initial").unwrap();
    let before = backend.head(temp.path()).unwrap().commit;
    assert!(before.is_some(), "initial root metadata was committed");
    assert!(
        !backend.status(temp.path()).unwrap().is_dirty,
        "root is clean before noop commit"
    );

    // No changes anywhere → success, nothing committed; the root HEAD stays put.
    let response =
        handle_commit(&backend, temp.path(), commit_request(), "op_commit_noop").unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(response.response.members.is_empty(), "no members committed");
    assert_eq!(
        backend.head(temp.path()).unwrap().commit,
        before,
        "root not committed again"
    );
}
