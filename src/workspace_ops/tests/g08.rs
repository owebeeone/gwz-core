use std::path::Path;

use crate::git::{Git2Backend, GitBackend};

use super::*;

#[test]
fn unknown_identity_override_refuses_before_root_publication() {
    let temp = TempDir::new("push-unknown-identity");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let remote = temp.path().join("root.git");
    init_bare_main(&remote);
    backend
        .add_remote(temp.path(), "origin", remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    commit_file(temp.path(), "root.txt", "work", "work", &[]).unwrap();
    let key = temp.path().join("unused-key");
    std::fs::write(&key, "fixture: this unused key must never be offered").unwrap();
    let result = handle_push(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".into()],
                    ..Default::default()
                }),
                transport: Some(crate::TransportOptions {
                    url_scheme: None,
                    default_identity: None,
                    remote_identities: vec![crate::RemoteSshIdentity {
                        remote: "typo".into(),
                        private_key_path: key.to_str().unwrap().into(),
                    }],
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: None,
        },
        "push",
    );
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), None);
    assert_eq!(
        result.unwrap_err().code,
        crate::model::ErrorCode::InvalidRequest
    );
}

#[test]
fn member_rejection_leaves_selected_root_remote_unchanged() {
    let temp = TempDir::new("push-root-barrier");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let root_remote = temp.path().join("root.git");
    init_bare_main(&root_remote);
    backend
        .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    let root_before = commit_file(temp.path(), "root.txt", "before", "before", &[]).unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    let fixture = RemoteFixture::new("push-barrier-member");
    let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
    let app = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &app).unwrap();
    let remote_after = fixture.commit_and_push("README.md", "remote", "remote", &backend);
    let local = commit_file(
        &app,
        "README.md",
        "local",
        "local",
        &[git2::Oid::from_str(&first).unwrap()],
    )
    .unwrap();
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
    );
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_after = backend
        .commit(temp.path(), "publish lock", false)
        .unwrap()
        .commit;
    assert_ne!(root_before, root_after);

    let response = handle_push(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: request_meta_with_workspace(),
            remote: None,
            refspec: None,
        },
        "op_push_barrier",
    )
    .unwrap();
    // Check actual remote state before aggregate reporting, the safety property.
    assert_eq!(
        read_repo_ref(&root_remote, "refs/heads/main"),
        Some(root_before)
    );
    assert_eq!(
        read_repo_ref(&fixture.remote, "refs/heads/main"),
        Some(remote_after)
    );
    let member = response
        .response
        .members
        .iter()
        .find(|row| row.member_id == "mem_app")
        .unwrap();
    assert_eq!(member.status, crate::MemberStatus::Failed);
    assert_eq!(
        member.error.as_ref().unwrap().code,
        crate::GwzErrorCode::RemoteRejected
    );
    let root = response
        .response
        .members
        .iter()
        .find(|row| row.member_id == "@root")
        .unwrap();
    assert_eq!(root.status, crate::MemberStatus::Rejected);
}

