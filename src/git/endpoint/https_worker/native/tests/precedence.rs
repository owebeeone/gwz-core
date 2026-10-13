//! Which identity answers (TR1.8 section 7, TD1, TD2): the logon session answers `Negotiate` and `NTLM` and the
//! helper is not asked; the helper answers `Basic` only; `Digest` is never answered. The helper here is the real
//! Git with a script helper, so the same rows run on both platforms (a fake `git` cannot be a script on Windows).
use super::*;
use std::path::{Path, PathBuf};

/// The real Git configured with one credential helper whose every call is logged: the helper's answer is a fixed
/// credential. `None` configures no helper at all.
struct Helper {
    _directory: tempfile::TempDir,
    config: https_auth::Config,
    log: PathBuf,
}
impl Helper {
    fn new(answers: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let bin = root.join("bin dir");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("helper.log");
        // The script is read by `sh`, on Windows by Git's own, so its paths are written with forward slashes.
        let posix = |path: &Path| path.display().to_string().replace('\\', "/");
        crate::git::endpoint::helper_script::write_helper_script(
            &bin.join("git-credential-tr44"),
            &format!(
                "cat >/dev/null\necho asked >> '{}'\nprintf 'username=fixture\\npassword=token\\n\\n'\n",
                posix(&log)
            ),
        );
        let global = root.join("gitconfig");
        std::fs::write(
            &global,
            if answers {
                "[credential]\n\thelper = tr44\n"
            } else {
                ""
            },
        )
        .unwrap();
        let inherited = std::env::var_os("PATH").expect("the test process has a PATH");
        let mut entries = vec![bin];
        entries.extend(std::env::split_paths(&inherited));
        let mut environment: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
            ("HOME".into(), root.as_os_str().into()),
            ("PATH".into(), std::env::join_paths(entries).unwrap()),
            ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        if let Some(system) = std::env::var_os("SystemRoot") {
            environment.push(("SystemRoot".into(), system));
        }
        Self {
            _directory: directory,
            config: https_auth::Config {
                executable: PathBuf::from("git"),
                environment,
            },
            log,
        }
    }
    /// How many times the helper was asked.
    fn asked(&self) -> usize {
        std::fs::read_to_string(&self.log).map_or(0, |text| text.lines().count())
    }
}

/// A server that challenges with `offer` until a request carries an `Authorization` header, and counts what it sees.
struct Challenger {
    server: crate::git::endpoint::https_fixture::Server,
    requests: Arc<AtomicUsize>,
    authorized: Arc<Mutex<Vec<String>>>,
}
impl Challenger {
    async fn new(offer: &'static str) -> Self {
        use crate::git::endpoint::https_fixture::{Server, response};
        let requests = Arc::new(AtomicUsize::new(0));
        let authorized = Arc::new(Mutex::new(Vec::new()));
        let (seen, headers) = (requests.clone(), authorized.clone());
        let server = Server::start(Arc::new(move |request| {
            seen.fetch_add(1, Ordering::Relaxed);
            let credential = request
                .headers()
                .get(AUTHORIZATION)
                .map(|value| value.to_str().unwrap_or_default().to_owned());
            let given = credential.is_some();
            if let Some(credential) = credential {
                headers.lock().unwrap().push(credential);
            }
            Box::pin(async move {
                let mut result = response(
                    if given { 200 } else { 401 },
                    GitService::UploadPackAdvertisement,
                    Bytes::new(),
                );
                if !given {
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                }
                result
            })
        }))
        .await;
        Self {
            server,
            requests,
            authorized,
        }
    }
}

