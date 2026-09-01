use std::fs;
use std::path::Path;

use crate::artifact::{list_markers, read_lock};
use crate::git::{Git2Backend, GitBackend};

use super::*;

// WS6: `gwz commit` fans out git commit across members + root (root last).

pub(crate) fn set_identity(repo: &Path) {
    let repo = git2::Repository::open(repo).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.name", "GWZ Test").unwrap();
    cfg.set_str("user.email", "gwz@example.invalid").unwrap();
}

fn head_message(repo: &Path) -> String {
    let repo = git2::Repository::open(repo).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    commit.message().unwrap().to_owned()
}

fn trailer_value(message: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}: ");
    message
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(ToOwned::to_owned))
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
    set_identity(temp);
    set_identity(&temp.join("remote"));
    fixture
}

fn commit_request() -> crate::CommitRequest {
    crate::CommitRequest {
        meta: request_meta(),
        message: "do the work".to_owned(),
        all: None,
        commit_marker: None,
    }
}

fn commit_request_for(targets: &[&str]) -> crate::CommitRequest {
    crate::CommitRequest {
        meta: crate::RequestMeta {
            selection: Some(crate::Selection {
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                ..Default::default()
            }),
            ..request_meta()
        },
        ..commit_request()
    }
}

fn dry_run_commit_request() -> crate::CommitRequest {
    crate::CommitRequest {
        meta: crate::RequestMeta {
            dry_run: Some(true),
            ..request_meta()
        },
        ..commit_request()
    }
}

fn root_row(response: &crate::CommitResponse) -> Option<&crate::MemberResponse> {
    response
        .response
        .members
        .iter()
        .find(|member| member.target_kind == Some(crate::TargetKind::Root))
}

fn staged_paths(backend: &Git2Backend, repo: &Path) -> Vec<String> {
    backend
        .status(repo)
        .unwrap()
        .files
        .into_iter()
        .filter(|file| !file.index_status.is_empty() && file.index_status != " ")
        .map(|file| file.path)
        .collect()
}

fn marker_dir_entries(root: &Path) -> Vec<String> {
    let dir = root.join(crate::artifact::MARKER_DIR);
    match fs::read_dir(&dir) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
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

    let member_message = head_message(&member_root);
    let root_message = head_message(temp.path());
    let root_marker_id = trailer_value(&root_message, "GWZ-Commit-ID").unwrap();
    assert_eq!(
        trailer_value(&member_message, "GWZ-Commit-ID").as_deref(),
        Some(root_marker_id.as_str())
    );
    assert_eq!(
        trailer_value(&root_message, "GWZ-Workspace-ID").as_deref(),
        Some("ws_ops")
    );

    let markers = list_markers(temp.path()).unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].gwz_commit_id, root_marker_id);
    assert_eq!(markers[0].members["mem_remote"].commit, after);
    assert!(
        markers[0]
            .committed_targets
            .iter()
            .any(|target| target == "mem_remote")
    );
    assert!(
        markers[0]
            .committed_targets
            .iter()
            .any(|target| target == "@root")
    );
    let repo = git2::Repository::open(temp.path()).unwrap();
    assert!(
        repo.revparse_single(&format!(
            "HEAD:gwz.conf/markers/{}.yaml",
            markers[0].gwz_commit_id
        ))
        .is_ok(),
        "marker artifact was committed in the root"
    );

    // The default selection includes the root, so the root commit is reported too.
    let root = root_row(&response).expect("default-selection commit reports a root row");
    assert_eq!(root.member_id, "@root");
    assert_eq!(root.member_path, ".");
    assert_eq!(
        root.state.as_ref().unwrap().commit,
        backend.head(temp.path()).unwrap().commit,
        "the root row carries the root commit"
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
    assert_eq!(
        response
            .response
            .members
            .iter()
            .filter(|member| member.target_kind == Some(crate::TargetKind::Member))
            .count(),
        0,
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
    assert_eq!(list_markers(temp.path()).unwrap().len(), 1);
    assert!(
        head_message(temp.path()).contains("GWZ-Commit-ID:"),
        "root-only marker commit has trailers"
    );
    assert_eq!(
        root_row(&response)
            .expect("root commit is reported")
            .state
            .as_ref()
            .unwrap()
            .commit,
        after,
        "the root row carries the root commit"
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
    let marker_count = list_markers(temp.path()).unwrap().len();
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
    assert_eq!(
        list_markers(temp.path()).unwrap().len(),
        marker_count,
        "noop does not create another marker"
    );
}

#[test]
fn commit_marker_can_be_disabled() {
    let temp = TempDir::new("commit-marker-disabled");
    let backend = Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "commit-marker-disabled-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();

    let mut request = commit_request();
    request.commit_marker = Some(false);
    let response = handle_commit(&backend, temp.path(), request, "op_commit_no_marker").unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );

    assert!(list_markers(temp.path()).unwrap().is_empty());
    assert!(!head_message(&member_root).contains("GWZ-Commit-ID:"));
    assert!(!head_message(temp.path()).contains("GWZ-Commit-ID:"));
}

// A commit whose selection excludes the root must not create a root commit: the root index
// is the user's, and commit is not pathspec-scoped, so committing it would sweep in
// everything staged there.

#[test]
fn member_selected_commit_leaves_the_root_uncommitted() {
    let temp = TempDir::new("commit-member-only");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-member-only-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    // Give the root a HEAD so "unchanged" is a real comparison, not "still empty".
    handle_commit(&backend, temp.path(), commit_request(), "op_commit_seed").unwrap();

    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();
    let root_before = backend.head(temp.path()).unwrap().commit;
    let markers_before = list_markers(temp.path()).unwrap().len();
    let marker_files_before = marker_dir_entries(temp.path());

    let response = handle_commit(
        &backend,
        temp.path(),
        commit_request_for(&["mem_remote"]),
        "op_commit_member_only",
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );

    // The member committed, with the coalescing trailers.
    let member_after = backend.head(&member_root).unwrap().commit;
    let member_message = head_message(&member_root);
    assert!(trailer_value(&member_message, "GWZ-Commit-ID").is_some());
    assert_eq!(
        trailer_value(&member_message, "GWZ-Workspace-ID").as_deref(),
        Some("ws_ops")
    );

    // The root did not.
    assert_eq!(
        backend.head(temp.path()).unwrap().commit,
        root_before,
        "root HEAD must not move for a member-scoped commit"
    );
    assert_eq!(
        list_markers(temp.path()).unwrap().len(),
        markers_before,
        "no marker artifact for a commit that does not commit the root"
    );
    assert_eq!(
        marker_dir_entries(temp.path()),
        marker_files_before,
        "no marker file was written"
    );

    // The lock records the new member head and is staged in the root index, capture-style.
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"].commit,
        member_after
    );
    assert!(
        staged_paths(&backend, temp.path())
            .iter()
            .any(|path| path.starts_with("gwz.conf/")),
        "gwz.conf is staged in the root index: {:?}",
        staged_paths(&backend, temp.path())
    );

    assert!(
        root_row(&response).is_none(),
        "no root row when the root was not committed"
    );
}

