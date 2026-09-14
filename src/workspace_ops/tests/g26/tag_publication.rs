//! Push plan step 2.2 (`dev-docs/GwzUrlSchemePushPlan.md`): tag publication
//! parity. A root tag push validates, reads and proves its lock dependencies
//! through the read URLs step 2.1 selects. These tests pin which URLs are used,
//! not how many reads are made, which step 3.3 changes.
use super::*;

/// An https clone of a workspace whose manifest keeps SSH URLs: pushing a root
/// tag uses each dependency's https member remote and reads nothing over SSH.
#[test]
fn a_root_tag_push_reads_its_dependencies_only_through_https() {
    let fixture = PublicationFixture::new(ROOT_HTTPS, APP_HTTPS, LIB_HTTPS);

    let reads = push_root_tag(&fixture, [ROOT_HTTPS, APP_HTTPS, LIB_HTTPS]);

    let ssh = reads
        .iter()
        .filter(|call| matches!(call, RemoteCall::Read { url, .. } if crate::git::uses_ssh(url)))
        .count();
    assert_eq!(ssh, 0);
}

/// Member remotes equal their SSH committed URLs, so the read URLs are the
/// committed URLs: a root tag push uses the URLs it used before step 2.1.
#[test]
fn an_all_ssh_root_tag_push_reads_its_committed_urls() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);

    push_root_tag(&fixture, [ROOT_SSH, APP_SSH, LIB_SSH]);
}

/// Tag the root head, whose committed lock names `app` and `lib`, publish `app`
/// so the proof can hold, and push the tag with `@root` selected, which tag's
/// members-only default leaves out. The tag must be the only transfer and come
/// after every read, and identity validation and every read must use only
/// `urls`: the root's own URL, then `app`'s and `lib`'s read URLs.
/// Returns the reads.
fn push_root_tag(fixture: &PublicationFixture, urls: [&str; 3]) -> Vec<RemoteCall> {
    let [root_url, app_url, lib_url] = urls;
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
    fixture.backend.set_tag(root, "v1", ROOT_HEAD);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    let world = crate::operation_context::TestWorld::physical();
    let request = crate::TagRequest {
        meta: crate::RequestMeta {
            selection: root_only(),
            ..request_meta_with_workspace()
        },
        op: crate::TagOp::Push,
        name: Some("v1".to_owned()),
        ..Default::default()
    };

    let response =
        handle_tag_in(&world.context(), &fixture.backend, root, request, "op_tag").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let tag_push = RemoteCall::Push {
        path: root.to_path_buf(),
        remote: "origin".to_owned(),
        url: root_url.to_owned(),
        refspecs: vec![format!("{ROOT_HEAD}:refs/tags/v1")],
    };
    assert_eq!(fixture.backend.prepared_pushes(), vec![tag_push.clone()]);
    assert_eq!(fixture.backend.remote_calls().last(), Some(&tag_push));
    let validated = |member: &Path, url: &str| {
        (
            Some(member.to_path_buf()),
            "origin".to_owned(),
            url.to_owned(),
        )
    };
    assert_eq!(
        fixture.backend.url_identity_checks(),
        vec![validated(&app, app_url), validated(&lib, lib_url)]
    );
    // Each read is one of these, however many reads a step makes.
    let expected = [
        read_at(root, root_url, root),
        read_at(root, app_url, &app),
        read_at(root, lib_url, &lib),
    ];
    let reads = fixture.backend.remote_reads();
    for call in &expected {
        assert!(reads.contains(call), "{call:?} was not read: {reads:#?}");
    }
    for call in &reads {
        assert!(expected.contains(call), "unexpected read {call:?}");
    }
    reads
}
