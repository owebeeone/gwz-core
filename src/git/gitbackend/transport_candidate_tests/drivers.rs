use super::*;
use crate::{operation::NullSink, workspace_ops::*};

fn meta(f: &common::SshdFixture, members: bool) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: "candidate".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(crate::TransportOptions {
            default_identity: Some(
                f.temp
                    .path()
                    .join("client_ed25519")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        }),
        selection: members.then(|| crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        }),
        ..Default::default()
    }
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
    let (f, b, e) = fixture();
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
    let init = handle_init_from_sources(
        &b,
        &root,
        crate::InitFromSourcesRequest {
            meta: meta(&f, false),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![source.clone()],
            ..Default::default()
        },
        "init",
        &NullSink,
    )
    .unwrap();
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
    let fetch = handle_fetch(
        &b,
        &root,
        crate::FetchRequest {
            meta: meta(&f, true),
        },
        "fetch",
    )
    .unwrap();
    assert_reused(&fetch.response);
    let tag = handle_tag(
        &b,
        &root,
        crate::TagRequest {
            meta: meta(&f, true),
            op: crate::TagOp::Fetch,
            remote: Some("origin".into()),
            ..Default::default()
        },
        "tags",
    )
    .unwrap();
    assert_reused(&tag.response);
    let push = handle_push(
        &b,
        &root,
        crate::PushRequest {
            meta: meta(&f, true),
            remote: Some("origin".into()),
            refspec: Some("refs/heads/main:refs/heads/published".into()),
            ..Default::default()
        },
        "push",
    )
    .unwrap();
    assert_reused(&push.response);
    handle_snapshot(
        &b,
        &root,
        crate::SnapshotRequest {
            meta: crate::RequestMeta {
                transport: None,
                ..meta(&f, true)
            },
            snapshot_id: "saved".into(),
            ..Default::default()
        },
        "snapshot",
    )
    .unwrap();
    // A missing member forces actual clone/fetch inside the nested materializer.
    std::fs::remove_dir_all(root.join("member")).unwrap();
    let nested = handle_pull_snapshot(
        &b,
        &root,
        crate::PullSnapshotRequest {
            meta: meta(&f, true),
            snapshot_id: "saved".into(),
        },
        "nested",
        &NullSink,
    )
    .unwrap();
    assert_reused(&nested.response);
    assert_eq!(
        nested.response.meta.transport.as_ref().unwrap().len(),
        1,
        "inner rows must appear exactly once"
    );
    let next = commit(&server, "next");
    let pull = handle_pull_head(
        &b,
        &root,
        crate::PullHeadRequest {
            meta: crate::RequestMeta {
                policy: Some(crate::OperationPolicy {
                    sync: Some(crate::SyncBehavior::FfOnly),
                    ..Default::default()
                }),
                ..meta(&f, true)
            },
        },
        "pull",
    )
    .unwrap();
    assert_reused(&pull.response);
    assert_eq!(
        b.head(&root.join("member")).unwrap().commit,
        Some(next.to_string())
    );
    let mut second = source;
    second.path = Some("second".into());
    let cloned = handle_clone_repo_member(
        &b,
        &root,
        crate::CloneRepoMemberRequest {
            meta: meta(&f, false),
            source: second,
            ..Default::default()
        },
        "member-clone",
        &NullSink,
    )
    .unwrap();
    assert_reused(&cloned.response);
    checkpoint(&root);
    let workspace_url = format!("ssh://{}@127.0.0.1:{}{}", f.user, f.port, root.display());
    let copy = f.temp.path().join("workspace-copy");
    let cloned = handle_clone_workspace_request(
        &b,
        f.temp.path(),
        crate::CloneWorkspaceRequest {
            meta: meta(&f, true),
            url: workspace_url,
            target: copy.to_string_lossy().into_owned(),
        },
        "workspace-clone",
        &NullSink,
    )
    .unwrap();
    assert_reused(&cloned.response);
    assert!(copy.join("member/payload").exists());
    assert!(copy.join("second/payload").exists());
    assert!(
        b.transport_observations().unwrap().snapshot().is_empty(),
        "each driver scopes its own rows"
    );
    e.shutdown();
}
