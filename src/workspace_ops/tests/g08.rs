use std::path::Path;

use crate::git::{Git2Backend, GitBackend};

use super::*;

#[test]
pub(crate) fn push_selected_member_to_local_bare_remote_succeeds() {
    let temp = TempDir::new("push-success");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let remote = temp.path().join("remote.git");
    init_bare_main(&remote);
    let repo_path = temp.path().join("repos/app");
    backend.create_repo(&repo_path).unwrap();
    backend
        .add_remote(&repo_path, "origin", remote.to_str().unwrap())
        .unwrap();
    let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", remote.to_str().unwrap(), &commit)],
    );

    let response = handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        response.response.members.single().status,
        crate::MemberStatus::Ok
    );
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), Some(commit));
}

#[test]
pub(crate) fn push_includes_workspace_root_by_default() {
    let temp = TempDir::new("push-root-default");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let remote = temp.path().join("root.git");
    init_bare_main(&remote);
    backend
        .add_remote(temp.path(), "origin", remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    std::fs::write(temp.path().join("root.txt"), "root\n").unwrap();
    backend.stage_paths(temp.path(), &["root.txt"]).unwrap();
    let commit = backend.commit(temp.path(), "root", false).unwrap().commit;

    let response = handle_push(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: request_meta_with_workspace(),
            remote: None,
            refspec: None,
        },
        "op_push",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let root = response.response.members.single();
    assert_eq!(root.member_id, "@root");
    assert_eq!(root.target_kind, Some(crate::TargetKind::Root));
    assert_eq!(root.status, crate::MemberStatus::Ok);
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), Some(commit));
}

#[test]
pub(crate) fn push_honors_request_remote_and_refspec() {
    let temp = TempDir::new("push-refspec");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let remote = temp.path().join("publish.git");
    init_bare_main(&remote);
    let repo_path = temp.path().join("repos/app");
    backend.create_repo(&repo_path).unwrap();
    backend
        .add_remote(&repo_path, "publish", remote.to_str().unwrap())
        .unwrap();
    let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", remote.to_str().unwrap(), &commit)],
    );

    let response = handle_push(
        &backend,
        temp.path(),
        push_request_explicit(
            Some("publish"),
            Some("refs/heads/main:refs/heads/published"),
        ),
        "op_push",
    )
    .unwrap();

    assert_eq!(
        response.response.members.single().status,
        crate::MemberStatus::Ok
    );
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), None);
    assert_eq!(read_repo_ref(&remote, "refs/heads/published"), Some(commit));
}

#[test]
pub(crate) fn push_local_only_member_without_remote_fails_or_skips_by_policy() {
    let temp = TempDir::new("push-local-only");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    handle_create_repo(
        &backend,
        temp.path(),
        create_repo_request("repos/app", None, None),
        "op_repo",
    )
    .unwrap();

    let failed = handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();
    assert_eq!(
        failed.response.meta.aggregate_status,
        crate::AggregateStatus::Rejected
    );
    assert_eq!(
        failed.response.members.single().status,
        crate::MemberStatus::Rejected
    );
    assert_eq!(
        failed
            .response
            .members
            .single()
            .error
            .as_ref()
            .unwrap()
            .code,
        crate::GwzErrorCode::MissingRemote
    );

    let skipped = handle_push(
        &backend,
        temp.path(),
        push_request(Some(crate::UnsupportedMemberBehavior::Skip), None),
        "op_push",
    )
    .unwrap();
    assert_eq!(
        skipped.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert_eq!(
        skipped.response.members.single().status,
        crate::MemberStatus::Skipped
    );
}

#[test]
pub(crate) fn push_remote_rejection_is_reported_per_member() {
    let temp = TempDir::new("push-reject");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new("push-reject-source");
    let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
    backend
        .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
        .unwrap();
    let remote_second = fixture.commit_and_push("README.md", "two", "second", &backend);
    let first_oid = git2::Oid::from_str(&first).unwrap();
    let local = commit_file(
        &temp.path().join("repos/app"),
        "README.md",
        "local",
        "local",
        &[first_oid],
    )
    .unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
    );

    let response = handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Failed
    );
    let member = response.response.members.single();
    assert_eq!(member.status, crate::MemberStatus::Failed);
    assert_eq!(
        member.error.as_ref().unwrap().code,
        crate::GwzErrorCode::RemoteRejected
    );
    assert_eq!(
        read_repo_ref(Path::new(fixture.remote_url()), "refs/heads/main"),
        Some(remote_second)
    );
}

#[test]
pub(crate) fn push_rejects_whole_batch_when_a_member_fails_preflight() {
    // F7/Q2: preflight all members before pushing any. A valid pushable member
    // (app) alongside an unmaterialized member (lib) must reject the whole push
    // WITHOUT advancing the valid member's remote.
    let temp = TempDir::new("push-reject-batch");
    let backend = Git2Backend::new();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let remote = temp.path().join("remote.git");
    init_bare_main(&remote);
    let app = temp.path().join("repos/app");
    backend.create_repo(&app).unwrap();
    backend
        .add_remote(&app, "origin", remote.to_str().unwrap())
        .unwrap();
    let commit = commit_file(&app, "README.md", "one", "initial", &[]).unwrap();
    // Two members: app (materialized) and lib (in the lock, NOT materialized).
    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_app", "repos/app", remote.to_str().unwrap(), &commit),
            ("mem_lib", "repos/lib", remote.to_str().unwrap(), &commit),
        ],
    );

    let response = handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Rejected
    );
    // The valid member must NOT have been pushed — nothing changed.
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), None);
}