async fn open(
    challenger: &Challenger,
    helper: Option<&Helper>,
    policy: AuthPolicy,
    caller: Option<NativeCaller>,
) -> (Endpoint, Result<Prepared, Failure>) {
    use crate::git::endpoint::https_fixture::input;
    let mut endpoint = Endpoint::new(
        challenger.server.config(),
        helper.map(|helper| helper.config.clone()),
        pool::Config::default(),
    )
    .unwrap();
    if let Some(caller) = caller {
        endpoint.client.set_native(caller);
    }
    let mut request = input(&challenger.server, GitService::UploadPackAdvertisement);
    request.policy = policy;
    let (result, _) = endpoint
        .client
        .prepare_attempt(
            request,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    (endpoint, result)
}

async fn finish(mut endpoint: Endpoint, result: Result<Prepared, Failure>) {
    drop(result);
    endpoint.client.finish_operation("operation");
    // A native session a failed exchange cancelled is reaped here: shutdown counts the slot it holds while it is
    // still pending, and a connection that is still closing is counted by shutdown, which waits for it.
    let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
}

/// The words the host gives a failed open under `policy`, with the host named `git.example.test`.
pub(super) fn words(failure: &Failure, policy: AuthPolicy) -> String {
    let (helpers_disabled, windows) = crate::transport_host::policy_words(Some(policy));
    crate::transport_host::HttpsOpenFailure {
        ended: None,
        failure: failure.clone(),
        anonymous: None,
        attempts: None,
        service: Some(GitService::UploadPackAdvertisement),
        helpers_disabled,
        cli_hint: true,
        windows,
        host: Some("git.example.test".into()),
        redirect: None,
        machine: Default::default(),
    }
    .reason()
}

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

#[test]
fn the_logon_session_answers_negotiate_and_ntlm_and_the_helper_is_not_asked() {
    run(async {
        for policy in [AuthPolicy::WindowsConfigured, AuthPolicy::WindowsDefault] {
            for offer in [
                "Negotiate",
                "NTLM",
                "Negotiate, Basic realm=x",
                "NTLM, Basic realm=x",
                "Digest realm=x, Negotiate",
                "Digest realm=x, NTLM",
                "Basic realm=x, NTLM, Negotiate",
            ] {
                let helper = Helper::new(true);
                let challenger = Challenger::new(offer).await;
                let (caller, port) = fake(true, 2);
                let (endpoint, result) =
                    open(&challenger, Some(&helper), policy, Some(caller)).await;
                let prepared = result
                    .as_ref()
                    .unwrap_or_else(|failure| panic!("{offer} {policy:?}: {failure:?}"));
                let native = prepared.opened.facts.native.as_ref().unwrap();
                assert_eq!(
                    native.source,
                    NativeSource::CurrentLogon,
                    "{offer} {policy:?}"
                );
                assert_eq!(
                    prepared.opened.facts.authenticated,
                    Some(true),
                    "{offer} {policy:?}"
                );
                assert_eq!(port.starts.load(Ordering::Acquire), 1, "{offer} {policy:?}");
                assert_eq!(
                    helper.asked(),
                    0,
                    "the helper was asked for {offer} under {policy:?}"
                );
                finish(endpoint, result).await;
            }
        }
    });
}

#[test]
fn basic_only_asks_the_helper_once_and_it_answers_over_basic() {
    run(async {
        let helper = Helper::new(true);
        let challenger = Challenger::new("Basic realm=x").await;
        let (endpoint, result) = open(
            &challenger,
            Some(&helper),
            AuthPolicy::WindowsConfigured,
            None,
        )
        .await;
        let prepared = result.as_ref().unwrap();
        assert_eq!(prepared.opened.facts.method, AuthMethod::Gh);
        assert!(prepared.opened.facts.native.is_none());
        assert_eq!(helper.asked(), 1);
        let sent = challenger.authorized.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("Basic "), "{sent:?}");
        finish(endpoint, result).await;
    });
}

#[test]
fn a_basic_challenge_with_a_utf8_realm_still_asks_the_helper_once() {
    run(async {
        let helper = Helper::new(true);
        let challenger = Challenger::new("Basic realm=\"Z\u{fc}rich\"").await;
        let (endpoint, result) = open(
            &challenger,
            Some(&helper),
            AuthPolicy::WindowsConfigured,
            None,
        )
        .await;
        let prepared = result
            .as_ref()
            .unwrap_or_else(|failure| panic!("{failure:?}"));
        assert_eq!(prepared.opened.facts.method, AuthMethod::Gh);
        assert_eq!(helper.asked(), 1);
        let sent = challenger.authorized.lock().unwrap().clone();
        assert!(
            sent.iter().all(|value| value.starts_with("Basic ")),
            "{sent:?}"
        );
        finish(endpoint, result).await;
    });
}

