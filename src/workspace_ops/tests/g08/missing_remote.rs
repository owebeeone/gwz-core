//! A push through a remote name that no selected repository has. The name is
//! not a Git remote of the root or of the member, so every selected target
//! answers `missing_remote`, the code core's remote lookup gives a missing
//! remote (gwz-dev `dev-docs/GwzLocalCloneDesign.md` §6, "Neither → existing
//! `missing_remote`"; `docs/ErrorCatalog.md`). The SSH identity check that
//! runs before the transfer looks the remote up first, and must give the same
//! answer rather than a Git failure.
use crate::git::{Git2Backend, GitBackend};

use super::*;

#[test]
fn a_push_to_a_remote_no_target_has_answers_missing_remote_for_every_target() {
    let temp = TempDir::new("push-missing-remote");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let root_remote = temp.path().join("root.git");
    init_bare_main(&root_remote);
    backend
        .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    let fixture = RemoteFixture::new("push-missing-remote-member");
    let published = fixture.commit_and_push("README.md", "one", "initial", &backend);
    let app = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &app).unwrap();
    let local = commit_file(
        &app,
        "README.md",
        "local",
        "local",
        &[git2::Oid::from_str(&published).unwrap()],
    )
    .unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
    );
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    backend.commit(temp.path(), "lock", false).unwrap();

    for dry_run in [true, false] {
        let response = handle_push(
            &backend,
            temp.path(),
            crate::PushRequest {
                meta: crate::RequestMeta {
                    dry_run: Some(dry_run),
                    ..request_meta_with_workspace()
                },
                remote: Some("A".to_owned()),
                refspec: None,
                remote_check: None,
            },
            "op_push_missing_remote",
        )
        .unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Rejected,
            "dry_run={dry_run}"
        );
        let mut targets = response
            .response
            .members
            .iter()
            .map(|row| {
                let error = row
                    .error
                    .as_ref()
                    .expect("a refused target names its error");
                (
                    row.member_id.as_str(),
                    row.status,
                    error.code,
                    error.message.as_str(),
                )
            })
            .collect::<Vec<_>>();
        targets.sort_by_key(|target| target.0);
        assert_eq!(
            targets,
            [
                (
                    "@root",
                    crate::MemberStatus::Rejected,
                    crate::GwzErrorCode::MissingRemote,
                    "missing remote 'A'",
                ),
                (
                    "mem_app",
                    crate::MemberStatus::Rejected,
                    crate::GwzErrorCode::MissingRemote,
                    "missing remote 'A'",
                ),
            ],
            "dry_run={dry_run}"
        );
    }
    // Nothing was published anywhere.
    assert_eq!(read_repo_ref(&root_remote, "refs/heads/main"), None);
    assert_eq!(
        read_repo_ref(&fixture.remote, "refs/heads/main"),
        Some(published)
    );
}
