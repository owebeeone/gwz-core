use super::*;

/// A loopback listener that accepts every connection and drops it at once, counting them: a refusal that reaches the
/// network fails the row, and an accept ends a native SSH, HTTP or git session quickly instead of after its timeout.
pub(crate) struct Recorder {
    pub(crate) port: u16,
    pub(crate) accepted: Arc<AtomicUsize>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) thread: Option<JoinHandle<()>>,
}
impl Recorder {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, done) = (accepted.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        count.fetch_add(1, Ordering::AcqRel);
                        drop(stream);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self {
            port,
            accepted,
            stop,
            thread: Some(thread),
        }
    }
    /// The connections made so far, after the listener has had time to see any that are in flight.
    pub(crate) fn accepted(&self) -> usize {
        std::thread::sleep(Duration::from_millis(100));
        self.accepted.load(Ordering::Acquire)
    }
    pub(crate) fn assert_untouched(&self, what: &str) {
        assert_eq!(
            self.accepted(),
            0,
            "{what}: a refused route reached the network"
        );
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The three routes a rewritten URL can land on: the words of the refusal, the URL the caller names (the rule's
/// prefix) and the URL the rule makes of it.
pub(crate) struct Rewrites {
    pub(crate) source: PathBuf,
    pub(crate) cases: Vec<(&'static str, String, String)>,
}
#[test]
pub(crate) fn an_insteadof_rewrite_to_a_refused_route_is_refused_for_the_read_and_the_push() {
    let harness = Harness::new();
    let recorder = Recorder::start();
    let rewrites = Rewrites::new(harness.temp.path(), recorder.port);
    let (request, client) = harness.request("route-insteadof");
    let before = git2::Repository::open(&rewrites.source)
        .unwrap()
        .head()
        .unwrap()
        .target();
    for (index, (route, named, _)) in rewrites.cases.iter().enumerate() {
        // `origin` is elsewhere, so a prepared push to `named` goes through an anonymous remote.
        let receiver = receiver_repository(
            harness.temp.path(),
            &format!("rewrite-{index}"),
            "ssh://nobody@127.0.0.1:9/x",
        );
        rewrites.set_in(&receiver, "insteadOf");
        let backend = request.backend();
        let refspec = "refs/heads/main:refs/heads/pushed";
        let plan = crate::git::GitPreparedPush {
            remote: "origin".into(),
            url: named.clone(),
            refspecs: vec![refspec.into()],
        };
        assert_names_route(
            &backend
                .ls_remote_url(&receiver, named, "origin", None)
                .unwrap_err(),
            route,
            "ls_remote_url",
        );
        assert_names_route(
            &backend.push_prepared(&receiver, &plan).unwrap_err(),
            route,
            "push_prepared",
        );
        assert_no_ref_or_fetch_head(&receiver, route);
        recorder.assert_untouched(route);
    }
    let after = git2::Repository::open(&rewrites.source)
        .unwrap()
        .head()
        .unwrap()
        .target();
    assert_eq!(before, after);
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_pushinsteadof_rewrite_to_a_refused_route_is_refused_for_the_prepare_and_the_push() {
    let harness = Harness::new();
    let recorder = Recorder::start();
    let rewrites = Rewrites::new(harness.temp.path(), recorder.port);
    let (request, client) = harness.request("route-pushinsteadof");
    for (index, (route, named, _)) in rewrites.cases.iter().enumerate() {
        // Through a named remote: the push URL the rule makes is the one `prepare_push` records.
        let sender = receiver_repository(harness.temp.path(), &format!("pushrule-{index}"), named);
        rewrites.set_in(&sender, "pushInsteadOf");
        let backend = request.backend();
        let refspec = "refs/heads/main:refs/heads/pushed";
        assert_names_route(
            &backend
                .prepare_push(&sender, "origin", refspec)
                .unwrap_err(),
            route,
            "prepare_push",
        );
        assert_names_route(
            &backend.push(&sender, "origin", refspec).unwrap_err(),
            route,
            "push",
        );
        // Through an anonymous remote: `origin` is elsewhere, and the plan names the URL the rule rewrites.
        let moved = receiver_repository(
            harness.temp.path(),
            &format!("pushrule-moved-{index}"),
            "ssh://nobody@127.0.0.1:9/x",
        );
        rewrites.set_in(&moved, "pushInsteadOf");
        let plan = crate::git::GitPreparedPush {
            remote: "origin".into(),
            url: named.clone(),
            refspecs: vec![refspec.into()],
        };
        assert_names_route(
            &backend.push_prepared(&moved, &plan).unwrap_err(),
            route,
            "push_prepared",
        );
        recorder.assert_untouched(route);
    }
    let source = git2::Repository::open(&rewrites.source).unwrap();
    assert!(source.find_reference("refs/heads/pushed").is_err());
    block_on(request.finish());
    block_on(client.finish());
}

/// The refusal's words when the new repository's own configuration, and not the global one the judge read, made the
/// rewrite: the clone's remote was observed after it was created.
const OBSERVED_AFTER_CREATION: &str = "after the route was judged";

/// Writes `text` as the global configuration of the child process the row runs in (its `HOME`), which libgit2 reads
/// when the child first needs its configuration.
fn write_global_configuration(home: &Path, text: &str) {
    std::fs::write(home.join(".gitconfig"), text).unwrap();
}

#[test]
pub(crate) fn a_global_insteadof_rewrite_to_a_refused_route_is_refused_for_the_clone_and_the_read()
{
    let Some(home) = child_home() else {
        let home = TempDir::new().unwrap();
        rerun_in_child(
            module_path!(),
            "a_global_insteadof_rewrite_to_a_refused_route_is_refused_for_the_clone_and_the_read",
            &child_environment(home.path()),
        );
        return;
    };
    let harness = Harness::new();
    let recorder = Recorder::start();
    let rewrites = Rewrites::new(harness.temp.path(), recorder.port);
    write_global_configuration(&home, &rewrites.config_text("insteadOf"));
    let (request, client) = harness.request("route-global-insteadof");
    for (index, (route, named, _)) in rewrites.cases.iter().enumerate() {
        let target = harness.temp.path().join(format!("rewrite-clone-{index}"));
        let backend = request.backend();
        let error = backend.clone_repo(named, &target).unwrap_err();
        assert_names_route(&error, route, "clone");
        // The rule is unconditional, so the judge predicted it: the clone was refused before its target existed, and
        // not by observing the remote the new repository created.
        assert!(
            !error.message.contains(OBSERVED_AFTER_CREATION),
            "{route}: {}",
            error.message
        );
        assert!(
            !target.exists(),
            "{route}: the refused clone made its target"
        );
        assert_names_route(
            &backend
                .read_remote_file(named, "origin", "payload")
                .unwrap_err(),
            route,
            "read_remote_file",
        );
        recorder.assert_untouched(route);
    }
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_conditional_include_rewrite_to_a_refused_route_is_refused_by_name_for_the_clone_and_the_read()
 {
    let Some(home) = child_home() else {
        let home = TempDir::new().unwrap();
        rerun_in_child(
            module_path!(),
            "a_conditional_include_rewrite_to_a_refused_route_is_refused_by_name_for_the_clone_and_the_read",
            &child_environment(home.path()),
        );
        return;
    };
    let harness = Harness::new();
    let recorder = Recorder::start();
    let rewrites = Rewrites::new(harness.temp.path(), recorder.port);
    // The rules apply to repositories under the temporary directory, the clone's target and the read's scratch
    // repository among them, and only once the repository exists: the judge, which reads no repository, cannot see them.
    include_rules_under_temporary(
        &home,
        &harness.temp.path().join("conditional.gitconfig"),
        &rewrites.config_text("insteadOf"),
    );
    let (request, client) = harness.request("route-conditional-include");
    for (index, (route, named, _)) in rewrites.cases.iter().enumerate() {
        let target = harness
            .temp
            .path()
            .join(format!("conditional-clone-{index}"));
        let backend = request.backend();
        let error = backend.clone_repo(named, &target).unwrap_err();
        assert_names_route(&error, route, "clone");
        assert!(
            error.message.contains(OBSERVED_AFTER_CREATION),
            "{route}: {}",
            error.message
        );
        assert!(
            !target.exists(),
            "{route}: the refused clone left its target"
        );
        let error = backend
            .read_remote_file(named, "origin", "payload")
            .unwrap_err();
        assert_names_route(&error, route, "read_remote_file");
        assert!(
            error.message.contains(OBSERVED_AFTER_CREATION),
            "{route}: {}",
            error.message
        );
        recorder.assert_untouched(route);
    }
    block_on(request.finish());
    block_on(client.finish());
}

#[test]
pub(crate) fn a_conditional_include_rewrite_to_a_served_route_is_refused_only_when_it_is_another_kind()
 {
    let Some(home) = child_home() else {
        let home = TempDir::new().unwrap();
        rerun_in_child(
            module_path!(),
            "a_conditional_include_rewrite_to_a_served_route_is_refused_only_when_it_is_another_kind",
            &child_environment(home.path()),
        );
        return;
    };
    let harness = Harness::new();
    let recorder = Recorder::start();
    include_rules_under_temporary(
        &home,
        &harness.temp.path().join("served.gitconfig"),
        &format!(
            "[url \"ssh://git@127.0.0.1:{port}/s.git\"]\n\tinsteadOf = https://example.invalid/to-ssh\n\
             [url \"https://127.0.0.1:{port}/s.git\"]\n\tinsteadOf = https://example.invalid/to-https\n",
            port = recorder.port
        ),
    );
    let (request, client) = harness.request("route-conditional-served");
    // An https:// remote the new repository's configuration makes an ssh:// one is another kind of route than the
    // one judged and bound: refused, with nothing connected.
    let target = harness.temp.path().join("conditional-ssh");
    let error = request
        .backend()
        .clone_repo("https://example.invalid/to-ssh", &target)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error.message.contains("different route"),
        "{}",
        error.message
    );
    assert!(!target.exists());
    recorder.assert_untouched("a rewrite to another kind of served route");
    // An https:// remote made another https:// one is the kind judged: it is not refused as a route.
    let target = harness.temp.path().join("conditional-https");
    let error = request
        .backend()
        .clone_repo("https://example.invalid/to-https", &target)
        .unwrap_err();
    assert!(
        !error.message.contains("does not serve"),
        "{}",
        error.message
    );
    block_on(request.finish());
    block_on(client.finish());
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The key the conditional rules file also sets: a repository that reads the file has it.
const CONDITIONAL_MARKER: &str = "gwzrules.conditional";

/// `directory` spelled as libgit2 compares it with the git directory of a repository under it: forward slashes, and
/// the plain `C:/...` or `//server/share/...` form, not the `\\?\` (verbatim) one that `canonicalize` makes on Windows,
/// which no git directory is ever spelled in.
fn spelled_as_a_git_directory(directory: &Path) -> String {
    let text = forward_slashes(directory);
    if let Some(unc) = text.strip_prefix("//?/UNC/") {
        return format!("//{unc}");
    }
    match text.strip_prefix("//?/") {
        Some(plain) => plain.to_owned(),
        None => text,
    }
}

/// Makes `rules` (written to `rules_path`) apply to every repository under the temporary directory, through one
/// `includeIf "gitdir/i:"` of the global configuration in `home`, and then proves that it does: a repository made there
/// reads the file, so a pattern libgit2 does not match against the git directory it sees (the verbatim `\\?\` spelling,
/// a case or 8.3-name difference) fails here, by name, and cannot leave the rewrite unapplied for the row to pass or
/// fail on some other ground. `gitdir/i:` because Windows paths are not case-sensitive.
fn include_rules_under_temporary(home: &Path, rules_path: &Path, rules: &str) {
    std::fs::write(
        rules_path,
        format!("{rules}[gwzrules]\n\tconditional = applied\n"),
    )
    .unwrap();
    let temporary = spelled_as_a_git_directory(&std::env::temp_dir().canonicalize().unwrap());
    let condition = format!("gitdir/i:{temporary}/");
    write_global_configuration(
        home,
        &format!(
            "[includeIf \"{condition}\"]\n\tpath = {}\n",
            forward_slashes(rules_path)
        ),
    );
    let probe = TempDir::new().unwrap();
    let repository = git2::Repository::init(probe.path().join("rules-probe")).unwrap();
    let applied = repository
        .config()
        .unwrap()
        .get_string(CONDITIONAL_MARKER)
        .ok();
    assert_eq!(
        applied.as_deref(),
        Some("applied"),
        "the conditional include `{condition}` was not applied to the repository at {}: the rewrite under test would \
         never be made",
        repository.path().display()
    );
}