#[test]
fn digest_beside_basic_is_ignored_and_basic_goes_through_the_helper() {
    run(async {
        let helper = Helper::new(true);
        let challenger = Challenger::new("Digest realm=x, Basic realm=x").await;
        let (endpoint, result) = open(
            &challenger,
            Some(&helper),
            AuthPolicy::WindowsConfigured,
            None,
        )
        .await;
        let prepared = result.as_ref().unwrap();
        assert_eq!(prepared.opened.facts.method, AuthMethod::Gh);
        assert_eq!(helper.asked(), 1);
        let sent = challenger.authorized.lock().unwrap().clone();
        assert!(
            sent.iter().all(|value| value.starts_with("Basic ")),
            "{sent:?}"
        );
        finish(endpoint, result).await;
    });
}

#[test]
fn basic_only_with_no_helper_fails_with_the_helper_facts_and_sends_no_credential() {
    run(async {
        let helper = Helper::new(false);
        let challenger = Challenger::new("Basic realm=x").await;
        let (endpoint, result) = open(
            &challenger,
            Some(&helper),
            AuthPolicy::WindowsConfigured,
            None,
        )
        .await;
        let failure = result.as_ref().err().unwrap();
        assert_eq!(failure.code, ErrorCode::Authentication);
        let facts = failure.facts.as_ref().unwrap();
        assert_eq!(facts.method, AuthMethod::Gh);
        assert_ne!(facts.authenticated, Some(false));
        assert!(!facts.credential_offered);
        assert!(challenger.authorized.lock().unwrap().is_empty());
        assert_eq!(challenger.requests.load(Ordering::Acquire), 1);
        // T4: the host's words say how to configure one.
        let said = words(failure, AuthPolicy::WindowsConfigured);
        assert!(
            said.starts_with("The server asks for a user name and password (Basic) and no git credential helper is configured."),
            "{said}"
        );
        finish(endpoint, result).await;
    });
}

#[test]
fn basic_only_with_helpers_disabled_never_asks_the_helper() {
    run(async {
        let helper = Helper::new(true);
        let challenger = Challenger::new("Basic realm=x").await;
        let (endpoint, result) =
            open(&challenger, Some(&helper), AuthPolicy::WindowsDefault, None).await;
        let failure = result.as_ref().err().unwrap();
        assert_eq!(failure.code, ErrorCode::Authentication);
        assert_eq!(helper.asked(), 0);
        assert!(challenger.authorized.lock().unwrap().is_empty());
        assert_eq!(challenger.requests.load(Ordering::Acquire), 1);
        let said = words(failure, AuthPolicy::WindowsDefault);
        assert!(
            said.starts_with("The server asked for an HTTPS credential, and credential helpers are off for this operation"),
            "{said}"
        );
        finish(endpoint, result).await;
    });
}

#[test]
fn digest_only_is_refused_at_once_naming_digest_and_asks_nobody() {
    run(async {
        for policy in [AuthPolicy::WindowsConfigured, AuthPolicy::WindowsDefault] {
            let helper = Helper::new(true);
            let challenger = Challenger::new("Digest realm=x, nonce=\"abc\"").await;
            let (caller, port) = fake(true, 2);
            let (endpoint, result) = open(&challenger, Some(&helper), policy, Some(caller)).await;
            let failure = result.as_ref().err().unwrap();
            assert_eq!(failure.code, ErrorCode::Authentication, "{policy:?}");
            let names = failure
                .detail
                .as_ref()
                .and_then(|detail| detail.schemes.clone());
            assert_eq!(names, Some(vec!["Digest".to_owned()]), "{policy:?}");
            let said = words(failure, policy);
            assert!(
                said.starts_with("The server asks for HTTPS authentication by Digest"),
                "{said}"
            );
            assert_eq!(helper.asked(), 0, "{policy:?}");
            assert_eq!(port.starts.load(Ordering::Acquire), 0, "{policy:?}");
            assert!(
                challenger.authorized.lock().unwrap().is_empty(),
                "{policy:?}"
            );
            assert_eq!(challenger.requests.load(Ordering::Acquire), 1, "{policy:?}");
            finish(endpoint, result).await;
        }
    });
}

