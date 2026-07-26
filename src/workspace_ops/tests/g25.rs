use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;
use crate::workspace_ops::pull_head_barrier::before_next_pull_barrier;

use super::*;

struct DivergedMember {
    fixture: RemoteFixture,
    path: PathBuf,
    local: String,
    remote: String,
}

struct RootFastForward {
    _temp: TempDir,
    before: String,
    remote: String,
}

fn seed_root_fast_forward(backend: &Git2Backend, root: &Path) -> RootFastForward {
    fs::write(root.join(".gitignore"), "repos/\n").unwrap();
    backend
        .stage_paths(root, &["gwz.conf", ".gitignore"])
        .unwrap();
    let before = commit_file(root, "root.txt", "root\n", "root baseline", &[]).unwrap();
    let temp = TempDir::new("pull-race-root-remote");
    let bare = temp.path().join("remote.git");
    init_bare_main(&bare);
    backend
        .add_remote(root, "origin", bare.to_str().unwrap())
        .unwrap();
    backend
        .push(root, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let peer = temp.path().join("peer");
    backend.clone_repo(bare.to_str().unwrap(), &peer).unwrap();
    let remote = commit_file(
        &peer,
        "remote-root.txt",
        "remote\n",
        "remote root",
        &[git2::Oid::from_str(&before).unwrap()],
    )
    .unwrap();
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    RootFastForward {
        _temp: temp,
        before,
        remote,
    }
}

fn seed_behind_member(
    backend: &Git2Backend,
    root: &Path,
    path: &str,
    fixture_name: &str,
) -> (RemoteFixture, String, String) {
    let fixture = RemoteFixture::new(fixture_name);
    let base = fixture.commit_and_push("README.md", "base\n", "base", backend);
    let member = root.join(path);
    backend.clone_repo(fixture.remote_url(), &member).unwrap();
    let remote = fixture.commit_and_push("README.md", "remote\n", "remote", backend);
    (fixture, base, remote)
}

fn seed_clean_diverged_member(
    backend: &Git2Backend,
    root: &Path,
    path: &str,
    fixture_name: &str,
) -> DivergedMember {
    let fixture = RemoteFixture::new(fixture_name);
    let base = fixture.commit_and_push("README.md", "base\n", "base", backend);
    let member = root.join(path);
    backend.clone_repo(fixture.remote_url(), &member).unwrap();
    let local = commit_file(
        &member,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    let remote = fixture.commit_and_push("remote.txt", "remote\n", "remote", backend);
    DivergedMember {
        fixture,
        path: member,
        local,
        remote,
    }
}

fn set_ref(path: &Path, name: &str, target: &str) {
    let repo = git2::Repository::open(path).unwrap();
    repo.reference(
        name,
        git2::Oid::from_str(target).unwrap(),
        true,
        "test race",
    )
    .unwrap();
}

fn merge_request_for(targets: Vec<&str>) -> crate::PullHeadRequest {
    let mut request = pull_head_request_with_sync(crate::SyncBehavior::Merge);
    if !targets.is_empty() {
        request.meta.selection = Some(crate::Selection {
            targets: targets.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        });
    }
    request
}

#[test]
fn final_pull_barrier_rejects_late_member_ref_drift_before_any_local_mutation() {
    let temp = TempDir::new("pull-final-barrier");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let (first_fixture, first_local, _first_remote) =
        seed_behind_member(&backend, temp.path(), "repos/first", "pull-barrier-first");
    let second =
        seed_clean_diverged_member(&backend, temp.path(), "repos/second", "pull-barrier-second");
    write_pull_fixture(
        temp.path(),
        vec![
            (
                "mem_first",
                "repos/first",
                first_fixture.remote_url(),
                &first_local,
            ),
            (
                "mem_second",
                "repos/second",
                second.fixture.remote_url(),
                &second.local,
            ),
        ],
    );
    let root_remote = seed_root_fast_forward(&backend, temp.path());

    let root_head = backend.head(temp.path()).unwrap();
    let root_lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let first_head = backend.head(&temp.path().join("repos/first")).unwrap();
    let first_index = fs::read(temp.path().join("repos/first/.git/index")).unwrap();
    let first_worktree = fs::read(temp.path().join("repos/first/README.md")).unwrap();
    let second_head = backend.head(&second.path).unwrap();
    let second_index = fs::read(second.path.join(".git/index")).unwrap();
    let second_local_file = fs::read(second.path.join("local.txt")).unwrap();

    let raced_path = second.path.clone();
    let raced_target = second.local.clone();
    before_next_pull_barrier(move || {
        set_ref(&raced_path, "refs/remotes/origin/main", &raced_target);
    });

    let error = handle_pull_head(
        &backend,
        temp.path(),
        merge_request_for(Vec::new()),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_second"));
    assert_eq!(error.member_path.as_deref(), Some("repos/second"));
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(
        root_head.commit.as_deref(),
        Some(root_remote.before.as_str())
    );
    assert!(!temp.path().join("remote-root.txt").exists());
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        root_lock
    );
    assert_eq!(
        backend.head(&temp.path().join("repos/first")).unwrap(),
        first_head
    );
    assert_eq!(
        fs::read(temp.path().join("repos/first/.git/index")).unwrap(),
        first_index
    );
    assert_eq!(
        fs::read(temp.path().join("repos/first/README.md")).unwrap(),
        first_worktree
    );
    assert_eq!(backend.head(&second.path).unwrap(), second_head);
    assert_eq!(
        fs::read(second.path.join(".git/index")).unwrap(),
        second_index
    );
    assert_eq!(
        fs::read(second.path.join("local.txt")).unwrap(),
        second_local_file
    );
    assert!(backend.merge_state(&second.path).unwrap().is_none());
}

#[test]
fn pull_executes_the_frozen_source_when_remote_ref_moves_after_the_barrier() {
    let temp = TempDir::new("pull-exact-source");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let member = seed_clean_diverged_member(
        &backend,
        temp.path(),
        "repos/app",
        "pull-exact-source-member",
    );
    write_pull_fixture(
        temp.path(),
        vec![(
            "mem_app",
            "repos/app",
            member.fixture.remote_url(),
            &member.local,
        )],
    );

    let raced_path = member.path.clone();
    let raced_target = member.local.clone();
    Git2Backend::before_next_prepared_execution(move || {
        set_ref(&raced_path, "refs/remotes/origin/main", &raced_target);
    });

    let response = handle_pull_head(
        &backend,
        temp.path(),
        merge_request_for(vec!["mem_app"]),
        "op_pull",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let result = backend.head(&member.path).unwrap().commit.unwrap();
    assert!(
        backend
            .commit_matches_merge(
                &member.path,
                &result,
                &member.local,
                &member.remote,
                "Merge refs/remotes/origin/main into main",
            )
            .unwrap()
    );
    assert_eq!(
        backend
            .read_ref(&member.path, "refs/remotes/origin/main")
            .unwrap()
            .as_deref(),
        Some(member.local.as_str())
    );
    assert!(backend.merge_state(&member.path).unwrap().is_none());
}

#[test]
fn mixed_pull_executes_frozen_root_and_member_sources_after_remote_refs_move() {
    let temp = TempDir::new("pull-exact-mixed");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let member = seed_clean_diverged_member(
        &backend,
        temp.path(),
        "repos/app",
        "pull-exact-mixed-member",
    );
    write_pull_fixture(
        temp.path(),
        vec![(
            "mem_app",
            "repos/app",
            member.fixture.remote_url(),
            &member.local,
        )],
    );
    let root_remote = seed_root_fast_forward(&backend, temp.path());

    let root_path = temp.path().to_path_buf();
    let root_race_target = root_remote.before.clone();
    let member_path = member.path.clone();
    let member_race_target = member.local.clone();
    Git2Backend::before_next_prepared_execution(move || {
        set_ref(&root_path, "refs/remotes/origin/main", &root_race_target);
        set_ref(
            &member_path,
            "refs/remotes/origin/main",
            &member_race_target,
        );
    });

    let response = handle_pull_head(
        &backend,
        temp.path(),
        merge_request_for(Vec::new()),
        "op_pull",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_remote.remote.as_str())
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("remote-root.txt")).unwrap(),
        "remote\n"
    );
    let member_result = backend.head(&member.path).unwrap().commit.unwrap();
    assert!(
        backend
            .commit_matches_merge(
                &member.path,
                &member_result,
                &member.local,
                &member.remote,
                "Merge refs/remotes/origin/main into main",
            )
            .unwrap()
    );
    assert_eq!(
        backend
            .read_ref(temp.path(), "refs/remotes/origin/main")
            .unwrap()
            .as_deref(),
        Some(root_remote.before.as_str())
    );
    assert_eq!(
        backend
            .read_ref(&member.path, "refs/remotes/origin/main")
            .unwrap()
            .as_deref(),
        Some(member.local.as_str())
    );
    assert!(backend.merge_state(temp.path()).unwrap().is_none());
    assert!(backend.merge_state(&member.path).unwrap().is_none());
}

#[test]
fn up_to_date_member_is_rechecked_during_execution_with_member_context() {
    let temp = TempDir::new("pull-up-to-date-execution-race");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-up-to-date-execution-race-member");
    let base = fixture.commit_and_push("README.md", "base\n", "base", &backend);
    let member = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &member).unwrap();
    let raced = commit_file(
        &member,
        "raced.txt",
        "raced\n",
        "raced",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.reset_hard(&member, "main", &base).unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &base)],
    );
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let raced_path = member.clone();
    let raced_target = raced.clone();
    Git2Backend::before_next_prepared_execution(move || {
        set_ref(&raced_path, "refs/heads/main", &raced_target);
    });

    let error = handle_pull_head(
        &backend,
        temp.path(),
        merge_request_for(vec!["mem_app"]),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_app"));
    assert_eq!(error.member_path.as_deref(), Some("repos/app"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(raced.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
}

#[test]
fn final_pull_barrier_reports_target_branch_drift_with_member_context() {
    let temp = TempDir::new("pull-target-drift");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let member = seed_clean_diverged_member(
        &backend,
        temp.path(),
        "repos/app",
        "pull-target-drift-member",
    );
    write_pull_fixture(
        temp.path(),
        vec![(
            "mem_app",
            "repos/app",
            member.fixture.remote_url(),
            &member.local,
        )],
    );

    let raced_path = member.path.clone();
    let raced_target = member.remote.clone();
    before_next_pull_barrier(move || {
        set_ref(&raced_path, "refs/heads/main", &raced_target);
    });

    let error = handle_pull_head(
        &backend,
        temp.path(),
        merge_request_for(vec!["mem_app"]),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_app"));
    assert_eq!(error.member_path.as_deref(), Some("repos/app"));
    assert!(backend.merge_state(&member.path).unwrap().is_none());
}
