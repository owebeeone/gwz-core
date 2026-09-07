use crate::git::{Git2Backend, GitBackend};

use super::*;

#[test]
fn tag_publication_plan_pins_objects_before_any_transfer() {
    let temp = TempDir::new("tag-frozen-source");
    let backend = Git2Backend::without_credential_helpers();
    backend.create_repo(temp.path()).unwrap();
    set_identity(temp.path());
    commit_file(temp.path(), "first.txt", "first", "first", &[]).unwrap();
    backend
        .tag_create(temp.path(), "v1", Some("original annotation"), false)
        .unwrap();
    let original = backend
        .read_ref(temp.path(), "refs/tags/v1")
        .unwrap()
        .unwrap();
    let plans = super::super::handle_tag::plan_tag_pushes(
        &backend,
        &[temp.path().to_path_buf()],
        Some("v1"),
    )
    .unwrap();
    backend.tag_delete(temp.path(), "v1").unwrap();
    let parent = backend.head(temp.path()).unwrap().commit.unwrap();
    commit_file(
        temp.path(),
        "second.txt",
        "second",
        "second",
        &[git2::Oid::from_str(&parent).unwrap()],
    )
    .unwrap();
    backend
        .tag_create(temp.path(), "v1", Some("replacement annotation"), false)
        .unwrap();
    assert_ne!(
        backend.read_ref(temp.path(), "refs/tags/v1").unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        plans,
        vec![(
            temp.path().to_path_buf(),
            format!("{original}:refs/tags/v1")
        )]
    );
}

#[test]
fn root_tag_create_list_push_and_delete_use_root_selection() {
    let temp = TempDir::new("root-tag-recovery");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let remote = temp.path().join("root.git");
    init_bare_main(&remote);
    backend
        .add_remote(temp.path(), "origin", remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    commit_file(temp.path(), "root.txt", "root", "root", &[]).unwrap();
    let mut request = tag_request(crate::TagOp::Create, Some("v-root"), None);
    request.message = Some("annotated root tag".into());
    request.meta.selection = Some(crate::Selection {
        targets: vec!["@root".into()],
        ..Default::default()
    });
    handle_tag(&backend, temp.path(), request.clone(), "op_tag_root").unwrap();
    assert!(
        backend
            .tag_list(temp.path())
            .unwrap()
            .contains(&"v-root".into())
    );
    request.op = crate::TagOp::List;
    handle_tag(&backend, temp.path(), request.clone(), "op_list_root").unwrap();
    request.op = crate::TagOp::Push;
    handle_tag(&backend, temp.path(), request.clone(), "op_push_root_tag").unwrap();
    assert!(read_repo_ref(&remote, "refs/tags/v-root").is_some());
    request.op = crate::TagOp::Delete;
    request.remote = Some("origin".into());
    handle_tag(
        &backend,
        temp.path(),
        request.clone(),
        "op_delete_remote_root_tag",
    )
    .unwrap();
    assert!(read_repo_ref(&remote, "refs/tags/v-root").is_none());
    request.remote = None;
    handle_tag(&backend, temp.path(), request, "op_delete_root_tag").unwrap();
    assert!(
        !backend
            .tag_list(temp.path())
            .unwrap()
            .contains(&"v-root".into())
    );
}

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
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Create, Some("v1"), None),
        "op",
    )
    .unwrap();
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Push, Some("v1"), Some("origin")),
        "op",
    )
    .unwrap();
    assert!(
        backend
            .ls_remote(&member_root, "origin")
            .unwrap()
            .iter()
            .any(|r| r.name == "refs/tags/v1"),
        "tag pushed to the remote"
    );

    // list --remote sees it with the prefix stripped.
    let listed = handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::List, None, Some("origin")),
        "op",
    )
    .unwrap();
    assert!(
        listed.tags.unwrap().iter().any(|t| t.name == "v1"),
        "v1 listed from the remote"
    );

    // delete --remote removes it from the remote but keeps the local copy.
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Delete, Some("v1"), Some("origin")),
        "op",
    )
    .unwrap();
    assert!(
        !backend
            .ls_remote(&member_root, "origin")
            .unwrap()
            .iter()
            .any(|r| r.name == "refs/tags/v1"),
        "tag removed from the remote"
    );
    assert!(
        backend
            .tag_list(&member_root)
            .unwrap()
            .contains(&"v1".to_owned()),
        "local tag retained after a remote delete"
    );
}

#[test]
fn fetch_restores_a_tag_from_the_remote() {
    let temp = TempDir::new("tag-fetch-ws");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-fetch-ws-source");
    let member_root = temp.path().join("remote");

    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Create, Some("v1"), None),
        "op",
    )
    .unwrap();
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Push, Some("v1"), Some("origin")),
        "op",
    )
    .unwrap();

    // Drop the local copy, then fetch it back from the remote.
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Delete, Some("v1"), None),
        "op",
    )
    .unwrap();
    assert!(
        !backend
            .tag_list(&member_root)
            .unwrap()
            .contains(&"v1".to_owned())
    );

    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Fetch, None, Some("origin")),
        "op",
    )
    .unwrap();
    assert!(
        backend
            .tag_list(&member_root)
            .unwrap()
            .contains(&"v1".to_owned()),
        "fetch restored the tag from the remote"
    );
}

#[test]
fn push_with_no_name_pushes_every_tag() {
    let temp = TempDir::new("tag-push-all");
    let backend = Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "tag-push-all-source");
    let member_root = temp.path().join("remote");

    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Create, Some("v1"), None),
        "op",
    )
    .unwrap();
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Create, Some("v2"), None),
        "op",
    )
    .unwrap();

    // Push with NO name → every tag lands on the remote (libgit2 can't expand a glob,
    // so the handler must enumerate concrete refspecs).
    handle_tag(
        &backend,
        temp.path(),
        tag_request(crate::TagOp::Push, None, Some("origin")),
        "op",
    )
    .unwrap();

    let remote_tags: Vec<String> = backend
        .ls_remote(&member_root, "origin")
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert!(
        remote_tags.contains(&"refs/tags/v1".to_owned()),
        "v1 pushed"
    );
    assert!(
        remote_tags.contains(&"refs/tags/v2".to_owned()),
        "v2 pushed"
    );
}
