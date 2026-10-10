use super::*;
use crate::{operation::NullSink, workspace_ops::*};

fn meta(f: &common::SshdFixture, members: bool, request_id: &str) -> crate::RequestMeta {
    crate::RequestMeta {
        selection: members.then(|| crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        }),
        ..request_meta(f, request_id)
    }
}
/// Runs one driver as the CLI runs a network command: in a request of its
/// own, whose host context admits exactly this metadata and operation.
fn scoped<T>(
    host: &Host,
    meta: crate::RequestMeta,
    operation: &str,
    driver: impl FnOnce(&Git2Backend, crate::RequestMeta) -> crate::model::ModelResult<T>,
) -> T {
    let request = host.request(meta.clone(), operation);
    let result = driver(request.backend(), meta).unwrap();
    assert!(
        request
            .backend()
            .transport_observations()
            .unwrap()
            .snapshot()
            .is_empty(),
        "each driver scopes its own rows"
    );
    host.finish(request);
    result
}
fn assert_reused(response: &crate::ResponseEnvelope) {
    assert!(
        matches!(
            response.meta.aggregate_status,
            crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
        ),
        "{response:?}"
    );
    let rows = response
        .meta
        .transport
        .as_ref()
        .expect("transport observations");
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|r| r.authenticated == Some(true) && !r.credential_offered),
        "{rows:?}"
    );
}
fn checkpoint(root: &Path) {
    let repo = git2::Repository::open(root).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "workspace",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap();
}
#[test]
fn candidate_command_drivers_share_pool_and_preserve_nested_observations() {
    let f = common::SshdFixture::new();
    let host = Host::new(&f);
    let server = git2::Repository::open_bare(&f.repository).unwrap();
    commit(&server, "first");
    let root = f.temp.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let source = crate::SourceUrl {
        url: url(&f),
        path: Some("member".into()),
        remote_name: None,
        branch: None,
    };
    let init = scoped(&host, meta(&f, false, "init"), "init", |b, meta| {
        handle_init_from_sources(
            b,
            &root,
            crate::InitFromSourcesRequest {
                meta,
                workspace_root: root.to_string_lossy().into_owned(),
                sources: vec![source.clone()],
                ..Default::default()
            },
            "init",
            &NullSink,
        )
    });
    assert_eq!(
        init.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        init.response
            .meta
            .transport
            .as_ref()
            .unwrap()
            .iter()
            .filter(|r| r.credential_offered)
            .count(),
        1
    );
    checkpoint(&root);
    let fetch = scoped(&host, meta(&f, true, "fetch"), "fetch", |b, meta| {
        handle_fetch(b, &root, crate::FetchRequest { meta }, "fetch")
    });
    assert_reused(&fetch.response);
    let tag = scoped(&host, meta(&f, true, "tags"), "tags", |b, meta| {
        handle_tag(
            b,
            &root,
            crate::TagRequest {
                meta,
                op: crate::TagOp::Fetch,
                remote: Some("origin".into()),
                ..Default::default()
            },
            "tags",
        )
    });
    assert_reused(&tag.response);
    let push = scoped(&host, meta(&f, true, "push"), "push", |b, meta| {
        handle_push(
            b,
            &root,
            crate::PushRequest {
                meta,
                remote: Some("origin".into()),
                refspec: Some("refs/heads/main:refs/heads/published".into()),
                ..Default::default()
            },
            "push",
        )
    });
    assert_reused(&push.response);
    // A snapshot opens no connection, so it runs as the CLI runs it: on a
    // backend without a host context.
    handle_snapshot(
        &Git2Backend::new(),
        &root,
        crate::SnapshotRequest {
            meta: crate::RequestMeta {
                transport: None,
                ..meta(&f, true, "snapshot")
            },
            snapshot_id: "saved".into(),
            ..Default::default()
        },
        "snapshot",
    )
    .unwrap();
    // A missing member forces actual clone/fetch inside the nested materializer.
    std::fs::remove_dir_all(root.join("member")).unwrap();
    let nested = scoped(&host, meta(&f, true, "nested"), "nested", |b, meta| {
        handle_pull_snapshot(
            b,
            &root,
            crate::PullSnapshotRequest {
                meta,
                snapshot_id: "saved".into(),
            },
            "nested",
            &NullSink,
        )
    });
    assert_reused(&nested.response);
    assert_eq!(
        nested.response.meta.transport.as_ref().unwrap().len(),
        1,
        "inner rows must appear exactly once"
    );
    let next = commit(&server, "next");
    let mut pull_meta = meta(&f, true, "pull");
    if let Some(policy) = pull_meta.policy.as_mut() {
        policy.sync = Some(crate::SyncBehavior::FfOnly);
    }
    let pull = scoped(&host, pull_meta, "pull", |b, meta| {
        handle_pull_head(b, &root, crate::PullHeadRequest { meta }, "pull")
    });
    assert_reused(&pull.response);
    assert_eq!(
        Git2Backend::new()
            .head(&root.join("member"))
            .unwrap()
            .commit,
        Some(next.to_string())
    );
    let mut second = source;
    second.path = Some("second".into());
    let cloned = scoped(
        &host,
        meta(&f, false, "member-clone"),
        "member-clone",
        |b, meta| {
            handle_clone_repo_member(
                b,
                &root,
                crate::CloneRepoMemberRequest {
                    meta,
                    source: second,
                    ..Default::default()
                },
                "member-clone",
                &NullSink,
            )
        },
    );
    assert_reused(&cloned.response);
    checkpoint(&root);
    let workspace_url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        f.user,
        f.port,
        common::server_path(&root)
    );
    let copy = f.temp.path().join("workspace-copy");
    let cloned = scoped(
        &host,
        meta(&f, true, "workspace-clone"),
        "workspace-clone",
        |b, meta| {
            handle_clone_workspace_request(
                b,
                f.temp.path(),
                crate::CloneWorkspaceRequest {
                    meta,
                    url: workspace_url,
                    target: copy.to_string_lossy().into_owned(),
                },
                "workspace-clone",
                &NullSink,
            )
        },
    );
    assert_reused(&cloned.response);
    assert!(copy.join("member/payload").exists());
    assert!(copy.join("second/payload").exists());
    host.shutdown();
}