#[test]
fn root_only_push_requires_the_committed_locks_member_objects() {
    for active in [true, false] {
        let temp = TempDir::new("push-root-only-dependencies");
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let root_remote = temp.path().join("root.git");
        let member_remote = temp.path().join("app.git");
        init_bare_main(&root_remote);
        init_bare_main(&member_remote);
        backend
            .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
            .unwrap();
        let app = temp.path().join("repos/app");
        backend.create_repo(&app).unwrap();
        backend
            .add_remote(&app, "origin", member_remote.to_str().unwrap())
            .unwrap();
        let member_commit = commit_file(&app, "README.md", "one", "one", &[]).unwrap();
        write_pull_fixture(
            temp.path(),
            vec![(
                "mem_app",
                "repos/app",
                member_remote.to_str().unwrap(),
                &member_commit,
            )],
        );
        let mut manifest = crate::artifact::read_manifest(temp.path()).unwrap();
        manifest.members[0].active = active;
        manifest.members[0].remotes[0].push = active;
        crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
        set_identity(temp.path());
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        let root_commit = backend
            .commit(temp.path(), "locked member", false)
            .unwrap()
            .commit;
        let request = crate::PushRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".into()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: None,
        };
        let refused = handle_push(
            &backend,
            temp.path(),
            request.clone(),
            "op_missing_dependency",
        )
        .unwrap();
        assert_eq!(read_repo_ref(&root_remote, "refs/heads/main"), None);
        assert_eq!(
            refused.response.members.single().status,
            crate::MemberStatus::Rejected
        );
        // A newer worktree lock pointing to an unrelated published commit cannot
        // stand in for the lock inside the root commit being published.
        git2::Repository::open(&app)
            .unwrap()
            .set_head("refs/heads/unrelated")
            .unwrap();
        let unrelated = commit_file(&app, "README.md", "unrelated", "unrelated", &[]).unwrap();
        backend
            .push(&app, "origin", "refs/heads/unrelated:refs/heads/main")
            .unwrap();
        write_pull_fixture(
            temp.path(),
            vec![(
                "mem_app",
                "repos/app",
                member_remote.to_str().unwrap(),
                &unrelated,
            )],
        );
        let still_refused = handle_push(
            &backend,
            temp.path(),
            request.clone(),
            "op_committed_dependency",
        )
        .unwrap();
        assert_eq!(
            still_refused.response.members.single().status,
            crate::MemberStatus::Rejected
        );
        assert_eq!(read_repo_ref(&root_remote, "refs/heads/main"), None);
        backend
            .push(&app, "origin", &format!("+{member_commit}:refs/heads/main"))
            .unwrap();
        let accepted =
            handle_push(&backend, temp.path(), request, "op_published_dependency").unwrap();
        assert_eq!(
            accepted.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(
            read_repo_ref(&root_remote, "refs/heads/main"),
            Some(root_commit)
        );
    }
}

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
    let observations = response.response.meta.transport.as_ref().unwrap();
    assert_eq!(observations.len(), 2);
    assert!(
        observations
            .iter()
            .all(|row| !row.credential_offered && row.authenticated.is_none())
    );

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
        private: false,
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

#[test]
fn root_push_freezes_source_before_transfer_events() {
    struct MoveHead(std::path::PathBuf);
    impl crate::operation::EventSink for MoveHead {
        fn deliver(&self, event: crate::OperationEvent) {
            if event.kind == crate::EventKind::OperationStarted {
                std::fs::write(self.0.join("late.txt"), "late work").unwrap();
                let backend = Git2Backend::without_credential_helpers();
                backend.stage_paths(&self.0, &["late.txt"]).unwrap();
                backend.commit(&self.0, "late", false).unwrap();
            }
        }
    }
    let temp = TempDir::new("push-frozen-root");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let remote = temp.path().join("root.git");
    init_bare_main(&remote);
    backend
        .add_remote(temp.path(), "origin", remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let captured = backend
        .commit(temp.path(), "captured", false)
        .unwrap()
        .commit;
    let response = handle_push_with_events(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".into()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: None,
        },
        "push",
        &MoveHead(temp.path().to_owned()),
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_ne!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(captured.as_str())
    );
    assert_eq!(read_repo_ref(&remote, "refs/heads/main"), Some(captured));
}

