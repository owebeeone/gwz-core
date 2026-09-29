use std::fs;

use crate::artifact::read_lock;
use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;

use super::*;

#[test]
pub(crate) fn pull_head_dirty_member_blocks_all_selected_members_before_mutation() {
    let temp = TempDir::new("pull-dirty");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let good = RemoteFixture::new("pull-dirty-good");
    let good_first = good.commit_and_push("README.md", "one", "initial", &backend);
    backend
        .clone_repo(good.remote_url(), &temp.path().join("repos/good"))
        .unwrap();
    let good_second = good.commit_and_push("README.md", "two", "second", &backend);

    let dirty = RemoteFixture::new("pull-dirty-bad");
    let dirty_first = dirty.commit_and_push("README.md", "one", "initial", &backend);
    backend
        .clone_repo(dirty.remote_url(), &temp.path().join("repos/dirty"))
        .unwrap();
    fs::write(temp.path().join("repos/dirty/README.md"), "dirty").unwrap();

    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_good", "repos/good", good.remote_url(), &good_first),
            ("mem_dirty", "repos/dirty", dirty.remote_url(), &dirty_first),
        ],
    );
    let lock_before = read_lock(temp.path()).unwrap();

    let err = handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap_err();

    assert_eq!(err.code, ErrorCode::DirtyMember);
    // Q1: the dirty member fails the non-mutating ls_remote validation pass BEFORE any
    // fetch, so the clean sibling's remote-tracking ref must not have advanced — it was
    // never fetched. (Pre-Q1 the sibling fetched during preflight and this would fail.)
    assert_eq!(
        backend
            .read_ref(&temp.path().join("repos/good"), "refs/remotes/origin/main")
            .unwrap(),
        Some(good_first.clone())
    );
    assert_eq!(
        backend
            .head(&temp.path().join("repos/good"))
            .unwrap()
            .commit,
        Some(good_first)
    );
    assert_ne!(
        backend
            .head(&temp.path().join("repos/good"))
            .unwrap()
            .commit,
        Some(good_second)
    );
    assert_eq!(read_lock(temp.path()).unwrap(), lock_before);
}

#[test]
pub(crate) fn pull_head_from_a_remote_no_target_has_answers_missing_remote_before_any_fetch() {
    // A `--remote` name that is a Git remote of neither the root nor the
    // member is the existing `missing_remote` (gwz-dev
    // `dev-docs/GwzLocalCloneDesign.md` §6, "Neither → existing
    // `missing_remote`"). The SSH identity check that runs before any fetch
    // looks the remote up first, and must give that answer rather than a Git
    // failure.
    let temp = TempDir::new("pull-missing-remote");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("pull-missing-remote-member");
    let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
    let app = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &app).unwrap();
    fixture.commit_and_push("README.md", "two", "second", &backend);
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &first)],
    );
    let lock_before = read_lock(temp.path()).unwrap();
    let request = crate::PullHeadRequest {
        meta: crate::RequestMeta {
            policy: Some(crate::OperationPolicy {
                remote: Some("A".to_owned()),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
    };

    let error = handle_pull_head(&backend, temp.path(), request, "op_pull").unwrap_err();

    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::MissingRemote, "missing remote 'A'")
    );
    assert_eq!(
        backend.read_ref(&app, "refs/remotes/origin/main").unwrap(),
        Some(first.clone())
    );
    assert_eq!(backend.head(&app).unwrap().commit, Some(first));
    assert_eq!(read_lock(temp.path()).unwrap(), lock_before);
}

#[test]
pub(crate) fn pull_head_unreachable_remote_blocks_fetch_of_all_members() {
    // Q1: a member whose remote is unreachable fails the ls_remote validation pass
    // BEFORE any fetch, so a sibling with a good remote is never fetched.
    let temp = TempDir::new("pull-unreachable");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    // Good member: a real remote that is ahead (a pull would fast-forward it).
    let good = RemoteFixture::new("pull-unreachable-good");
    let good_first = good.commit_and_push("README.md", "one", "initial", &backend);
    backend
        .clone_repo(good.remote_url(), &temp.path().join("repos/good"))
        .unwrap();
    good.commit_and_push("README.md", "two", "second", &backend);

    // Bad member: clone a real repo, then point its origin at a nonexistent path so
    // ls_remote cannot connect.
    let bad = RemoteFixture::new("pull-unreachable-bad");
    let bad_first = bad.commit_and_push("README.md", "one", "initial", &backend);
    let bad_path = temp.path().join("repos/bad");
    backend.clone_repo(bad.remote_url(), &bad_path).unwrap();
    let bad_url = "file:///gwz/does-not-exist.git";
    git2::Repository::open(&bad_path)
        .unwrap()
        .remote_set_url("origin", bad_url)
        .unwrap();

    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_good", "repos/good", good.remote_url(), &good_first),
            ("mem_bad", "repos/bad", bad_url, &bad_first),
        ],
    );

    let result = handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull");
    assert!(result.is_err());

    // The good member was never fetched: its remote-tracking ref did not advance.
    assert_eq!(
        backend
            .read_ref(&temp.path().join("repos/good"), "refs/remotes/origin/main")
            .unwrap(),
        Some(good_first)
    );
}