#[test]
fn a_worker_that_provenance_refuses_is_worded_by_what_is_wrong_with_it_and_the_remedy() {
    run(async {
        // The refusals of step 4.10's installed-path rows, as the host words them (code review P3-11): a missing or
        // mismatched worker is `Unavailable`, a file that is not the worker `Protocol`. No token was made.
        for (kind, start) in [
            (
                gwz_sspi::ErrorKind::WorkerUnavailable,
                "GWZ could not use its Windows sign-in worker",
            ),
            (
                gwz_sspi::ErrorKind::WorkerMismatch,
                "GWZ could not use its Windows sign-in worker",
            ),
            (
                gwz_sspi::ErrorKind::Protocol,
                "GWZ's Windows sign-in worker did not answer as the one installed with gwz does",
            ),
        ] {
            let challenger = Challenger::new("NTLM").await;
            let (endpoint, result) = open(
                &challenger,
                None,
                AuthPolicy::WindowsDefault,
                Some(NativeCaller::refused_for_test(kind)),
            )
            .await;
            let failure = result
                .as_ref()
                .err()
                .expect("a refused worker ends the open");
            let said = words(failure, AuthPolicy::WindowsDefault);
            assert!(said.starts_with(start), "{kind:?}: {said}");
            assert!(said.contains("No credential was sent."), "{kind:?}: {said}");
            assert!(said.contains("Reinstall gwz"), "{kind:?}: {said}");
            assert!(said.contains("--transport native"), "{kind:?}: {said}");
            // The server saw the anonymous request that drew the challenge, and nothing after it.
            assert_eq!(challenger.requests.load(Ordering::Acquire), 1, "{kind:?}");
            assert!(challenger.authorized.lock().unwrap().is_empty(), "{kind:?}");
            finish(endpoint, result).await;
        }
    });
}

#[test]
fn a_scheme_nobody_answers_is_refused_naming_it() {
    run(async {
        let challenger = Challenger::new("Bearer realm=x").await;
        let (endpoint, result) = open(&challenger, None, AuthPolicy::WindowsConfigured, None).await;
        let failure = result.as_ref().err().unwrap();
        assert_eq!(failure.code, ErrorCode::Authentication);
        let names = failure
            .detail
            .as_ref()
            .and_then(|detail| detail.schemes.clone());
        assert_eq!(names, Some(vec!["Bearer".to_owned()]));
        let said = words(failure, AuthPolicy::WindowsConfigured);
        assert!(
            said.contains("gwz answers Basic, and on Windows NTLM and Negotiate too."),
            "{said}"
        );
        finish(endpoint, result).await;
    });
}

#[test]
fn a_rejected_logon_identity_ends_the_operation_with_no_replay_and_no_helper() {
    run(async {
        use crate::git::endpoint::https_fixture::{Server, response};
        for policy in [AuthPolicy::WindowsConfigured, AuthPolicy::WindowsDefault] {
            let requests = Arc::new(AtomicUsize::new(0));
            let seen = requests.clone();
            // The server challenges with NTLM whatever the request carries: it rejects the logon identity.
            let server = Server::start(Arc::new(move |_| {
                seen.fetch_add(1, Ordering::Relaxed);
                Box::pin(async {
                    let mut result =
                        response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result
                        .headers_mut()
                        .insert(WWW_AUTHENTICATE, "NTLM, Basic realm=x".parse().unwrap());
                    result
                })
            }))
            .await;
            let challenger = Challenger {
                server,
                requests: requests.clone(),
                authorized: Arc::new(Mutex::new(Vec::new())),
            };
            let helper = Helper::new(true);
            let (caller, port) = fake(true, 2);
            let (endpoint, result) = open(&challenger, Some(&helper), policy, Some(caller)).await;
            let failure = result.as_ref().err().unwrap();
            assert_eq!(failure.code, ErrorCode::Authentication, "{policy:?}");
            let facts = failure.facts.as_ref().unwrap();
            assert_eq!(facts.method, AuthMethod::Sspi, "{policy:?}");
            assert_eq!(facts.authenticated, Some(false), "{policy:?}");
            assert_eq!(
                facts.native.as_ref().unwrap().scheme,
                NativeScheme::Ntlm,
                "{policy:?}"
            );
            assert_eq!(helper.asked(), 0, "{policy:?}");
            assert_eq!(port.starts.load(Ordering::Acquire), 1, "{policy:?}");
            assert_eq!(
                requests.load(Ordering::Acquire),
                2,
                "one challenge and one rejected answer: {policy:?}"
            );
            // T3: the host's words name the host and the scheme, and the next action.
            assert_eq!(
                words(failure, policy),
                "git.example.test rejected your Windows account (NTLM). GWZ tries nothing else. Ask the server's administrator for access for this account, or use an SSH remote."
            );
            finish(endpoint, result).await;
        }
    });
}