#[test]
fn root_publication_refuses_unimplemented_source_availability_contracts() {
    use crate::artifact::{self, ArtifactSourceKind};
    for kind in [
        ArtifactSourceKind::Archive,
        ArtifactSourceKind::Package,
        ArtifactSourceKind::Local,
        ArtifactSourceKind::Generated,
        ArtifactSourceKind::Git,
    ] {
        let temp = TempDir::new("push-unsupported-dependency");
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
        let remote = temp.path().join("root.git");
        init_bare_main(&remote);
        backend
            .add_remote(temp.path(), "origin", remote.to_str().unwrap())
            .unwrap();
        write_pull_fixture(
            temp.path(),
            vec![("mem_app", "app", remote.to_str().unwrap(), &"0".repeat(40))],
        );
        let mut manifest = artifact::read_manifest(temp.path()).unwrap();
        let mut lock = artifact::read_lock(temp.path()).unwrap();
        manifest.members[0].source_kind = kind;
        let state = lock.members.get_mut("mem_app").unwrap();
        state.source_kind = kind;
        state.commit = None;
        artifact::write_manifest(temp.path(), &manifest).unwrap();
        artifact::write_lock(temp.path(), &lock).unwrap();
        artifact::refresh_conf_integrity_marker(temp.path()).unwrap();
        set_identity(temp.path());
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        backend
            .commit(temp.path(), "unsupported dependency", false)
            .unwrap();
        let response = handle_push(
            &backend,
            temp.path(),
            crate::PushRequest {
                meta: crate::RequestMeta {
                    selection: Some(crate::Selection {
                        targets: vec!["@root".into()],
                        ..Default::default()
                    }),
                    ..request_meta_with_workspace()
                },
                remote: None,
                refspec: None,
            },
            "push",
        )
        .unwrap();
        assert_eq!(read_repo_ref(&remote, "refs/heads/main"), None, "{kind:?}");
        let expected = if kind == ArtifactSourceKind::Git {
            crate::GwzErrorCode::RemoteRejected
        } else {
            crate::GwzErrorCode::UnsupportedSourceKind
        };
        assert_eq!(
            response
                .response
                .members
                .single()
                .error
                .as_ref()
                .unwrap()
                .code,
            expected
        );
    }
}

#[test]
fn root_dependency_identity_is_checked_before_member_publication() {
    for missing_key in [true, false] {
        let temp = TempDir::new("push-dependency-identity");
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
        let root_remote = temp.path().join("root.git");
        init_bare_main(&root_remote);
        backend
            .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
            .unwrap();
        let fixture = RemoteFixture::new("push-dependency-identity-member");
        let initial = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let app = temp.path().join("repos/app");
        backend.clone_repo(fixture.remote_url(), &app).unwrap();
        let local = commit_file(
            &app,
            "README.md",
            "two",
            "local",
            &[git2::Oid::from_str(&initial).unwrap()],
        )
        .unwrap();
        // The member push is local; the committed lock's availability check uses SSH.
        write_pull_fixture(
            temp.path(),
            vec![(
                "mem_app",
                "repos/app",
                "ssh://git@127.0.0.1:1/app.git",
                &local,
            )],
        );
        if missing_key {
            git2::Repository::open(&app)
                .unwrap()
                .config()
                .unwrap()
                .set_str(
                    "remote.origin.gwzSshIdentity",
                    temp.path().join("missing-key").to_str().unwrap(),
                )
                .unwrap();
        }
        set_identity(temp.path());
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        backend.commit(temp.path(), "lock", false).unwrap();
        let response = handle_push(
            &backend,
            temp.path(),
            crate::PushRequest {
                meta: request_meta_with_workspace(),
                remote: None,
                refspec: None,
            },
            "push",
        )
        .unwrap();
        assert_eq!(
            read_repo_ref(&fixture.remote, "refs/heads/main"),
            Some(initial)
        );
        assert_eq!(read_repo_ref(&root_remote, "refs/heads/main"), None);
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Rejected
        );
    }
}

