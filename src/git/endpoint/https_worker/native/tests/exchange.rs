//! Endpoint exchanges: deadlines, identity sources, rounds, publication and Basic fallback.
cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    use super::*;
    #[test]
    fn production_pool_wait_cannot_extend_fixed_deadline() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let server = Server::start(Arc::new(|_| Box::pin(async { response(200, GitService::UploadPackAdvertisement, Bytes::new()) }))).await;
            let endpoint = Endpoint::new(server.config(), None, pool::Config { total: 1, per_host: 1, per_user_host: 1,
                allocation_timeout_ms: 500, connect_timeout_ms: 500, ..Default::default() }).unwrap();
            let (held, _) = endpoint.client.prepare_attempt(input(&server, GitService::UploadPackAdvertisement), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let held = held.unwrap();
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let started = Instant::now(); let mut budget = endpoint.client.budget(); budget.connect = Some(Duration::from_millis(30));
            let (result, connect) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
            let error = result.err().unwrap(); assert_eq!(error.code, ErrorCode::Timeout); assert!(error.setup_cause.is_none());
            assert_eq!(connect, FirstConnect::None); assert!(started.elapsed() < Duration::from_millis(200)); drop(held);
        });
    }
    #[test]
    fn production_source_matrix_uses_current_logon_only_when_no_identity() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for (offer, policy, answer, source) in [
                ("Negotiate", AuthPolicy::WindowsConfigured, "exit 17", NativeSource::CurrentLogon),
                ("Basic realm=x, NTLM, Negotiate", AuthPolicy::WindowsDefault, "exit 17", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=fixture@example.test\npassword=fixture\n\n'", NativeSource::Configured),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=\npassword=fixture\n\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf '\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "NO_EXECUTABLE", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "exit 17", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=a\nusername=b\npassword=x\n\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "UNSTARTABLE", NativeSource::CurrentLogon),
            ] {
                let server = Server::start(Arc::new(move |request| Box::pin(async move {
                    let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, offer.parse().unwrap()); } result
                }))).await;
                let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
                crate::git::endpoint::helper_script::write_git_fixture(&helper, answer);
                if answer == "UNSTARTABLE" { std::fs::write(&helper, b"invalid executable fixture").unwrap(); }
                let auth = (answer != "NO_EXECUTABLE").then_some(https_auth::Config { executable: helper, environment: Vec::new() });
                let mut endpoint = Endpoint::new(server.config(), auth, pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = policy;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.native.as_ref().unwrap().source, source);
                assert_eq!(port.starts.load(Ordering::Acquire), 1); drop(prepared); endpoint.client.finish_operation("operation");
                assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
        });
    }
    #[test]
    fn production_round_cap_and_http_allowance_do_not_reset_native_deadline() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for (io, delay, expected) in [(9000, 0, ErrorCode::Protocol), (60, 40, ErrorCode::Timeout)] {
                let server = Server::start(Arc::new(move |request| Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result.headers_mut().insert(WWW_AUTHENTICATE, if request.headers().contains_key(AUTHORIZATION) { "NTLM YQ==" } else { "NTLM" }.parse().unwrap()); result
                }))).await;
                let mut endpoint = Endpoint::new_with_io_timeout(server.config(), None, pool::Config::default(), io).unwrap();
                let (caller, port) = fake(false, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let mut budget = endpoint.client.budget();
                let (result, connect) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
                let error = result.err().unwrap(); assert_eq!(error.code, expected); assert!(error.setup_cause.is_none()); assert_eq!(connect, FirstConnect::Connected);
                let facts = error.facts.unwrap(); assert!(facts.native.unwrap().authoritative); assert!(facts.credential_offered);
                assert_eq!(port.deadlines.lock().unwrap().as_slice(), &[budget.logical_deadline.unwrap()]);
                assert_eq!(port.starts.load(Ordering::Acquire), 1);
                if expected == ErrorCode::Protocol { assert_eq!(port.steps.load(Ordering::Acquire), 8); }
            }
        });
    }
    #[test]
    fn production_native_exchange_keeps_facts_and_refuses_replacement_before_post() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            for replace in [false, true] {
                let posts = Arc::new(AtomicUsize::new(0)); let observed = posts.clone();
                let server = Server::start(Arc::new(move |request| {
                    let posts = observed.clone(); Box::pin(async move {
                        let post = request.method() == "POST";
                        if post { posts.fetch_add(1, Ordering::Relaxed); }
                        let mut result = response(if post || request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 },
                            if post { GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement }, Bytes::new());
                        if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } result
                    })
                })).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let (caller, _) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let (prepared, _) = endpoint.client.prepare_attempt(request.clone(), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); let auth = prepared.native_route.clone().unwrap();
                let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 1];
                assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                if replace { auth.revoke(); }
                request.service = GitService::UploadPackExchange;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                if replace {
                    assert_eq!(prepared.err().unwrap().code, ErrorCode::Authentication); assert_eq!(posts.load(Ordering::Acquire), 0);
                } else {
                    let prepared = prepared.unwrap(); assert!(prepared.opened.facts.credential_offered); assert_eq!(prepared.opened.facts.authenticated, Some(true));
                    let (stream, task) = attach(prepared); stream.end_write().await.unwrap();
                    assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                    assert_eq!(posts.load(Ordering::Acquire), 1); assert_eq!(server.connections.load(Ordering::Acquire), 1);
                }
            }
        });
    }
    #[test]
    fn production_final_remote_token_completes_without_extra_request() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            let requests = Arc::new(AtomicUsize::new(0)); let observed = requests.clone();
            let server = Server::start(Arc::new(move |request| {
                observed.fetch_add(1, Ordering::Relaxed); Box::pin(async move {
                    let authorized = request.headers().contains_key(AUTHORIZATION);
                    let mut result = response(if authorized { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::from_static(b"accepted"));
                    result.headers_mut().insert(WWW_AUTHENTICATE, if authorized { "NTLM YQ==" } else { "NTLM" }.parse().unwrap()); result
                })
            })).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, port) = fake(false, 4); endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.authenticated, Some(true));
            assert_eq!(port.steps.load(Ordering::Acquire), 2); assert_eq!(requests.load(Ordering::Acquire), 2);
            let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 8];
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 8); assert_eq!(&bytes, b"accepted");
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
        });
    }
    #[test]
    fn production_helper_crossing_deadline_or_cancel_never_falls_back() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for cancel_request in [false, true] {
                let server = Server::start(Arc::new(|_| Box::pin(async {
                    let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); result
                }))).await;
                let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git"); let entered = dir.path().join("entered");
                let script = format!("touch '{}'; sleep 1; printf 'username=fixture\npassword=fixture\n\n'", entered.display());
                crate::git::endpoint::helper_script::write_git_fixture(&helper, &script);
                let mut endpoint = Endpoint::new(server.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsConfigured;
                let client = endpoint.client.clone(); let cancel = CancellationToken::new(); let signal = cancel.clone();
                let task = tokio::spawn(async move {
                    let mut budget = client.budget(); budget.connect = Some(Duration::from_millis(if cancel_request { 1000 } else { 250 }));
                    client.prepare_attempt(request, &cancel, &mut budget, &mut None).await
                });
                while !entered.exists() { tokio::time::sleep(Duration::from_millis(1)).await; }
                if cancel_request { signal.cancel(); }
                let (result, connect) = task.await.unwrap(); let error = result.err().unwrap();
                assert_eq!(error.code, if cancel_request { ErrorCode::Cancelled } else { ErrorCode::Timeout });
                assert_eq!(connect, FirstConnect::Connected); assert_eq!(port.starts.load(Ordering::Acquire), 0);
                endpoint.client.finish_operation("operation"); assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
        });
    }
    #[test]
    fn production_configured_basic_redirect_requeries_without_forwarding_credentials() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let arrivals = Arc::new(AtomicUsize::new(0)); let seen = arrivals.clone();
            let target = Server::start(Arc::new(move |request| {
                seen.fetch_add(1, Ordering::Relaxed);
                let authorized = request.headers().contains_key(AUTHORIZATION);
                if seen.load(Ordering::Relaxed) == 1 { assert!(!authorized); }
                Box::pin(async move {
                    let mut result = response(if authorized { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if !authorized { result.headers_mut().insert(WWW_AUTHENTICATE, "Basic realm=x".parse().unwrap()); }
                    result
                })
            })).await;
            let location = format!("{}/info/refs", target.url);
            let source = Server::start(Arc::new(move |request| {
                let location = location.clone(); Box::pin(async move {
                    let authorized = request.headers().contains_key(AUTHORIZATION);
                    let mut result = response(if authorized { 302 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if authorized { result.headers_mut().insert(hyper::header::LOCATION, location.parse().unwrap()); }
                    else { result.headers_mut().insert(WWW_AUTHENTICATE, "Basic realm=x".parse().unwrap()); }
                    result
                })
            })).await;
            let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
            crate::git::endpoint::helper_script::write_git_fixture(&helper, "printf 'username=fixture\npassword=fixture\n\n'");
            let endpoint = Endpoint::new(source.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
            let mut request = input(&source, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsConfigured;
            let mut budget = endpoint.client.budget();
            let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
            let prepared = prepared.unwrap(); assert!(prepared.opened.facts.credential_offered);
            assert_eq!(prepared.opened.facts.authenticated, None); assert!(prepared.opened.facts.native.is_none());
            assert_eq!(arrivals.load(Ordering::Acquire), 2); assert_eq!(budget.redirect_hops, 1);
        });
    }
    #[test]
    fn production_anonymous_advertisement_and_post_do_not_require_native_availability() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            for policy in [AuthPolicy::WindowsDefault, AuthPolicy::WindowsConfigured] {
                let server = Server::start(Arc::new(|request| Box::pin(async move {
                    assert!(!request.headers().contains_key(AUTHORIZATION));
                    response(200, if request.method() == "POST" { GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement }, Bytes::new())
                }))).await;
                let endpoint = Endpoint::new(server.config(), None, pool::Config { connect_timeout_ms: 0, ..Default::default() }).unwrap();
                for service in [GitService::UploadPackAdvertisement, GitService::UploadPackExchange] {
                    let mut request = input(&server, service); request.policy = policy;
                    let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                    let prepared = prepared.unwrap(); assert!(prepared.opened.facts.native.is_none()); assert!(!prepared.opened.facts.credential_offered);
                    let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 1];
                    assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                }
            }
        });
    }
    #[test]
    fn production_configured_basic_stays_usable_without_native_availability() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            let post = Arc::new(AtomicUsize::new(0)); let observed = post.clone();
            let server = Server::start(Arc::new(move |request| {
                let post = observed.clone(); Box::pin(async move {
                    let service = if request.method() == "POST" { post.fetch_add(1, Ordering::Relaxed); GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement };
                    response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, service, Bytes::new())
                })
            })).await;
            let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
            crate::git::endpoint::helper_script::write_git_fixture(&helper, "printf 'username=fixture\\npassword=fixture\\n\\n'");
            let endpoint = Endpoint::new(server.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
            for service in [GitService::UploadPackAdvertisement, GitService::UploadPackExchange] {
                let mut request = input(&server, service); request.policy = AuthPolicy::WindowsConfigured;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.method, AuthMethod::Gh);
                let (stream, task) = attach(prepared); stream.end_write().await.unwrap();
                let mut bytes = [0; 1]; assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
            }
            assert_eq!(post.load(Ordering::Acquire), 1);
        });
    }

    #[test]
    fn production_initial_nonempty_and_zero_refuse_without_publication() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input};
            for (timeout, token) in [(30000, "Negotiate c3ludGhldGlj"), (0, "Negotiate")] {
                let server = Server::start(Arc::new(move |_| Box::pin(async move {
                    hyper::Response::builder().status(401).header("WWW-Authenticate", token)
                        .body(http_body_util::Full::new(Bytes::new())).unwrap()
                }))).await;
                let endpoint = Endpoint::new(server.config(), None, pool::Config { connect_timeout_ms: timeout, ..Default::default() }).unwrap();
                let mut input = input(&server, GitService::UploadPackAdvertisement);
                input.policy = AuthPolicy::WindowsDefault;
                let mut budget = endpoint.client.budget();
                let (result, _) = endpoint.client.prepare_attempt(input, &CancellationToken::new(), &mut budget, &mut None).await;
                let failed = result.err().expect("native refused");
                assert_eq!(failed.code, ErrorCode::UnsupportedOperation);
                let facts = failed.facts.unwrap();
                assert_eq!(facts.native.unwrap().observation, NativeObservation::NotStarted);
                assert!(!facts.credential_offered);
            }
        });
    }
    #[test]
    fn production_drain_refusal_never_claims_publication() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let sends = Arc::new(AtomicUsize::new(0)); let observed = sends.clone();
            let server = Server::start(Arc::new(move |request| {
                if request.headers().contains_key(AUTHORIZATION) { observed.fetch_add(1, Ordering::Relaxed); }
                Box::pin(async { let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::from(vec![0; 65537]));
                    result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); result })
            })).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, _) = fake(true, 2); endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let (result, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let error = result.err().unwrap(); assert_eq!(error.code, ErrorCode::Authentication);
            assert!(!error.facts.unwrap().credential_offered); assert_eq!(sends.load(Ordering::Acquire), 0);
        });
    }
} }
