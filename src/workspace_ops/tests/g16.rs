use crate::git::{Git2Backend, GitBackend};

use super::*;

// GWZTag Phase 2: handle_tag — create / list / delete git tags across
// members (+ root), and `materialize --tag` re-meaned to checkout each member's git tag.

fn tag_request(op: crate::TagOp, name: Option<&str>) -> crate::TagRequest {
    crate::TagRequest {
        meta: request_meta(),
        op,
        name: name.map(str::to_owned),
        message: None,
        signed: None,
        remote: None,
        all: None,
    }
}

#[test]
fn create_then_list_then_delete() {
    let temp = TempDir::new("tag-cld");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-cld-source");
    let member_root = temp.path().join("remote");

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    assert!(
        backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()),
        "git tag created in the member"
    );

    let listed = handle_tag(&backend, temp.path(), tag_request(crate::TagOp::List, None), "op").unwrap();
    let tags = listed.tags.expect("list populates tags");
    assert!(
        tags.iter().any(|tag| tag.name == "v1" && tag.members >= 1),
        "v1 listed (a real git ref)"
    );

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Delete, Some("v1")), "op").unwrap();
    assert!(
        !backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()),
        "tag gone after delete"
    );
}

#[test]
fn create_requires_a_name() {
    let temp = TempDir::new("tag-noname");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-noname-source");
    let err = handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, None), "op").unwrap_err();
    assert_eq!(err.code, crate::model::ErrorCode::InvalidRequest);
}

#[test]
fn materialize_tag_restores_the_tagged_commit() {
    let temp = TempDir::new("tag-mat");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-mat-source");
    let member_root = temp.path().join("remote");
    set_identity(&member_root);

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    let tagged = backend.head(&member_root).unwrap().commit.unwrap();

    // Advance the member past the tag.
    std::fs::write(member_root.join("more.txt"), "z\n").unwrap();
    backend.stage_paths(&member_root, &["more.txt"]).unwrap();
    backend.commit(&member_root, "advance", false).unwrap();
    assert_ne!(backend.head(&member_root).unwrap().commit.unwrap(), tagged);

    // `materialize --tag v1` checks the member back out to the tagged commit.
    let events = CollectingSink::default();
    handle_materialize(
        &backend,
        temp.path(),
        materialize_named_request(crate::MaterializeTargetKind::Tag, "v1"),
        "op_mat",
        &events,
    )
    .unwrap();
    assert_eq!(
        backend.head(&member_root).unwrap().commit.unwrap(),
        tagged,
        "member restored to the tagged commit"
    );
}

fn init_two_member_workspace(
    temp: &std::path::Path,
    backend: &Git2Backend,
) -> (RemoteFixture, RemoteFixture) {
    let fa = RemoteFixture::new("two-app");
    fa.commit_and_push("README.md", "a", "init a", backend);
    let fb = RemoteFixture::new("two-lib");
    fb.commit_and_push("README.md", "b", "init b", backend);
    let events = CollectingSink::default();
    handle_init_from_sources(
        backend,
        temp,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: temp.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: fa.remote_url().to_owned(),
                    path: Some("app".to_owned()),
                    remote_name: None,
                    branch: None,
                },
                crate::SourceUrl {
                    url: fb.remote_url().to_owned(),
                    path: Some("lib".to_owned()),
                    remote_name: None,
                    branch: None,
                },
            ],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &events,
    )
    .unwrap();
    (fa, fb)
}

#[test]
fn materialize_tag_skips_untagged_members() {
    let temp = TempDir::new("tag-mat-subset");
    let backend = Git2Backend::new();
    let (_fa, _fb) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    set_identity(&app);

    // Tag both members, then drop the tag from `lib` so only `app` carries it.
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    backend.tag_delete(&lib, "v1").unwrap();
    let tagged = backend.head(&app).unwrap().commit.unwrap();

    // Advance `app` past the tag; `lib` must stay put.
    std::fs::write(app.join("more.txt"), "z\n").unwrap();
    backend.stage_paths(&app, &["more.txt"]).unwrap();
    backend.commit(&app, "advance", false).unwrap();
    let lib_head = backend.head(&lib).unwrap().commit.unwrap();

    // `materialize --tag v1` (default selection) restores the tagged member and skips the rest,
    // rather than erroring LockNotFound on the untagged member.
    let events = CollectingSink::default();
    handle_materialize(
        &backend,
        temp.path(),
        materialize_named_request(crate::MaterializeTargetKind::Tag, "v1"),
        "op_mat",
        &events,
    )
    .unwrap();
    assert_eq!(backend.head(&app).unwrap().commit.unwrap(), tagged, "tagged member restored");
    assert_eq!(backend.head(&lib).unwrap().commit.unwrap(), lib_head, "untagged member untouched");
}

#[test]
fn create_is_idempotent() {
    let temp = TempDir::new("tag-idem");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-idem-source");
    let member_root = temp.path().join("remote");
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    // A second create succeeds (skips members already carrying the tag) instead of erroring.
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    assert!(backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()));
}

#[test]
fn signed_without_message_is_rejected() {
    let temp = TempDir::new("tag-signed");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-signed-source");
    let request = crate::TagRequest {
        meta: request_meta(),
        op: crate::TagOp::Create,
        name: Some("v1".to_owned()),
        message: None,
        signed: Some(true),
        remote: None,
        all: None,
    };
    assert_eq!(
        handle_tag(&backend, temp.path(), request, "op").unwrap_err().code,
        crate::model::ErrorCode::InvalidRequest
    );
}

#[test]
fn create_with_message_stores_the_annotation() {
    let temp = TempDir::new("tag-annot-ws");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-annot-source");
    let member_root = temp.path().join("remote");
    let request = crate::TagRequest {
        meta: request_meta(),
        op: crate::TagOp::Create,
        name: Some("rel".to_owned()),
        message: Some("release one".to_owned()),
        signed: None,
        remote: None,
        all: None,
    };
    handle_tag(&backend, temp.path(), request, "op").unwrap();
    let repo = git2::Repository::open(&member_root).unwrap();
    let object = repo.revparse_single("refs/tags/rel").unwrap();
    assert_eq!(
        object.as_tag().unwrap().message().unwrap().map(str::trim),
        Some("release one")
    );
}

#[test]
fn tags_span_the_committed_workspace_root() {
    let temp = TempDir::new("tag-root");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-root-source");
    // Give the workspace root a commit (otherwise create skips its unborn HEAD).
    set_identity(temp.path());
    std::fs::write(temp.path().join("root.txt"), "r\n").unwrap();
    backend.stage_paths(temp.path(), &["root.txt"]).unwrap();
    backend.commit(temp.path(), "root", false).unwrap();

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1")), "op").unwrap();
    assert!(
        backend.tag_list(temp.path()).unwrap().contains(&"v1".to_owned()),
        "the workspace root carries the tag"
    );
    let listed = handle_tag(&backend, temp.path(), tag_request(crate::TagOp::List, None), "op").unwrap();
    let v1 = listed.tags.unwrap().into_iter().find(|t| t.name == "v1").expect("v1 listed");
    assert_eq!(v1.members, 2, "member + root both carry the tag");
}
