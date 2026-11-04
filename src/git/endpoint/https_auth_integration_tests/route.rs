use super::*;
use crate::git::endpoint::helper_script::{real_git, script_path, write_helper_script};
use hyper::header::{LOCATION, WWW_AUTHENTICATE};
use std::path::PathBuf;
#[test]
fn gh_redirect_looks_up_each_origin_and_never_reuses_auth_header() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let seen = Arc::new(Mutex::new(Vec::<(String, Option<Vec<u8>>)>::new()));
            let seen_target = seen.clone();
            let target = Server::start(Arc::new(move |request| {
                let seen_target = seen_target.clone();
                Box::pin(async move {
                    let authorization = request
                        .headers()
                        .get(AUTHORIZATION)
                        .map(|value| value.as_bytes().to_vec());
                    seen_target
                        .lock()
                        .unwrap()
                        .push((request.uri().to_string(), authorization.clone()));
                    if authorization.is_some() {
                        response(200, GitService::UploadPackAdvertisement, "target")
                    } else {
                        response(401, GitService::UploadPackAdvertisement, "challenge")
                    }
                })
            }))
            .await;
            let target_url = target.url.clone();
            let source_seen = seen.clone();
            let source = Server::start(Arc::new(move |request| {
                let source_seen = source_seen.clone();
                let target_url = target_url.clone();
                Box::pin(async move {
                    let authorization = request
                        .headers()
                        .get(AUTHORIZATION)
                        .map(|value| value.as_bytes().to_vec());
                    source_seen
                        .lock()
                        .unwrap()
                        .push((request.uri().to_string(), authorization.clone()));
                    let location = format!("{target_url}/info/refs?service=git-upload-pack");
                    hyper::Response::builder()
                        .status(302)
                        .header("Location", location)
                        .body(http_body_util::Full::new(bytes::Bytes::new()))
                        .unwrap()
                })
            }))
            .await;
            let log = tempfile::NamedTempFile::new().unwrap();
            let source_host = authority(&source.url);
            let target_host = authority(&target.url);
            let (_helper_dir, auth) = recording_gh(
                log.path(),
                &source_host,
                "source-token",
                &target_host,
                "target-token",
            );
            let mut endpoint = Endpoint::new(
                source.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let mut request = input(&source, GitService::UploadPackAdvertisement);
            request.policy = AuthPolicy::Gh;
            finish(
                endpoint
                    .client
                    .prepare_budget_for_transition(
                        request,
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap(),
            )
            .await;
            let records = seen.lock().unwrap().clone();
            assert_eq!(records.len(), 3);
            assert_eq!(records[0].1, Some(auth_header("source-token")));
            assert!(records[1].1.is_none());
            assert_eq!(records[2].1, Some(auth_header("target-token")));
            assert_ne!(records[0].1, records[2].1);
            let helper_input = fs::read_to_string(log.path()).unwrap();
            assert!(helper_input.contains(&format!("url=https://{source_host}/")));
            assert!(helper_input.contains(&format!("url=https://{target_host}/")));
            assert!(helper_input.matches("/repo").count() >= 2);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn concurrent_same_operation_conflicting_redirects_fail_protocol() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let target_a = Server::start(Arc::new(|_| {
                Box::pin(async { response(200, GitService::UploadPackAdvertisement, "a") })
            }))
            .await;
            let target_b = Server::start(Arc::new(|_| {
                Box::pin(async { response(200, GitService::UploadPackAdvertisement, "b") })
            }))
            .await;
            let choices = Arc::new(AtomicUsize::new(0));
            let counter = choices.clone();
            let location_a = format!("{}/info/refs?service=git-upload-pack", target_a.url);
            let location_b = format!("{}/info/refs?service=git-upload-pack", target_b.url);
            let source = Server::start(Arc::new(move |_| {
                let counter = counter.clone();
                let location = if counter.fetch_add(1, Ordering::SeqCst) % 2 == 0 {
                    location_a.clone()
                } else {
                    location_b.clone()
                };
                Box::pin(async move {
                    hyper::Response::builder()
                        .status(302)
                        .header("Location", location)
                        .body(http_body_util::Full::new(bytes::Bytes::new()))
                        .unwrap()
                })
            }))
            .await;
            let mut endpoint = Endpoint::new(
                source.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let cancellation = CancellationToken::new();
            let (mut first_budget, mut second_budget) =
                (endpoint.client.budget(), endpoint.client.budget());
            let (mut first_challenge, mut second_challenge) = (None, None);
            let first = endpoint.client.prepare_budget_for_transition(
                input(&source, GitService::UploadPackAdvertisement),
                &cancellation,
                &mut first_budget,
                &mut first_challenge,
            );
            let second = endpoint.client.prepare_budget_for_transition(
                input(&source, GitService::UploadPackAdvertisement),
                &cancellation,
                &mut second_budget,
                &mut second_challenge,
            );
            let (first, second) = tokio::join!(first, second);
            let mut successes = 0;
            let mut protocol_failures = 0;
            for result in [first, second] {
                match result {
                    Ok(prepared) => {
                        successes += 1;
                        finish(prepared).await;
                    }
                    Err(failure) if failure.code == ErrorCode::Protocol => {
                        protocol_failures += 1;
                    }
                    Err(failure) => panic!("unexpected conflict result: {failure:?}"),
                }
            }
            assert_eq!(successes, 1);
            assert_eq!(protocol_failures, 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

// A redirect after the challenge (TR1.6 OQ4 (d), decided by the operator on 2026-11-01 from the evidence run at
// gwz-core-evidence `d52abba2`; `dev-docs/GwzTransportReleaseConvergencePlan.md` section 10 D-V): it is followed under
// discovery's rules; to the same origin (scheme, host, port) the accepted credential is resent; to another origin
// nothing is sent, and only that origin's own 401 starts a fresh lookup for its own URL; a redirect never causes a
// helper `erase`. The rows run the real Git's `credential fill` over a configured script helper, with
// `credential.useHttpPath` off and on; the helper answers origin A only for the repository it was challenged for
// when Git gives it the path, as a path-keyed store does. The rule is the same on every platform, so each row runs
// under the `gh` path (`Gh`) and the Windows configured-helper path (`WindowsConfigured`, whose Basic challenge goes
// to the helper through the native 401 handler). HTTPS model step R4, reported with sub-step H3b.
/// Where the challenged discovery of origin A is redirected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// Another repository path of origin A.
    SameOrigin,
    /// Another repository path of origin A, which refuses every credential with a Basic 401.
    SameOriginRefusing,
    /// Origin B, which serves the advertisement to anyone.
    OtherOrigin,
    /// Origin B, which challenges.
    OtherOriginChallenging,
}
/// The policy discovery and its POST run under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    /// An anonymous discovery whose 401 is replayed under `Gh`.
    Gh,
    /// `WindowsConfigured`: a Basic challenge is answered by the configured helper (`answer_basic`).
    WindowsConfigured,
}
const POLICIES: [Policy; 2] = [Policy::Gh, Policy::WindowsConfigured];

/// One request a server saw: which server, its path, and its Authorization header.
type Record = (char, String, Option<Vec<u8>>);

/// A configured helper through the real Git: its script logs each call's action and input to the returned file, and
/// answers `token-a` for `host_a` (only for the path `repo` when Git gives it a path) and `token-b` for `host_b`.
fn configured(
    directory: &Path,
    use_http_path: bool,
    host_a: &str,
    host_b: &str,
) -> (https_auth::Config, PathBuf) {
    let script = directory.join("helper");
    let log = directory.join("helper-log");
    write_helper_script(
        &script,
        "printf 'action=%s\\n' \"$1\" >> \"$HELPER_LOG\"\n\
[ \"$1\" = get ] || exit 0\n\
input=$(/bin/cat)\n\
printf '%s\\n' \"$input\" >> \"$HELPER_LOG\"\n\
case \"$input\" in\n\
  *\"host=$HOST_A\"*)\n\
    case \"$input\" in *path=*) case \"$input\" in *path=repo*) ;; *) exit 0 ;; esac ;; esac\n\
    token=token-a ;;\n\
  *\"host=$HOST_B\"*) token=token-b ;;\n\
  *) exit 0 ;;\n\
esac\n\
printf 'username=fixture\\npassword=%s\\n' \"$token\"\n",
    );
    let config = directory.join("gitconfig");
    fs::write(
        &config,
        format!(
            "[credential]\n\thelper = !{}\n\tuseHttpPath = {use_http_path}\n",
            script_path(&script)
        ),
    )
    .unwrap();
    let git = real_git();
    let mut environment = vec![
        ("HOME".into(), directory.as_os_str().into()),
        (
            "PATH".into(),
            std::env::join_paths([git.parent().unwrap()]).unwrap(),
        ),
        ("GIT_CONFIG_GLOBAL".into(), config.as_os_str().into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("HELPER_LOG".into(), script_path(&log).into()),
        ("HOST_A".into(), host_a.into()),
        ("HOST_B".into(), host_b.into()),
    ];
    // A Windows process is started with no `SystemRoot` unless it is given one.
    if let Some(root) = std::env::var_os("SystemRoot") {
        environment.push(("SystemRoot".into(), root));
    }
    (
        https_auth::Config {
            executable: git,
            environment,
        },
        log,
    )
}

/// A fixture server named `name` that records each request in `records` and answers it with `answer(path, credentialed)`.
async fn recording(
    name: char,
    records: &Arc<Mutex<Vec<Record>>>,
    answer: impl Fn(&str, bool) -> hyper::Response<http_body_util::Full<bytes::Bytes>>
    + Send
    + Sync
    + 'static,
) -> Server {
    let (records, answer) = (records.clone(), Arc::new(answer));
    Server::start(Arc::new(move |request| {
        let authorization = request
            .headers()
            .get(AUTHORIZATION)
            .map(|value| value.as_bytes().to_vec());
        let path = request.uri().path().to_owned();
        records
            .lock()
            .unwrap()
            .push((name, path.clone(), authorization.clone()));
        let answer = answer.clone();
        let body = request.into_body();
        Box::pin(async move {
            let _ = body.collect().await;
            answer(&path, authorization.is_some())
        })
    }))
    .await
}
fn challenge(service: GitService) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    let mut challenge = response(401, service, "challenge");
    challenge
        .headers_mut()
        .insert(WWW_AUTHENTICATE, "Basic realm=\"fixture\"".parse().unwrap());
    challenge
}
fn served(
    service: GitService,
    credentialed: bool,
) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    if credentialed {
        response(200, service, "served")
    } else {
        challenge(service)
    }
}

/// What one run saw: every request of both servers in order, the helper's log, and how discovery ended.
struct Run {
    records: Vec<Record>,
    log: String,
    /// Origin B's `host` as Git gives it to a helper.
    host_b: String,
    /// Discovery's outcome: `Ok` when it was served, else its failure's code.
    outcome: Result<(), ErrorCode>,
}
impl Run {
    /// The requests of server `name` to `path`.
    fn at(&self, name: char, path: &str) -> Vec<Option<Vec<u8>>> {
        self.records
            .iter()
            .filter(|(server, seen, _)| *server == name && seen == path)
            .map(|(_, _, authorization)| authorization.clone())
            .collect()
    }
    /// How often Git asked the helper for a credential; and asserts that nothing else was asked of it.
    #[track_caller]
    fn lookups(&self) -> usize {
        assert!(
            !self.log.contains("action=erase") && !self.log.contains("action=store"),
            "the helper was told to erase or store: {}",
            self.log
        );
        self.log.matches("action=get").count()
    }
}

/// Discovers origin A's `/repo` under `policy`; A challenges the anonymous request and redirects the credentialed one
/// to `target`. When discovery reaches the same origin and is served, the POST to the repository it found follows,
/// under the same policy.
fn discover(target: Target, policy: Policy, use_http_path: bool) -> Run {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let records = Arc::new(Mutex::new(Vec::new()));
            let challenging = target == Target::OtherOriginChallenging;
            let refusing = target == Target::SameOriginRefusing;
            let b = recording('B', &records, move |_, credentialed| {
                served(
                    GitService::UploadPackAdvertisement,
                    credentialed || !challenging,
                )
            })
            .await;
            let location = match target {
                Target::SameOrigin | Target::SameOriginRefusing => {
                    "/moved/info/refs?service=git-upload-pack".to_owned()
                }
                _ => format!(
                    "https://{}/elsewhere/info/refs?service=git-upload-pack",
                    authority(&b.url)
                ),
            };
            let a = recording('A', &records, move |path, credentialed| match path {
                "/repo/info/refs" if credentialed => hyper::Response::builder()
                    .status(302)
                    .header(LOCATION, location.as_str())
                    .body(http_body_util::Full::new(bytes::Bytes::new()))
                    .unwrap(),
                "/moved/info/refs" if refusing => challenge(GitService::UploadPackAdvertisement),
                "/moved/git-upload-pack" => served(GitService::UploadPackExchange, credentialed),
                _ => served(GitService::UploadPackAdvertisement, credentialed),
            })
            .await;
            let directory = tempfile::tempdir().unwrap();
            let (auth, log) = configured(
                directory.path(),
                use_http_path,
                &authority(&a.url),
                &authority(&b.url),
            );
            let mut endpoint = Endpoint::new(
                a.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let discovered = match policy {
                Policy::Gh => {
                    let mut first = None;
                    let discovered = endpoint
                        .client
                        .prepare_auto(
                            input(&a, GitService::UploadPackAdvertisement),
                            &CancellationToken::new(),
                            &mut first,
                        )
                        .await;
                    assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
                    discovered
                }
                Policy::WindowsConfigured => {
                    let mut discovery = input(&a, GitService::UploadPackAdvertisement);
                    discovery.policy = AuthPolicy::WindowsConfigured;
                    endpoint
                        .client
                        .prepare_budget_for_transition(
                            discovery,
                            &CancellationToken::new(),
                            &mut endpoint.client.budget(),
                            &mut None,
                        )
                        .await
                }
            };
            let outcome = match discovered {
                Ok(prepared) => {
                    finish(prepared).await;
                    Ok(())
                }
                Err(failure) => Err(failure.code),
            };
            if target == Target::SameOrigin && outcome.is_ok() {
                let mut post = input(&a, GitService::UploadPackExchange);
                post.policy = match policy {
                    Policy::Gh => AuthPolicy::Gh,
                    Policy::WindowsConfigured => AuthPolicy::WindowsConfigured,
                };
                let prepared = endpoint
                    .client
                    .prepare_budget_for_transition(
                        post,
                        &CancellationToken::new(),
                        &mut endpoint.client.budget(),
                        &mut None,
                    )
                    .await
                    .unwrap_or_else(|failure| {
                        panic!("the POST, {policy:?}, useHttpPath {use_http_path}: {failure:?}")
                    });
                let (stream, task) = attach(prepared);
                stream.write_all(b"0000").await.unwrap();
                stream.end_write().await.unwrap();
                let mut buffer = [0; 64];
                while stream.read(&mut buffer).await.unwrap() != 0 {}
                stream.close().await.unwrap();
                task.await.unwrap();
            }
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            Run {
                records: records.lock().unwrap().clone(),
                log: fs::read_to_string(log).unwrap_or_default(),
                host_b: authority(&b.url),
                outcome,
            }
        })
}
/// Every policy with `useHttpPath` off and on.
fn cases() -> impl Iterator<Item = (Policy, bool)> {
    POLICIES
        .into_iter()
        .flat_map(|policy| [false, true].map(move |use_http_path| (policy, use_http_path)))
}