/// Design §7 names only `PushRequest.remote` for `gwz push --remote <name>`,
/// and the operator's cross-driver ruling 4 (2026-09-06, LCM1.0c follow-up 3)
/// has both drivers encode the token there once, no longer also in
/// `OperationPolicy.remote`. Core needs no change for that: both resolvers
/// read the request field first and fall back to the policy field for a
/// caller that still sets only that (request-over-policy precedence,
/// boundaries §3). A characterisation pin, not a behaviour change -- it was
/// green on first run.
#[test]
pub(crate) fn push_remote_binds_from_the_request_field_first_and_the_policy_second() {
    use crate::workspace_ops::push_member::{resolve_push_remote, resolve_root_push_remote};

    let member = crate::artifact::ManifestMember {
        id: "mem_app".to_owned(),
        path: "repos/app".to_owned(),
        source_kind: crate::artifact::ArtifactSourceKind::Git,
        source_id: "src_app".to_owned(),
        active: true,
        desired: None,
        remotes: vec![crate::artifact::RemoteArtifact {
            name: "origin".to_owned(),
            url: "file:///nowhere/origin.git".to_owned(),
            fetch: true,
            push: true,
        }],
    };
    let mut both = push_request(None, Some("policy-hub"));
    both.remote = Some("hub".to_owned());
    let request_only = push_request_explicit(Some("hub"), None);
    let policy_only = push_request(None, Some("hub"));
    let neither = push_request_explicit(None, None);

    assert_eq!(resolve_push_remote(&member, &both).unwrap(), "hub");
    assert_eq!(resolve_push_remote(&member, &request_only).unwrap(), "hub");
    assert_eq!(resolve_push_remote(&member, &policy_only).unwrap(), "hub");
    assert_eq!(
        resolve_push_remote(&member, &neither).unwrap(),
        "origin",
        "no token at all: the member's own push remote"
    );

    let temp = TempDir::new("push-remote-precedence");
    let backend = Git2Backend::new();
    let root = temp.path().join("root");
    backend.create_repo(&root).unwrap();
    assert_eq!(
        resolve_root_push_remote(&backend, &root, &both).unwrap(),
        "hub"
    );
    assert_eq!(
        resolve_root_push_remote(&backend, &root, &request_only).unwrap(),
        "hub"
    );
    assert_eq!(
        resolve_root_push_remote(&backend, &root, &policy_only).unwrap(),
        "hub"
    );
    assert_eq!(
        resolve_root_push_remote(&backend, &root, &neither)
            .unwrap_err()
            .code,
        crate::model::ErrorCode::MissingRemote,
        "a root with no remote and no token is the existing missing_remote"
    );
}

pub(crate) fn push_request(
    unsupported_member: Option<crate::UnsupportedMemberBehavior>,
    remote: Option<&str>,
) -> crate::PushRequest {
    crate::PushRequest {
        meta: crate::RequestMeta {
            selection: Some(member_only_push_selection()),
            policy: Some(crate::OperationPolicy {
                unsupported_member,
                remote: remote.map(ToOwned::to_owned),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
        remote: None,
        refspec: None,
    }
}

pub(crate) fn push_request_explicit(
    remote: Option<&str>,
    refspec: Option<&str>,
) -> crate::PushRequest {
    crate::PushRequest {
        meta: crate::RequestMeta {
            selection: Some(member_only_push_selection()),
            ..request_meta_with_workspace()
        },
        remote: remote.map(ToOwned::to_owned),
        refspec: refspec.map(ToOwned::to_owned),
    }
}

fn member_only_push_selection() -> crate::Selection {
    crate::Selection {
        targets: vec!["@all".to_owned()],
        exclude_targets: vec!["@root".to_owned()],
        ..Default::default()
    }
}

pub(crate) fn read_repo_ref(repo_path: &Path, ref_name: &str) -> Option<String> {
    let repo = git2::Repository::open(repo_path).unwrap();
    repo.find_reference(ref_name)
        .ok()
        .and_then(|reference| reference.target())
        .map(|target| target.to_string())
}

pub(crate) fn init_bare_main(path: &Path) {
    let repo = git2::Repository::init_bare(path).unwrap();
    repo.set_head("refs/heads/main").unwrap();
}

impl RemoteFixture {
    pub(crate) fn new(prefix: &str) -> Self {
        let temp = TempDir::new(prefix);
        let source = temp.path().join("source");
        let remote = temp.path().join("remote.git");
        Git2Backend::new().create_repo(&source).unwrap();
        init_bare_main(&remote);
        Git2Backend::new()
            .add_remote(&source, "origin", remote.to_str().unwrap())
            .unwrap();
        Self {
            _temp: temp,
            source,
            remote,
        }
    }

    pub(crate) fn remote_url(&self) -> &str {
        self.remote.to_str().unwrap()
    }

    pub(crate) fn commit_and_push(
        &self,
        relative_path: &str,
        content: &str,
        message: &str,
        backend: &Git2Backend,
    ) -> String {
        let parent = backend
            .head(&self.source)
            .unwrap()
            .commit
            .and_then(|commit| git2::Oid::from_str(&commit).ok());
        let parents = parent.into_iter().collect::<Vec<_>>();
        let commit = commit_file(&self.source, relative_path, content, message, &parents).unwrap();
        backend
            .push(&self.source, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();
        commit
    }
}
