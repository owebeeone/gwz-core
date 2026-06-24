use crate::git::{Git2Backend, GitBackend};

use super::*;

// GWZTag Phase 3: handle_tag remote ops — push, list --remote, delete --remote, fetch — fanned
// out over the members (whose origin is the bare RemoteFixture); the root is local-only.

fn tag_request(op: crate::TagOp, name: Option<&str>, remote: Option<&str>) -> crate::TagRequest {
    crate::TagRequest {
        meta: request_meta(),
        op,
        name: name.map(str::to_owned),
        message: None,
        signed: None,
        remote: remote.map(str::to_owned),
        all: None,
    }
}

#[test]
fn push_then_list_remote_then_delete_remote() {
    let temp = TempDir::new("tag-remote");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-remote-source");
    let member_root = temp.path().join("remote");

    // Tag locally, then push to the member's origin.
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1"), None), "op").unwrap();
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Push, Some("v1"), Some("origin")), "op").unwrap();
    assert!(
        backend
            .ls_remote(&member_root, "origin")
            .unwrap()
            .iter()
            .any(|r| r.name == "refs/tags/v1"),
        "tag pushed to the remote"
    );

    // list --remote sees it with the prefix stripped.
    let listed =
        handle_tag(&backend, temp.path(), tag_request(crate::TagOp::List, None, Some("origin")), "op").unwrap();
    assert!(
        listed.tags.unwrap().iter().any(|t| t.name == "v1"),
        "v1 listed from the remote"
    );

    // delete --remote removes it from the remote but keeps the local copy.
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Delete, Some("v1"), Some("origin")), "op").unwrap();
    assert!(
        !backend
            .ls_remote(&member_root, "origin")
            .unwrap()
            .iter()
            .any(|r| r.name == "refs/tags/v1"),
        "tag removed from the remote"
    );
    assert!(
        backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()),
        "local tag retained after a remote delete"
    );
}

#[test]
fn fetch_restores_a_tag_from_the_remote() {
    let temp = TempDir::new("tag-fetch-ws");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-fetch-ws-source");
    let member_root = temp.path().join("remote");

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1"), None), "op").unwrap();
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Push, Some("v1"), Some("origin")), "op").unwrap();

    // Drop the local copy, then fetch it back from the remote.
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Delete, Some("v1"), None), "op").unwrap();
    assert!(!backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()));

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Fetch, None, Some("origin")), "op").unwrap();
    assert!(
        backend.tag_list(&member_root).unwrap().contains(&"v1".to_owned()),
        "fetch restored the tag from the remote"
    );
}

#[test]
fn push_with_no_name_pushes_every_tag() {
    let temp = TempDir::new("tag-push-all");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-push-all-source");
    let member_root = temp.path().join("remote");

    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v1"), None), "op").unwrap();
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Create, Some("v2"), None), "op").unwrap();

    // Push with NO name → every tag lands on the remote (libgit2 can't expand a glob,
    // so the handler must enumerate concrete refspecs).
    handle_tag(&backend, temp.path(), tag_request(crate::TagOp::Push, None, Some("origin")), "op").unwrap();

    let remote_tags: Vec<String> = backend
        .ls_remote(&member_root, "origin")
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert!(remote_tags.contains(&"refs/tags/v1".to_owned()), "v1 pushed");
    assert!(remote_tags.contains(&"refs/tags/v2".to_owned()), "v2 pushed");
}
