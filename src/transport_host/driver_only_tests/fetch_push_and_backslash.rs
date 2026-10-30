use super::*;

/// A repository with `url` as its `origin`, and no fetched ref.
pub(crate) fn receiver_repository(root: &Path, name: &str, url: &str) -> PathBuf {
    let path = root.join(name);
    let repo = git2::Repository::init(&path).unwrap();
    repo.remote("origin", url).unwrap();
    path
}

#[test]
pub(crate) fn a_fetch_ls_remote_or_tag_fetch_of_a_local_only_route_is_refused_with_no_ref_written()
{
    let harness = Harness::new();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-fetch");
    for (index, (route, url)) in routes.cases.iter().enumerate() {
        let receiver = receiver_repository(harness.temp.path(), &format!("fetch-{index}"), url);
        let backend = request.backend();
        let errors = [
            ("fetch", backend.fetch(&receiver, "origin").unwrap_err()),
            (
                "tag_fetch",
                backend.tag_fetch(&receiver, "origin").unwrap_err(),
            ),
            (
                "ls_remote",
                backend.ls_remote(&receiver, "origin").unwrap_err(),
            ),
            (
                "ls_remote_url",
                backend
                    .ls_remote_url(&receiver, url, "origin", None)
                    .unwrap_err(),
            ),
        ];
        for (operation, error) in &errors {
            assert_names_route(error, route, operation);
        }
        let repo = git2::Repository::open(&receiver).unwrap();
        assert_eq!(
            repo.references().unwrap().count(),
            0,
            "{route}: a ref was written"
        );
        assert!(
            !receiver.join(".git/FETCH_HEAD").exists(),
            "{route}: FETCH_HEAD was written"
        );
    }
    routes.assert_untouched();
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_push_to_a_local_only_route_is_refused_and_the_peer_keeps_its_refs() {
    let harness = Harness::new();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-push");
    let source = git2::Repository::open(&routes.source).unwrap();
    let before = source.head().unwrap().target();
    for (index, (route, url)) in routes.cases.iter().enumerate() {
        let sender = receiver_repository(harness.temp.path(), &format!("push-{index}"), url);
        let backend = request.backend();
        let refspec = "refs/heads/main:refs/heads/pushed";
        let plan = crate::git::GitPreparedPush {
            remote: "origin".into(),
            url: url.clone(),
            refspecs: vec![refspec.into()],
        };
        assert_names_route(
            &backend.push(&sender, "origin", refspec).unwrap_err(),
            route,
            "push",
        );
        assert_names_route(
            &backend.push_prepared(&sender, &plan).unwrap_err(),
            route,
            "push_prepared",
        );
    }
    assert_eq!(source.head().unwrap().target(), before);
    assert!(source.find_reference("refs/heads/pushed").is_err());
    routes.assert_untouched();
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_clone_family_local_peer_is_refused_and_the_receiver_is_not_touched() {
    let harness = Harness::new();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-family");
    let receiver = receiver_repository(
        harness.temp.path(),
        "family-receiver",
        "ssh://nobody@127.0.0.1:9/x",
    );
    let source = routes.source.to_string_lossy().into_owned();
    let backend = request.backend();
    let fetched = backend
        .fetch_anonymous(
            &receiver,
            &source,
            &["+refs/heads/*:refs/gwz/local-imports/x/*"],
        )
        .unwrap_err();
    assert_names_route(&fetched, "clone-family local peer", "fetch_anonymous");
    let pushed = backend
        .push_anonymous(&receiver, &source, "refs/heads/main:refs/heads/main")
        .unwrap_err();
    assert_names_route(&pushed, "clone-family local peer", "push_anonymous");
    // libgit2 creates FETCH_HEAD even when it fetches nothing, so its absence shows the fetch never ran.
    assert!(!receiver.join(".git/FETCH_HEAD").exists());
    assert_eq!(
        git2::Repository::open(&receiver)
            .unwrap()
            .references()
            .unwrap()
            .count(),
        0
    );
    routes.assert_untouched();
    block_on(request.finish());
    block_on(client.finish());
}

// The route the binding binds, and the URL libgit2 connects to (remediation round 1).

/// SCP-like spellings with a backslash before the first top-level `:`: the check once read them as SSH, and the binding
/// read them as local paths and left them to libgit2's own SSH. The bracketed ones name the recorder's port.
pub(crate) fn backslash_urls(port: u16) -> Vec<String> {
    vec![
        format!(r"[git\x@127.0.0.1:{port}]:source.git"),
        format!(r"[u\x@127.0.0.1:{port}]:r"),
    ]
}

#[test]
pub(crate) fn a_backslash_before_the_first_colon_is_a_local_path_and_nothing_connects() {
    let harness = Harness::new();
    let recorder = Recorder::start();
    let (request, client) = harness.request("route-backslash");
    for (index, url) in backslash_urls(recorder.port).iter().enumerate() {
        let receiver = receiver_repository(harness.temp.path(), &format!("slash-{index}"), url);
        let backend = request.backend();
        let refspec = "refs/heads/main:refs/heads/pushed";
        let plan = crate::git::GitPreparedPush {
            remote: "origin".into(),
            url: url.clone(),
            refspecs: vec![refspec.into()],
        };
        let clone_target = harness.temp.path().join(format!("slash-clone-{index}"));
        let errors = [
            (
                "ls_remote",
                backend.ls_remote(&receiver, "origin").unwrap_err(),
            ),
            ("fetch", backend.fetch(&receiver, "origin").unwrap_err()),
            (
                "tag_fetch",
                backend.tag_fetch(&receiver, "origin").unwrap_err(),
            ),
            (
                "push",
                backend.push(&receiver, "origin", refspec).unwrap_err(),
            ),
            (
                "push_prepared",
                backend.push_prepared(&receiver, &plan).unwrap_err(),
            ),
            (
                "ls_remote_url",
                backend
                    .ls_remote_url(&receiver, url, "origin", None)
                    .unwrap_err(),
            ),
            ("clone", backend.clone_repo(url, &clone_target).unwrap_err()),
            (
                "read_remote_file",
                backend
                    .read_remote_file(url, "origin", "payload")
                    .unwrap_err(),
            ),
        ];
        for (operation, error) in &errors {
            assert_names_route(error, "local path", operation);
        }
        assert!(!clone_target.exists());
        assert_no_ref_or_fetch_head(&receiver, url);
    }
    recorder.assert_untouched("a backslash SCP spelling");
    block_on(request.finish());
    block_on(client.finish());
}

pub(crate) fn assert_no_ref_or_fetch_head(receiver: &Path, what: &str) {
    assert_eq!(
        git2::Repository::open(receiver)
            .unwrap()
            .references()
            .unwrap()
            .count(),
        0,
        "{what}: a ref was written"
    );
    assert!(
        !receiver.join(".git/FETCH_HEAD").exists(),
        "{what}: FETCH_HEAD was written"
    );
}