// The refusal script is a POSIX script forced in `authorized_keys` and made executable with a mode, which waits for
// step 4.3's Windows fake-helper form and step 4.11.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn candidate_service_refusal_skips_only_private_members_and_forgets_observations() {
            use std::os::unix::fs::PermissionsExt;
            let (f, host, request, b) = fixture();
            let server = git2::Repository::open_bare(&f.repository).unwrap();
            commit(&server, "fixture");
            let script = f.temp.path().join("service.sh");
            std::fs::write(&script, "#!/bin/sh\ncase \"$SSH_ORIGINAL_COMMAND\" in\n *inaccessible.git*) echo 'ERROR: Repository not found.' >&2; exit 1 ;;\n *broken.git*) echo 'fixture backend failed' >&2; exit 2 ;;\n *) eval \"$SSH_ORIGINAL_COMMAND\" ;;\nesac\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let public = std::fs::read_to_string(f.temp.path().join("client_ed25519.pub")).unwrap();
            std::fs::write(
                f.temp.path().join("authorized_keys"),
                format!("command=\"{}\" {}", script.display(), public),
            )
            .unwrap();
            let denied = format!("ssh://{}@127.0.0.1:{}/inaccessible.git", f.user, f.port);
            let broken = format!("ssh://{}@127.0.0.1:{}/broken.git", f.user, f.port);
            // The whole error goes in the failure message: a CI failure here must say
            // whether the refusal was lost or the clone failed another way.
            let refused = b
                .clone_repo(&denied, &f.temp.path().join("refused"))
                .unwrap_err();
            assert_eq!(
                refused.code,
                crate::model::ErrorCode::RemoteRejected,
                "{refused:?}"
            );
            let failed = b
                .clone_repo(&broken, &f.temp.path().join("broken"))
                .unwrap_err();
            assert_eq!(
                failed.code,
                crate::model::ErrorCode::GitCommandFailed,
                "{failed:?}"
            );
            host.finish(request);
            let root = f.temp.path().join("private-workspace");
            std::fs::create_dir(&root).unwrap();
            scoped(&host, meta(&f, false, "init"), "init", |b, meta| {
                handle_init_from_sources(
                    b,
                    &root,
                    crate::InitFromSourcesRequest {
                        meta,
                        workspace_root: root.to_string_lossy().into(),
                        sources: vec![crate::SourceUrl {
                            url: url(&f),
                            path: Some("secret".into()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    "init",
                    &NullSink,
                )
            });
            git2::Repository::open(root.join("secret"))
                .unwrap()
                .remote_set_url("origin", &denied)
                .unwrap();
            // `repo sync` opens no connection, so it runs as the CLI runs it: on a
            // backend without a host context.
            handle_repo_sync(
                &Git2Backend::new(),
                &root,
                crate::RepoSyncRequest {
                    meta: crate::RequestMeta {
                        transport: None,
                        ..meta(&f, true, "private")
                    },
                    private: Some(true),
                },
                "private",
            )
            .unwrap();
            checkpoint(&root);
            std::fs::remove_dir_all(root.join("secret")).unwrap();
            let result = scoped(
                &host,
                meta(&f, true, "private-materialize"),
                "private-materialize",
                |b, meta| {
                    handle_materialize(
                        b,
                        &root,
                        crate::MaterializeRequest {
                            meta,
                            target: crate::MaterializeTarget {
                                kind: crate::MaterializeTargetKind::Lock,
                                ..Default::default()
                            },
                        },
                        "private-materialize",
                        &NullSink,
                    )
                },
            );
            assert!(
                matches!(
                    result.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{result:?}"
            );
            assert!(!root.join("secret").exists());
            assert!(
                result
                    .response
                    .meta
                    .transport
                    .as_ref()
                    .is_none_or(Vec::is_empty),
                "{result:?}"
            );
            assert!(
                result.response.members.is_empty(),
                "private refusal must be quiet: {result:?}"
            );
            host.shutdown();
        }
    }
}