#[test]
fn oq4_1_a_redirect_after_the_challenge_to_the_same_origin_resends_the_accepted_credential() {
    // OQ4 (d): A's credentialed discovery is redirected to `/moved` on A. The new path gets the accepted credential on
    // its first request, unasked: Git asked the helper once, for `/repo`, and never for `/moved` (which a path-keyed
    // helper could not answer). The POST to the repository discovery found carries the same credential from its first
    // request, under `WindowsConfigured` too (R4w3 Code P2-1: the resent hop's 200 marks the route Basic).
    for (policy, use_http_path) in cases() {
        let run = discover(Target::SameOrigin, policy, use_http_path);
        let case = format!("{policy:?}, useHttpPath {use_http_path}");
        assert_eq!(run.outcome, Ok(()), "{case}");
        let accepted = Some(auth_header("token-a"));
        assert_eq!(
            run.at('A', "/repo/info/refs"),
            [None, accepted.clone()],
            "{case}"
        );
        assert_eq!(
            run.at('A', "/moved/info/refs"),
            std::slice::from_ref(&accepted),
            "{case}: the same origin's new path was not sent the accepted credential first"
        );
        assert_eq!(
            run.at('A', "/moved/git-upload-pack"),
            [accepted],
            "{case}: the POST did not carry the accepted credential"
        );
        assert_eq!(run.lookups(), 1, "{case}: {}", run.log);
        assert_eq!(
            run.log.contains("path=repo"),
            use_http_path,
            "{case}: {}",
            run.log
        );
        assert!(!run.log.contains("path=moved"), "{case}: {}", run.log);
    }
}

