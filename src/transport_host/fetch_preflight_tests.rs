use super::driver_tests::{CliHarness, block_on, fixture_url};
use crate::RemoteSshIdentity;
use crate::operation::NullSink;
use crate::workspace_ops::{handle_fetch, handle_init_from_sources};

#[test]
fn fetch_checks_later_repo_identity_before_any_tracking_ref_moves() {
    let harness = CliHarness::new();
    let server = git2::Repository::open_bare(&harness.fixture.repository).unwrap();
    let first = {
        let blob = server.blob(b"first").unwrap();
        let mut tree = server.treebuilder(None).unwrap();
        tree.insert("payload", blob, 0o100644).unwrap();
        let tree = server.find_tree(tree.write().unwrap()).unwrap();
        let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
        server.set_head("refs/heads/main").unwrap();
        server
            .commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])
            .unwrap()
    };
    let root = harness.fixture.temp.path().join("fetch-preflight");
    std::fs::create_dir(&root).unwrap();
    let init_meta = harness.meta("fetch-preflight-init");
    let init_client = harness
        .endpoint
        .register_request(&init_meta.request_id)
        .unwrap();
    let init_request = block_on(harness.runtime.request(init_meta.clone(), "init".into())).unwrap();
    handle_init_from_sources(
        init_request.backend(),
        &root,
        crate::InitFromSourcesRequest {
            meta: init_meta,
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: fixture_url(&harness.fixture),
                    path: Some("first".into()),
                    ..Default::default()
                },
                crate::SourceUrl {
                    url: fixture_url(&harness.fixture),
                    path: Some("last".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        "init",
        &NullSink,
    )
    .unwrap();
    let _ = block_on(init_request.finish());
    let _ = block_on(init_client.finish());

    for member in ["first", "last"] {
        git2::Repository::open(root.join(member))
            .unwrap()
            .config()
            .unwrap()
            .set_str(
                "remote.origin.gwzSshIdentity",
                if member == "first" {
                    "client_ed25519"
                } else {
                    "missing-last-key"
                },
            )
            .unwrap();
    }
    let before_first = git2::Repository::open(root.join("first"))
        .unwrap()
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .target();
    let before_last = git2::Repository::open(root.join("last"))
        .unwrap()
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .target();
    assert_eq!(before_first, Some(first));
    assert_eq!(before_last, Some(first));
    let second = {
        let blob = server.blob(b"second").unwrap();
        let mut tree = server.treebuilder(None).unwrap();
        tree.insert("payload", blob, 0o100644).unwrap();
        let tree = server.find_tree(tree.write().unwrap()).unwrap();
        let sig = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
        server
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "second",
                &tree,
                [&server.find_commit(first).unwrap()].as_slice(),
            )
            .unwrap()
    };

    let mut fetch_meta = harness.meta("fetch-preflight");
    let transport = fetch_meta.transport.as_mut().unwrap();
    transport.default_identity = None;
    transport.remote_identities = Vec::<RemoteSshIdentity>::new();
    let fetch_client = harness
        .endpoint
        .register_request(&fetch_meta.request_id)
        .unwrap();
    let fetch_request =
        block_on(harness.runtime.request(fetch_meta.clone(), "fetch".into())).unwrap();
    let result = handle_fetch(
        fetch_request.backend(),
        &root,
        crate::FetchRequest { meta: fetch_meta },
        "fetch",
    );
    let error = result.expect_err("missing later identity must reject before fanout");
    assert_eq!(
        git2::Repository::open(root.join("first"))
            .unwrap()
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        before_first
    );
    assert_eq!(
        git2::Repository::open(root.join("last"))
            .unwrap()
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        before_last
    );
    assert_ne!(second, first);
    assert!(
        error
            .response_meta
            .as_ref()
            .and_then(|meta| meta.transport.as_ref())
            .is_none_or(Vec::is_empty)
    );
    let _ = block_on(fetch_request.finish());
    let _ = block_on(fetch_client.finish());
    let _ = block_on(harness.endpoint.shutdown());
}