#[test]
fn member_push_freezes_source_and_destination_before_transfer_events() {
    struct MoveSource {
        path: std::path::PathBuf,
        other: std::path::PathBuf,
    }
    impl crate::operation::EventSink for MoveSource {
        fn deliver(&self, event: crate::OperationEvent) {
            if event.kind == crate::EventKind::OperationStarted {
                let backend = Git2Backend::without_credential_helpers();
                std::fs::write(self.path.join("late.txt"), "late").unwrap();
                backend.stage_paths(&self.path, &["late.txt"]).unwrap();
                backend.commit(&self.path, "late", false).unwrap();
                git2::Repository::open(&self.path)
                    .unwrap()
                    .remote_set_pushurl("origin", Some(self.other.to_str().unwrap()))
                    .unwrap();
            }
        }
    }
    let temp = TempDir::new("push-frozen-member");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    let fixture = RemoteFixture::new("push-captured-destination");
    fixture.commit_and_push("README.md", "initial", "initial", &backend);
    let app = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &app).unwrap();
    set_identity(&app);
    std::fs::write(app.join("ready.txt"), "ready").unwrap();
    backend.stage_paths(&app, &["ready.txt"]).unwrap();
    let captured = backend.commit(&app, "captured", false).unwrap().commit;
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &captured)],
    );
    let other = temp.path().join("other.git");
    init_bare_main(&other);
    let response = handle_push_with_events(
        &backend,
        temp.path(),
        push_request(None, None),
        "push",
        &MoveSource {
            path: app.clone(),
            other: other.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        read_repo_ref(&other, "refs/heads/main"),
        None,
        "a callback must not redirect publication"
    );
    assert_eq!(
        read_repo_ref(&fixture.remote, "refs/heads/main"),
        Some(captured.clone())
    );
    assert_ne!(backend.head(&app).unwrap().commit, Some(captured));
}

#[test]
fn root_rejection_preserves_member_publication_and_root_retry_is_cloneable() {
    let temp = TempDir::new("push-root-retry-clone");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
    set_identity(temp.path());
    let root_remote = temp.path().join("root.git");
    init_bare_main(&root_remote);
    backend
        .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
        .unwrap();
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    backend.commit(temp.path(), "base", false).unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let worker = temp.path().join("root-worker");
    backend
        .clone_repo(root_remote.to_str().unwrap(), &worker)
        .unwrap();
    set_identity(&worker);
    std::fs::write(worker.join("other.txt"), "other").unwrap();
    backend.stage_paths(&worker, &["other.txt"]).unwrap();
    let remote_root = backend.commit(&worker, "other", false).unwrap().commit;
    backend
        .push(&worker, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    let fixture = RemoteFixture::new("retry-member");
    fixture.commit_and_push("README.md", "base", "base", &backend);
    let app = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &app).unwrap();
    set_identity(&app);
    std::fs::write(app.join("next.txt"), "next").unwrap();
    backend.stage_paths(&app, &["next.txt"]).unwrap();
    let member_commit = backend.commit(&app, "next", false).unwrap().commit;
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &member_commit)],
    );
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_commit = backend.commit(temp.path(), "lock", false).unwrap().commit;
    let response = handle_push(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: request_meta_with_workspace(),
            remote: None,
            refspec: None,
        },
        "push",
    )
    .unwrap();
    assert_eq!(
        read_repo_ref(&root_remote, "refs/heads/main"),
        Some(remote_root)
    );
    assert_eq!(
        read_repo_ref(&fixture.remote, "refs/heads/main"),
        Some(member_commit.clone())
    );
    assert_eq!(
        response
            .response
            .members
            .iter()
            .find(|row| row.member_id == "mem_app")
            .unwrap()
            .status,
        crate::MemberStatus::Ok
    );
    assert_eq!(
        response
            .response
            .members
            .iter()
            .find(|row| row.member_id == "@root")
            .unwrap()
            .status,
        crate::MemberStatus::Failed
    );
    let retry = handle_push(
        &backend,
        temp.path(),
        crate::PushRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".into()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: Some("+refs/heads/main:refs/heads/main".into()),
        },
        "explicit-force-retry",
    )
    .unwrap();
    assert_eq!(
        retry.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        read_repo_ref(&root_remote, "refs/heads/main"),
        Some(root_commit)
    );
    let fresh = temp.path().join("fresh-clone");
    handle_clone_workspace(
        &backend,
        request_meta(),
        root_remote.to_str().unwrap(),
        fresh.to_str().unwrap(),
        "fresh",
        &crate::operation::NullSink,
    )
    .unwrap();
    assert_eq!(
        backend.head(&fresh.join("repos/app")).unwrap().commit,
        Some(member_commit)
    );
}