#[test]
fn root_selected_commit_skips_members_and_reports_a_root_row() {
    let temp = TempDir::new("commit-root-selected");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-root-selected-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();
    let member_before = backend.head(&member_root).unwrap().commit;

    let response = handle_commit(
        &backend,
        temp.path(),
        commit_request_for(&["@root"]),
        "op_commit_root_selected",
    )
    .unwrap();

    assert_eq!(
        backend.head(&member_root).unwrap().commit,
        member_before,
        "an unselected member must not be committed"
    );
    assert!(backend.head(temp.path()).unwrap().commit.is_some());
    assert!(head_message(temp.path()).contains("GWZ-Commit-ID:"));
    assert_eq!(
        list_markers(temp.path()).unwrap().len(),
        1,
        "the root commit persists its marker"
    );

    let root = root_row(&response).expect("root commit is reported");
    assert_eq!(
        root.state.as_ref().unwrap().commit,
        backend.head(temp.path()).unwrap().commit
    );
}

#[test]
fn member_selected_commit_does_not_sweep_the_root_index() {
    let temp = TempDir::new("commit-no-overclaim");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-no-overclaim-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    handle_commit(&backend, temp.path(), commit_request(), "op_commit_seed").unwrap();
    let root_before = backend.head(temp.path()).unwrap().commit;

    // The user stages an unrelated file in the root and commits only the member.
    fs::create_dir_all(temp.path().join("dev-docs")).unwrap();
    fs::write(temp.path().join("dev-docs/WorkInProgress.md"), "draft\n").unwrap();
    backend
        .stage_paths(temp.path(), &["dev-docs/WorkInProgress.md"])
        .unwrap();
    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();

    handle_commit(
        &backend,
        temp.path(),
        commit_request_for(&["mem_remote"]),
        "op_commit_member_scoped",
    )
    .unwrap();

    assert_eq!(
        backend.head(temp.path()).unwrap().commit,
        root_before,
        "there is no new root commit to sweep the index into"
    );
    assert!(
        staged_paths(&backend, temp.path())
            .iter()
            .any(|path| path == "dev-docs/WorkInProgress.md"),
        "the user's staged root file is still staged: {:?}",
        staged_paths(&backend, temp.path())
    );
    let repo = git2::Repository::open(temp.path()).unwrap();
    assert!(
        repo.revparse_single("HEAD:dev-docs/WorkInProgress.md")
            .is_err(),
        "the user's staged root file was not committed"
    );
}

#[test]
fn dry_run_commit_mutates_nothing() {
    let temp = TempDir::new("commit-dry-run");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "commit-dry-run-source");

    let member_root = temp.path().join("remote");
    set_identity(&member_root);
    set_identity(temp.path());
    fs::write(member_root.join("work.txt"), "data\n").unwrap();
    backend.stage_paths(&member_root, &["work.txt"]).unwrap();

    let root_head = backend.head(temp.path()).unwrap();
    let member_head = backend.head(&member_root).unwrap();
    let lock_path = temp.path().join(crate::artifact::LOCK_PATH);
    let lock_bytes = fs::read(&lock_path).expect("lock artifact exists");
    let root_index = staged_paths(&backend, temp.path());
    let member_index = staged_paths(&backend, &member_root);
    let markers_before = marker_dir_entries(temp.path());

    let response = handle_commit(
        &backend,
        temp.path(),
        dry_run_commit_request(),
        "op_commit_dry_run",
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(
        response.response.members.is_empty(),
        "dry run reports no rows"
    );

    assert_eq!(backend.head(temp.path()).unwrap(), root_head, "root HEAD");
    assert_eq!(
        backend.head(&member_root).unwrap(),
        member_head,
        "member HEAD"
    );
    assert_eq!(fs::read(&lock_path).unwrap(), lock_bytes, "lock bytes");
    assert_eq!(
        staged_paths(&backend, temp.path()),
        root_index,
        "root index"
    );
    assert_eq!(
        staged_paths(&backend, &member_root),
        member_index,
        "member index"
    );
    assert_eq!(marker_dir_entries(temp.path()), markers_before, "markers");
}