#[test]
fn a_challenged_post_is_refused_and_its_body_is_sent_once() {
    run(async {
        use crate::git::endpoint::https_fixture::{Server, attach, input, response};
        for (policy, offer) in [
            (AuthPolicy::WindowsConfigured, "Basic realm=x"),
            (AuthPolicy::WindowsConfigured, "NTLM"),
            (AuthPolicy::WindowsDefault, "Negotiate"),
        ] {
            let posts = Arc::new(AtomicUsize::new(0));
            let seen = posts.clone();
            // Discovery goes through anonymously; the POST is what the server challenges (TD4).
            let server = Server::start(Arc::new(move |request| {
                let post = request.method() == "POST";
                if post {
                    seen.fetch_add(1, Ordering::Relaxed);
                }
                Box::pin(async move {
                    let mut result = response(
                        if post { 401 } else { 200 },
                        if post {
                            GitService::UploadPackExchange
                        } else {
                            GitService::UploadPackAdvertisement
                        },
                        Bytes::new(),
                    );
                    if post {
                        result
                            .headers_mut()
                            .insert(WWW_AUTHENTICATE, offer.parse().unwrap());
                    }
                    result
                })
            }))
            .await;
            let helper = Helper::new(true);
            let mut endpoint = Endpoint::new(
                server.config(),
                Some(helper.config.clone()),
                pool::Config::default(),
            )
            .unwrap();
            let (caller, port) = fake(true, 2);
            endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement);
            request.policy = policy;
            let advertised = endpoint
                .client
                .prepare_attempt(
                    request.clone(),
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await
                .0
                .unwrap();
            let (stream, task) = attach(advertised);
            stream.end_write().await.unwrap();
            let mut bytes = [0; 1];
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 0);
            stream.close().await.unwrap();
            task.await.unwrap();
            request.service = GitService::UploadPackExchange;
            let posted = endpoint
                .client
                .prepare_attempt(
                    request,
                    &CancellationToken::new(),
                    &mut endpoint.client.budget(),
                    &mut None,
                )
                .await
                .0;
            let refused = match posted {
                Err(failure) => failure.code,
                Ok(prepared) => {
                    let (stream, task) = attach(prepared);
                    stream.end_write().await.unwrap();
                    let refused = stream
                        .read(&mut bytes)
                        .await
                        .err()
                        .expect("the challenged POST is refused");
                    let _ = stream.close().await;
                    let _ = task.await;
                    match refused {
                        gwz_transport::stream::Error::PeerFailed { code, .. } => code,
                        other => panic!("the POST failed another way: {other:?}"),
                    }
                }
            };
            assert_eq!(refused, ErrorCode::Authentication, "{policy:?} {offer}");
            assert_eq!(
                posts.load(Ordering::Acquire),
                1,
                "the body was replayed: {policy:?} {offer}"
            );
            assert_eq!(helper.asked(), 0, "{policy:?} {offer}");
            assert_eq!(port.starts.load(Ordering::Acquire), 0, "{policy:?} {offer}");
            endpoint.client.finish_operation("operation");
            let _ = endpoint.client.reap_cleanup(Duration::from_secs(2)).await;
            let _ = endpoint.shutdown(Duration::from_secs(2)).await;
        }
    });
}
