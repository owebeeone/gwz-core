//! The words around a discovery redirect (parity design section 14 item 13, step 4.9): a redirect from https to
//! http is refused with text T5, which quotes the address the server sent as untrusted text and tells the user to ask
//! the server's administrator; a followed redirect is told to the reader a command gave, as detail (`--verbose`).
use super::https_tests::{endpoint_home, fixture, meta};
use super::*;
use gwz_transport::protocol::GitService;
use std::sync::atomic::{AtomicUsize, Ordering};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Text T5 with the two addresses filled in.
fn t5(url: &str, new_url: &str) -> String {
    format!(
        "{url} redirected to \"{new_url}\", which is not https. GWZ does not follow a redirect from https to http, because the request would then travel unencrypted. The address in quotes was sent by the server: treat it as untrusted text, and do not use it. Ask the server's administrator to fix the redirect."
    )
}

fn transport(root: &std::path::Path, server: &fixture::Server) -> TransportRuntime {
    TransportRuntime::with_https(
        SshEndpointConfig::fixture(endpoint_home(root), None),
        HttpsEndpointConfig {
            tls: server.config(),
            auth: None,
        },
        HelperSlots::new(),
    )
    .unwrap()
}

#[test]
fn a_redirect_from_https_to_http_is_refused_with_text_t5() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let server = fixture::Server::start(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let mut moved = fixture::response(302, GitService::UploadPackAdvertisement, "");
                moved.headers_mut().insert(
                    "Location",
                    "http://elsewhere.example:8080/moved/info/refs?service=git-upload-pack"
                        .parse()
                        .unwrap(),
                );
                moved
            })
        }))
        .await;
        let runtime = transport(root.path(), &server);
        let request = runtime
            .request(meta("downgrade"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let url = server.url.clone();
        let message = tokio::task::spawn_blocking(move || {
            let error = context
                .open_https_recording(
                    &url,
                    GitService::UploadPackAdvertisement,
                    Some(AuthPolicy::Anonymous),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                    Arc::new(Mutex::new(None)),
                )
                .err()
                .expect("a downgrade is refused");
            let failure = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<HttpsOpenFailure>())
                .expect("the failure is the HTTPS open's");
            (failure.reason(), error.to_string())
        })
        .await
        .unwrap();
        let expected = t5(
            &server.url,
            "http://elsewhere.example:8080/moved/info/refs?service=git-upload-pack",
        );
        assert_eq!(message.0, expected);
        assert_eq!(message.1, expected);
        // The redirect was not followed, and the text advises nothing about the address it quotes.
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert!(
            !message.0.contains("set the remote") && !message.0.contains("use http"),
            "{}",
            message.0
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn a_refusal_is_worded_once_and_another_failure_of_the_operation_keeps_its_own_words() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let server = fixture::Server::start(Arc::new(|request| {
            Box::pin(async move {
                if request.uri().path().starts_with("/ok") {
                    return fixture::response(404, GitService::UploadPackAdvertisement, "");
                }
                let mut moved = fixture::response(302, GitService::UploadPackAdvertisement, "");
                moved.headers_mut().insert(
                    "Location",
                    "http://elsewhere.example/moved/info/refs".parse().unwrap(),
                );
                moved
            })
        }))
        .await;
        let runtime = transport(root.path(), &server);
        let request = runtime
            .request(meta("twice"), "fetch".into())
            .await
            .unwrap();
        let context = request.context.clone();
        let (downgraded, other) = (server.url.clone(), server.url.replace("/repo", "/ok"));
        let words = tokio::task::spawn_blocking(move || {
            let open = |url: &str| {
                let error = context
                    .open_https_recording(
                        url,
                        GitService::UploadPackAdvertisement,
                        Some(AuthPolicy::Anonymous),
                        Arc::new(|_, _| {}),
                        Arc::new(|_| {}),
                        Arc::new(Mutex::new(None)),
                    )
                    .err()
                    .unwrap();
                error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<HttpsOpenFailure>())
                    .unwrap()
                    .reason()
            };
            (open(&downgraded), open(&other))
        })
        .await
        .unwrap();
        assert!(words.0.contains("which is not https"), "{}", words.0);
        assert!(!words.1.contains("which is not https"), "{}", words.1);
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

/// Code review P3-4: the refusal belongs to the open that met it. Three members of one operation are opened at once and
/// the server answers all three together: two are redirected to http (each to a different address), and the third is
/// refused for another reason. Each of the first two is worded with its own two addresses, and the third with neither.
#[test]
fn concurrent_members_each_get_their_own_redirect_refusal_and_an_unrelated_failure_gets_none() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let arrived = Arc::new(AtomicUsize::new(0));
        let seen = arrived.clone();
        let server = fixture::Server::start(Arc::new(move |request| {
            let seen = seen.clone();
            Box::pin(async move {
                // Answer only when all three members' discoveries are in, so that the three refusals are on record
                // together before any member words its failure.
                seen.fetch_add(1, Ordering::SeqCst);
                let begun = std::time::Instant::now();
                while seen.load(Ordering::SeqCst) < 3 && begun.elapsed() < Duration::from_secs(5) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let target = match request.uri().path().split('/').nth(1) {
                    Some("one") => "http://one.example/moved/info/refs",
                    Some("two") => "http://two.example:8080/moved/info/refs",
                    _ => "ftp://three.example/moved/info/refs",
                };
                let mut moved = fixture::response(302, GitService::UploadPackAdvertisement, "");
                moved
                    .headers_mut()
                    .insert("Location", target.parse().unwrap());
                moved
            })
        }))
        .await;
        let runtime = transport(root.path(), &server);
        let request = runtime
            .request(meta("members"), "fetch".into())
            .await
            .unwrap();
        let urls: Vec<String> = ["one", "two", "three"]
            .iter()
            .map(|member| server.url.replace("/repo", &format!("/{member}/repo")))
            .collect();
        let mut opens = Vec::new();
        for url in &urls {
            let (context, url) = (request.context.clone(), url.clone());
            opens.push(tokio::task::spawn_blocking(move || {
                let error = context
                    .open_https_recording(
                        &url,
                        GitService::UploadPackAdvertisement,
                        Some(AuthPolicy::Anonymous),
                        Arc::new(|_, _| {}),
                        Arc::new(|_| {}),
                        Arc::new(Mutex::new(None)),
                    )
                    .err()
                    .expect("every member is refused");
                error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<HttpsOpenFailure>())
                    .expect("the failure is the HTTPS open's")
                    .reason()
            }));
        }
        let mut words = Vec::new();
        for open in opens {
            words.push(open.await.unwrap());
        }
        assert_eq!(
            words[0],
            t5(&urls[0], "http://one.example/moved/info/refs"),
            "{words:?}"
        );
        assert_eq!(
            words[1],
            t5(&urls[1], "http://two.example:8080/moved/info/refs"),
            "{words:?}"
        );
        assert_eq!(
            words[2], "HTTPS endpoint request failed: InvalidRequest",
            "{words:?}"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn a_followed_redirect_is_told_to_the_reader_as_detail_and_only_to_a_reader_that_asked_for_detail()
{
    for detail in [false, true] {
        runtime().block_on(async {
            let root = tempfile::tempdir().unwrap();
            let server = fixture::Server::start(Arc::new(|request| {
                Box::pin(async move {
                    if request.uri().path() == "/repo/info/refs" {
                        let mut moved =
                            fixture::response(302, GitService::UploadPackAdvertisement, "");
                        moved.headers_mut().insert(
                            "Location",
                            "/moved/info/refs?service=git-upload-pack".parse().unwrap(),
                        );
                        moved
                    } else {
                        fixture::response(200, GitService::UploadPackAdvertisement, "ok")
                    }
                })
            }))
            .await;
            let runtime = transport(root.path(), &server);
            let lines = Arc::new(Mutex::new(Vec::<String>::new()));
            let kept = lines.clone();
            runtime.set_notes(
                &NoteSink::new(move |line| kept.lock().unwrap().push(line.to_owned()))
                    .with_detail(detail),
            );
            let request = runtime
                .request(meta("followed"), "fetch".into())
                .await
                .unwrap();
            let context = request.context.clone();
            let url = server.url.clone();
            tokio::task::spawn_blocking(move || {
                let stream = context
                    .open_https_recording(
                        &url,
                        GitService::UploadPackAdvertisement,
                        Some(AuthPolicy::Anonymous),
                        Arc::new(|_, _| {}),
                        Arc::new(|_| {}),
                        Arc::new(Mutex::new(None)),
                    )
                    .unwrap();
                let mut rpc = crate::git::endpoint::https_remote::RpcIo::new(stream, true);
                let mut body = Vec::new();
                std::io::Read::read_to_end(&mut rpc, &mut body).unwrap();
            })
            .await
            .unwrap();
            let begun = std::time::Instant::now();
            while detail
                && lines.lock().unwrap().is_empty()
                && begun.elapsed() < Duration::from_secs(5)
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            // A pass of the endpoint's loop is a few milliseconds; give a reader that asked for nothing the same.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let told = lines.lock().unwrap().clone();
            if detail {
                assert_eq!(
                    told,
                    [format!(
                        "note: {} redirected to {}; following it",
                        server.url,
                        server.url.replace("/repo", "/moved")
                    )]
                );
            } else {
                assert!(told.is_empty(), "{told:?}");
            }
            assert_eq!(request.finish().await.pending_local_work, 0);
            assert_eq!(runtime.shutdown().await.pending_local_work, 0);
        });
    }
}
