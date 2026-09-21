//! Aggregate command-driver coverage over one host endpoint.

use super::driver_tests::{CliHarness, block_on};
use crate::RequestMeta;
use crate::operation::NullSink;
use crate::workspace_ops::*;
use std::path::Path;

fn with_scope<T>(
    harness: &CliHarness,
    meta: RequestMeta,
    operation: &str,
    f: impl FnOnce(&crate::git::Git2Backend) -> crate::model::ModelResult<T>,
) -> T {
    let client = harness.endpoint.register_request(&meta.request_id).unwrap();
    let request = block_on(harness.runtime.request(meta, operation.to_owned())).unwrap();
    let result = f(request.backend()).unwrap();
    let _ = block_on(request.finish());
    let _ = block_on(client.finish());
    result
}

fn commit(repo: &git2::Repository, text: &str) -> git2::Oid {
    let blob = repo.blob(text.as_bytes()).unwrap();
    let mut tree = repo.treebuilder(None).unwrap();
    tree.insert("payload", blob, 0o100644).unwrap();
    let tree = repo.find_tree(tree.write().unwrap()).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
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

fn checkpoint(path: &Path) {
    let repo = git2::Repository::open(path).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
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

fn url(harness: &CliHarness) -> String {
    format!(
        "ssh://{}@127.0.0.1:{}{}",
        harness.fixture.user,
        harness.fixture.port,
        harness.fixture.repository.to_string_lossy()
    )
}

#[test]
fn all_workspace_command_funnels_keep_one_endpoint_and_fresh_scopes() {
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    let first = commit(&server, "first");
    let root = harness.fixture.temp.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let source = crate::SourceUrl {
        url: url(&harness),
        path: Some("member".into()),
        ..Default::default()
    };

    let init_meta = harness.meta("commands-init");
    with_scope(&harness, init_meta.clone(), "init", |backend| {
        handle_init_from_sources(
            backend,
            &root,
            crate::InitFromSourcesRequest {
                meta: init_meta,
                workspace_root: root.to_string_lossy().into_owned(),
                sources: vec![source.clone()],
                ..Default::default()
            },
            "init",
            &NullSink,
        )
        .map(|outcome| {
            assert!(
                matches!(
                    outcome.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{outcome:?}"
            );
        })
    });
    let member = root.join("member");
    assert!(
        git2::Repository::open(&member)
            .unwrap()
            .find_commit(first)
            .is_ok()
    );

    checkpoint(&root);
    let push_meta = member_meta(&harness, "commands-push");
    with_scope(&harness, push_meta.clone(), "push", |backend| {
        handle_push(
            backend,
            &root,
            crate::PushRequest {
                meta: push_meta,
                remote: Some("origin".into()),
                refspec: Some("refs/heads/main:refs/heads/published".into()),
                ..Default::default()
            },
            "push",
        )
        .map(|outcome| {
            assert!(
                matches!(
                    outcome.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{outcome:?}"
            );
        })
    });
    assert_eq!(
        server
            .find_reference("refs/heads/published")
            .unwrap()
            .target(),
        Some(first)
    );

    let mut snapshot_meta = member_meta(&harness, "commands-snapshot");
    snapshot_meta.transport = None;
    with_scope(&harness, snapshot_meta.clone(), "snapshot", |backend| {
        handle_snapshot(
            backend,
            &root,
            crate::SnapshotRequest {
                meta: snapshot_meta,
                snapshot_id: "saved".into(),
                ..Default::default()
            },
            "snapshot",
        )
        .map(|outcome| {
            assert!(
                matches!(
                    outcome.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{outcome:?}"
            );
        })
    });
    std::fs::remove_dir_all(&member).unwrap();
    let pull_snapshot_meta = member_meta(&harness, "commands-pull-snapshot");
    with_scope(
        &harness,
        pull_snapshot_meta.clone(),
        "pull-snapshot",
        |backend| {
            handle_pull_snapshot(
                backend,
                &root,
                crate::PullSnapshotRequest {
                    meta: pull_snapshot_meta,
                    snapshot_id: "saved".into(),
                },
                "pull-snapshot",
                &NullSink,
            )
            .map(|outcome| {
                assert!(
                    matches!(
                        outcome.response.meta.aggregate_status,
                        crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                    ),
                    "{outcome:?}"
                );
            })
        },
    );
    assert!(member.exists());

    let next = commit(&server, "next");
    let mut pull_meta = member_meta(&harness, "commands-pull-head");
    pull_meta.policy = Some(crate::OperationPolicy {
        sync: Some(crate::SyncBehavior::FfOnly),
        ..Default::default()
    });
    with_scope(&harness, pull_meta.clone(), "pull-head", |backend| {
        handle_pull_head(
            backend,
            &root,
            crate::PullHeadRequest {
                meta: pull_meta,
                ..Default::default()
            },
            "pull-head",
        )
        .map(|outcome| {
            assert!(
                matches!(
                    outcome.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{outcome:?}"
            );
        })
    });
    assert_eq!(
        git2::Repository::open(&member)
            .unwrap()
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        Some(next)
    );

    checkpoint(&root);
    let copy = harness.fixture.temp.path().join("workspace-copy");
    let clone_meta = harness.meta("commands-workspace-clone");
    with_scope(&harness, clone_meta.clone(), "workspace-clone", |backend| {
        handle_clone_workspace_request(
            backend,
            harness.fixture.temp.path(),
            crate::CloneWorkspaceRequest {
                meta: clone_meta,
                url: format!(
                    "ssh://{}@127.0.0.1:{}{}",
                    harness.fixture.user,
                    harness.fixture.port,
                    root.display()
                ),
                target: copy.to_string_lossy().into_owned(),
            },
            "workspace-clone",
            &NullSink,
        )
        .map(|outcome| {
            assert!(
                matches!(
                    outcome.response.meta.aggregate_status,
                    crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
                ),
                "{outcome:?}"
            );
        })
    });
    assert!(copy.join("member").exists());
}

fn member_meta(harness: &CliHarness, id: &str) -> RequestMeta {
    let mut meta = harness.meta(id);
    meta.selection = Some(crate::Selection {
        targets: vec!["@all".into()],
        exclude_targets: vec!["@root".into()],
        ..Default::default()
    });
    meta
}

#[test]
fn cli_repository_refusal_is_quiet_only_for_private_members() {
    use std::os::unix::fs::PermissionsExt;
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    commit(&server, "private fixture");
    let script = harness.fixture.temp.path().join("service.sh");
    std::fs::write(&script, "#!/bin/sh\ncase \"$SSH_ORIGINAL_COMMAND\" in\n *inaccessible.git*) echo 'ERROR: Repository not found.' >&2; exit 1 ;;\n *) eval \"$SSH_ORIGINAL_COMMAND\" ;;\nesac\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let public =
        std::fs::read_to_string(harness.fixture.temp.path().join("client_ed25519.pub")).unwrap();
    std::fs::write(
        harness.fixture.temp.path().join("authorized_keys"),
        format!("command=\"{}\" {}", script.display(), public),
    )
    .unwrap();
    let denied = format!(
        "ssh://{}@127.0.0.1:{}/inaccessible.git",
        harness.fixture.user, harness.fixture.port
    );
    let root = harness.fixture.temp.path().join("private-workspace");
    std::fs::create_dir(&root).unwrap();
    let meta = harness.meta("private-init");
    with_scope(&harness, meta.clone(), "init", |backend| {
        handle_init_from_sources(
            backend,
            &root,
            crate::InitFromSourcesRequest {
                meta,
                workspace_root: root.to_string_lossy().into(),
                sources: vec![crate::SourceUrl {
                    url: url(&harness),
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
    let mut meta = member_meta(&harness, "private-sync");
    meta.transport = None;
    with_scope(&harness, meta.clone(), "private", |backend| {
        handle_repo_sync(
            backend,
            &root,
            crate::RepoSyncRequest {
                meta,
                private: Some(true),
            },
            "private",
        )
    });
    checkpoint(&root);
    std::fs::remove_dir_all(root.join("secret")).unwrap();
    let meta = member_meta(&harness, "private-materialize");
    let result = with_scope(&harness, meta.clone(), "materialize", |backend| {
        handle_materialize(
            backend,
            &root,
            crate::MaterializeRequest {
                meta,
                target: crate::MaterializeTarget {
                    kind: crate::MaterializeTargetKind::Lock,
                    ..Default::default()
                },
            },
            "materialize",
            &NullSink,
        )
    });
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
        "private refusal must remain quiet: {result:?}"
    );
}