#[test]
fn oq4_2_a_redirect_after_the_challenge_to_another_origin_sends_it_nothing() {
    // OQ4 (d): A's credentialed discovery is redirected to origin B, which serves anyone. B gets no credential, the
    // helper is not asked again, and discovery succeeds.
    for (policy, use_http_path) in cases() {
        let run = discover(Target::OtherOrigin, policy, use_http_path);
        let case = format!("{policy:?}, useHttpPath {use_http_path}");
        assert_eq!(run.outcome, Ok(()), "{case}");
        assert_eq!(
            run.at('A', "/repo/info/refs"),
            [None, Some(auth_header("token-a"))],
            "{case}"
        );
        assert_eq!(
            run.at('B', "/elsewhere/info/refs"),
            [None],
            "{case}: another origin was sent a credential"
        );
        assert_eq!(run.lookups(), 1, "{case}: {}", run.log);
    }
}

#[test]
fn oq4_3_another_origin_that_challenges_starts_a_lookup_for_its_own_url() {
    // OQ4 (d): A's credentialed discovery is redirected to origin B, which challenges. B's first request carries
    // nothing; only B's own 401 starts a lookup, for B's URL (its path given to the helper only under useHttpPath), and
    // B gets B's credential, never A's.
    for (policy, use_http_path) in cases() {
        let run = discover(Target::OtherOriginChallenging, policy, use_http_path);
        let case = format!("{policy:?}, useHttpPath {use_http_path}");
        assert_eq!(run.outcome, Ok(()), "{case}");
        assert_eq!(
            run.at('A', "/repo/info/refs"),
            [None, Some(auth_header("token-a"))],
            "{case}"
        );
        assert_eq!(
            run.at('B', "/elsewhere/info/refs"),
            [None, Some(auth_header("token-b"))],
            "{case}"
        );
        assert_eq!(run.lookups(), 2, "{case}: {}", run.log);
        let b_lookup = run.log.rsplit("action=get").next().unwrap();
        assert!(
            b_lookup.contains(&format!("host={}", run.host_b)),
            "{case}: {}",
            run.log
        );
        assert_eq!(
            b_lookup.contains("path=elsewhere"),
            use_http_path,
            "{case}: {}",
            run.log
        );
    }
}

#[test]
fn oq4_4_a_same_origin_redirect_target_that_refuses_the_credential_gets_it_once() {
    // OQ4 (d) with draft 3(d) ("`Rejected` is per operation and terminal"; R4w3 State P2-1): A's credentialed discovery
    // is redirected to `/moved` on A, which refuses every credential. `/moved` gets the accepted credential once and is
    // never sent it again: discovery ends `Authentication`, Git asked the helper once and was told nothing else (no
    // `erase`, no `store`).
    for (policy, use_http_path) in cases() {
        let run = discover(Target::SameOriginRefusing, policy, use_http_path);
        let case = format!("{policy:?}, useHttpPath {use_http_path}");
        assert_eq!(run.outcome, Err(ErrorCode::Authentication), "{case}");
        let accepted = Some(auth_header("token-a"));
        assert_eq!(
            run.at('A', "/repo/info/refs"),
            [None, accepted.clone()],
            "{case}"
        );
        assert_eq!(
            run.at('A', "/moved/info/refs"),
            [accepted],
            "{case}: the refused credential was sent again"
        );
        assert_eq!(run.lookups(), 1, "{case}: {}", run.log);
    }
}
