use super::*;

/// A repository the refused routes would reach if they were served: one commit on `main`.
pub(crate) fn source_repository(root: &Path) -> (PathBuf, git2::Oid) {
    let path = root.join("source");
    let repo = git2::Repository::init(&path).unwrap();
    std::fs::write(path.join("payload"), b"payload").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("payload")).unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let id = repo
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "first",
            &tree,
            &[],
        )
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    (path, id)
}

/// The ways to name a remote that the `cli` placement does not serve, each with the words its refusal must carry.
/// `http://` and `git://` name a loopback listener that records every connection: a refusal that connects fails the row.
pub(crate) struct Routes {
    pub(crate) source: PathBuf,
    pub(crate) listener: TcpListener,
    pub(crate) cases: Vec<(&'static str, String)>,
}
impl Routes {
    pub(crate) fn new(root: &Path) -> Self {
        let (source, _) = source_repository(root);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let file = url::Url::from_file_path(&source).unwrap().to_string();
        let cases = vec![
            ("file://", file),
            ("local path", source.to_string_lossy().into_owned()),
            ("http://", format!("http://127.0.0.1:{port}/source.git")),
            ("git://", format!("git://127.0.0.1:{port}/source.git")),
        ];
        Self {
            source,
            listener,
            cases,
        }
    }
    /// Nothing connected to the loopback listener.
    pub(crate) fn assert_untouched(&self) {
        match self.listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("a refused route reached the network: {other:?}"),
        }
    }
}

pub(crate) fn assert_names_route(error: &ModelError, route: &str, operation: &str) {
    assert_eq!(
        error.code,
        ErrorCode::UnsupportedOperation,
        "{operation} {route}"
    );
    assert!(
        error.message.contains(&format!("the {route} route")),
        "{operation} {route}: {}",
        error.message
    );
    assert!(
        error.message.contains("cli placement"),
        "{operation} {route}: {}",
        error.message
    );
}

#[test]
pub(crate) fn a_clone_of_a_local_only_route_is_refused_before_it_makes_the_target() {
    let harness = Harness::new();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-clone");
    for (index, (route, url)) in routes.cases.iter().enumerate() {
        let target = harness.temp.path().join(format!("clone-{index}"));
        let error = request.backend().clone_repo(url, &target).unwrap_err();
        assert_names_route(&error, route, "clone");
        assert!(
            !target.exists(),
            "{route}: the refused clone made its target"
        );
    }
    routes.assert_untouched();
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_remote_file_read_from_a_local_only_route_is_refused() {
    let harness = Harness::new();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-read");
    for (route, url) in &routes.cases {
        let error = request
            .backend()
            .read_remote_file(url, "origin", "payload")
            .unwrap_err();
        assert_names_route(&error, route, "read_remote_file");
    }
    routes.assert_untouched();
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn the_binding_in_a_cli_context_never_leaves_the_native_route_in_place() {
    let harness = Harness::new();
    let recorder = Recorder::start();
    let (source, _) = source_repository(harness.temp.path());
    let (request, client) = harness.request("route-binding");
    let cases: Vec<(&str, String)> = vec![
        (
            "file://",
            url::Url::from_file_path(&source).unwrap().to_string(),
        ),
        ("local path", source.to_string_lossy().into_owned()),
        (
            "http://",
            format!("http://127.0.0.1:{}/s.git", recorder.port),
        ),
        ("git://", format!("git://127.0.0.1:{}/s.git", recorder.port)),
        (
            "local path",
            format!(r"[git\x@127.0.0.1:{}]:s.git", recorder.port),
        ),
    ];
    for windows in [false, true] {
        let mut backend = request.backend().clone();
        if windows {
            // Windows carries a plain http:// remote on libgit2's own route; a cli host does not.
            backend.ssh = backend.ssh.with_windows_policy_for_test();
        }
        for (route, url) in &cases {
            let repo = git2::Repository::init(harness.temp.path().join("binding")).unwrap();
            let mut remote = repo.remote_anonymous(url).unwrap();
            let connected = remote.connect_auth(
                git2::Direction::Fetch,
                Some(backend.remote_callbacks_for_test(url)),
                None,
            );
            let Err(error) = connected else {
                panic!("windows={windows} {route}: the binding left libgit2's own route in place");
            };
            assert!(
                error.message().contains(&format!("the {route} route")),
                "windows={windows} {route}: {}",
                error.message()
            );
        }
    }
    recorder.assert_untouched("the binding of an unbound route");
    block_on(request.finish());
    block_on(client.finish());
}

impl Rewrites {
    pub(crate) fn new(root: &Path, port: u16) -> Self {
        let (source, _) = source_repository(root);
        let file = url::Url::from_file_path(&source).unwrap().to_string();
        let cases = vec![
            ("file://", "https://example.invalid/file".to_owned(), file),
            (
                "http://",
                "https://example.invalid/http".to_owned(),
                format!("http://127.0.0.1:{port}/h"),
            ),
            (
                "git://",
                "https://example.invalid/git".to_owned(),
                format!("git://127.0.0.1:{port}/g"),
            ),
        ];
        Self { source, cases }
    }
    /// The rules as a configuration file's text (`rule` is `insteadOf` or `pushInsteadOf`).
    pub(crate) fn config_text(&self, rule: &str) -> String {
        self.cases
            .iter()
            .map(|(_, named, target)| format!("[url \"{target}\"]\n\t{rule} = {named}\n"))
            .collect()
    }
    pub(crate) fn set_in(&self, repo: &Path, rule: &str) {
        let mut config = git2::Repository::open(repo).unwrap().config().unwrap();
        for (_, named, target) in &self.cases {
            config
                .set_str(&format!("url.{target}.{}", rule.to_lowercase()), named)
                .unwrap();
        }
    }
}

/// The refused routes of a remote file read, in a process whose temporary directory is a path that does not exist: a
/// read that makes its scratch directory before the route check fails with an I/O error instead of the route's name,
/// and the path must still not exist after it.
pub(crate) fn scratch_child(root: PathBuf) {
    let harness = Harness::in_dir(TempDir::new_in(&root).unwrap());
    let recorder = Recorder::start();
    let routes = Routes::new(harness.temp.path());
    let (request, client) = harness.request("route-scratch");
    let mut cases: Vec<(&str, String)> = routes.cases.clone();
    cases.extend(
        backslash_urls(recorder.port)
            .into_iter()
            .map(|url| ("local path", url)),
    );
    for (route, url) in &cases {
        let error = request
            .backend()
            .read_remote_file(url, "origin", "payload")
            .unwrap_err();
        assert_names_route(&error, route, "read_remote_file (no scratch root)");
        assert!(
            !root.join("no-scratch-root").exists(),
            "{route}: the scratch root was made"
        );
    }
    recorder.assert_untouched("a read with no scratch root");
    block_on(request.finish());
    block_on(client.finish());
}
