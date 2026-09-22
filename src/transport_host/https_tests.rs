//! Host-scoped HTTPS coverage.
//!
//! The server below is a real smart-HTTP peer: `git http-backend` owns the
//! advertisement and pack exchange while the fixture supplies local TLS.
//! Keeping the request on the host path catches accidental native fallback and
//! makes cleanup observable at the same boundary as production callers.

use super::*;
use crate::TransportOptions;
use crate::git::GitBackend;
use crate::git::endpoint::{
    https_auth, https_connection, https_policy,
    https_worker::{Input, Prepared},
};
use crate::operation::NullSink;
use crate::workspace_ops::{handle_fetch, handle_init_from_sources, handle_tag};
use bytes::Bytes;
use gwz_transport::{
    protocol::{AuthPolicy, GitService, MessageKind},
    stream::Stream,
};
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    header::{HeaderName, HeaderValue},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, process::Command, runtime::Builder};
use tokio_util::sync::CancellationToken;

#[path = "../git/endpoint/https_fixture.rs"]
pub(super) mod fixture;

pub(super) fn endpoint_home(root: &Path) -> PathBuf {
    let home = root.join("endpoint-home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/known_hosts"), b"").unwrap();
    home
}

pub(super) fn repository_with_payload(root: &Path, payload: &[u8]) -> (PathBuf, git2::Oid) {
    let path = root.join("repo");
    let repository = git2::Repository::init_bare(&path).unwrap();
    repository
        .config()
        .unwrap()
        .set_bool("http.receivepack", true)
        .unwrap();
    let blob = repository.blob(payload).unwrap();
    let mut tree_builder = repository.treebuilder(None).unwrap();
    tree_builder.insert("payload", blob, 0o100644).unwrap();
    let tree = repository.find_tree(tree_builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let commit = repository
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "https host fixture",
            &tree,
            &[],
        )
        .unwrap();
    repository.set_head("refs/heads/main").unwrap();
    (path, commit)
}

pub(super) fn repository(root: &Path) -> (PathBuf, git2::Oid) {
    repository_with_payload(root, b"https host fixture\n")
}

fn fake_gh(root: &Path) -> https_auth::Config {
    use std::os::unix::fs::PermissionsExt;
    let executable = root.join("gh");
    std::fs::write(
        &executable,
        "#!/bin/sh\ncat >/dev/null\nprintf 'username=fixture\\npassword=fixture-token\\n\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    https_auth::Config {
        executable,
        environment: Vec::new(),
    }
}

fn commit_worktree(path: &Path, text: &str) -> git2::Oid {
    let repository = git2::Repository::open(path).unwrap();
    std::fs::write(path.join("payload"), text).unwrap();
    let mut index = repository.index().unwrap();
    index.add_path(Path::new("payload")).unwrap();
    index.write().unwrap();
    let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
    let parent = repository.head().unwrap().peel_to_commit().unwrap();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "https host update",
            &tree,
            &[&parent],
        )
        .unwrap()
}

pub(super) async fn git_http_backend(
    root: Arc<PathBuf>,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
        .to_owned();
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(error.to_string())))
                .unwrap();
        }
    };
    let project_root = root.parent().unwrap_or(root.as_ref());
    let mut child = match Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", project_root.as_os_str())
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", &path)
        .env("QUERY_STRING", &query)
        .env("REQUEST_METHOD", &method)
        .env("CONTENT_TYPE", &content_type)
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_HOST", &host)
        .env("SERVER_NAME", "localhost")
        .env("SERVER_PORT", "443")
        .env("SERVER_PROTOCOL", "HTTP/1.1")
        .env("REMOTE_ADDR", "127.0.0.1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from(error.to_string())))
                .unwrap();
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(&body).await.is_err() {
            let _ = child.kill().await;
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from_static(
                    b"git http-backend input failed",
                )))
                .unwrap();
        }
    }
    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(error) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from(error.to_string())))
                .unwrap();
        }
    };
    if !output.status.success() {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from(output.stderr)))
            .unwrap();
    }
    let Some(separator) = output
        .stdout
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
    else {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from_static(
                b"git http-backend omitted headers",
            )))
            .unwrap();
    };
    let headers = &output.stdout[..separator];
    let payload = Bytes::copy_from_slice(&output.stdout[separator + 4..]);
    let mut status = StatusCode::OK;
    let mut response = Response::builder();
    for line in headers.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Ok(line) = std::str::from_utf8(line) else {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from_static(
                    b"git http-backend emitted invalid headers",
                )))
                .unwrap();
        };
        if let Some(value) = line.strip_prefix("Status:") {
            if let Some(code) = value.split_whitespace().next() {
                if let Ok(code) = code.parse::<u16>() {
                    if let Ok(parsed) = StatusCode::from_u16(code) {
                        status = parsed;
                    }
                }
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(name) = HeaderName::from_bytes(name.trim().as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value.trim()) else {
            continue;
        };
        response = response.header(name, value);
    }
    response.status(status).body(Full::new(payload)).unwrap()
}

pub(super) fn meta(request_id: &str) -> RequestMeta {
    RequestMeta {
        request_id: request_id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(TransportPlacement::Local),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn local_https_advertisement_and_clone_use_scoped_host_runtime() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (repository, expected_commit) = repository(root.path());
        let repository = Arc::new(repository);
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move { git_http_backend(repository, request).await })
        }))
        .await;
        let local = SshEndpointConfig::fixture(endpoint_home(root.path()), None);
        let https = HttpsEndpointConfig {
            tls: server.config(),
            auth: None,
        };
        let transport = TransportRuntime::with_https(local, https).unwrap();
        let capabilities = transport
            .capabilities(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into(),
            })
            .unwrap();
        assert!(capabilities.schemes.unwrap().contains(&Scheme::Https));
        assert!(
            capabilities
                .auth_policies
                .unwrap()
                .contains(&AuthPolicy::Anonymous)
        );

        let request_meta = meta("https-local-clone");
        let request = transport
            .request(request_meta, "clone".into())
            .await
            .unwrap();
        let backend = request.backend().clone();
        let target = root.path().join("clone");
        let target_for_clone = target.clone();
        let url = server.url.clone();
        let result =
            tokio::task::spawn_blocking(move || backend.clone_repo(&url, &target_for_clone))
                .await
                .unwrap();
        let clone = result.unwrap();
        assert_eq!(clone.head.commit, Some(expected_commit.to_string()));
        assert_eq!(
            std::fs::read_to_string(target.join("payload")).unwrap(),
            "https host fixture\n"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
        assert!(server.connections.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    });
}

#[test]
fn local_https_workspace_commands_keep_the_same_host_scope() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (repository, expected_commit) = repository(root.path());
        let server_repository = git2::Repository::open(&repository).unwrap();
        server_repository
            .tag_lightweight(
                "fixture-tag",
                &server_repository
                    .find_object(expected_commit, None)
                    .unwrap(),
                false,
            )
            .unwrap();
        let repository_path = repository.clone();
        let repository = Arc::new(repository);
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move { git_http_backend(repository, request).await })
        }))
        .await;
        let local = SshEndpointConfig::fixture(endpoint_home(root.path()), None);
        let transport = TransportRuntime::with_https(
            local,
            HttpsEndpointConfig {
                tls: server.config(),
                auth: Some(fake_gh(root.path())),
            },
        )
        .unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let init_meta = meta("https-command-init");
        let init_request = transport
            .request(init_meta.clone(), "init".into())
            .await
            .unwrap();
        let init_backend = init_request.backend().clone();
        let init_url = server.url.clone();
        let init_workspace = workspace.clone();
        let init = tokio::task::spawn_blocking(move || {
            handle_init_from_sources(
                &init_backend,
                &init_workspace,
                crate::InitFromSourcesRequest {
                    meta: init_meta,
                    workspace_root: init_workspace.to_string_lossy().into_owned(),
                    sources: vec![crate::SourceUrl {
                        url: init_url,
                        path: Some("member".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                "init",
                &NullSink,
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            init.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert_eq!(
            git2::Repository::open(workspace.join("member"))
                .unwrap()
                .head()
                .unwrap()
                .target(),
            Some(expected_commit)
        );
        assert_eq!(init_request.finish().await.pending_local_work, 0);

        let fetch_meta = meta("https-command-fetch");
        let fetch_request = transport
            .request(fetch_meta.clone(), "fetch".into())
            .await
            .unwrap();
        let fetch_backend = fetch_request.backend().clone();
        let fetch_workspace = workspace.clone();
        let remote_url = server.url.clone();
        let member = workspace.join("member");
        let member_for_fetch = member.clone();
        let fetch = tokio::task::spawn_blocking(move || {
            let result = handle_fetch(
                &fetch_backend,
                &fetch_workspace,
                crate::FetchRequest { meta: fetch_meta },
                "fetch",
            )?;
            let refs = fetch_backend
                .ls_remote(&member_for_fetch, "origin")
                .expect("HTTPS advertisement after fetch");
            let remote_file = fetch_backend
                .read_remote_file(&remote_url, "HEAD", "payload")
                .expect("HTTPS remote-file clone")
                .unwrap();
            fetch_backend
                .tag_fetch(&member_for_fetch, "origin")
                .expect("HTTPS tag fetch after remote read");
            let tags = fetch_backend.tag_list(&member_for_fetch)?;
            Ok::<_, crate::model::ModelError>((result, refs, remote_file, tags))
        })
        .await
        .unwrap()
        .unwrap();
        let (fetch, refs, remote_file, tags) = fetch;
        assert!(matches!(
            fetch.response.meta.aggregate_status,
            crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
        ));
        assert!(refs.iter().any(|item| item.name == "refs/tags/fixture-tag"));
        assert_eq!(remote_file, b"https host fixture\n");
        assert!(tags.iter().any(|tag| tag == "fixture-tag"));
        assert_eq!(fetch_request.finish().await.pending_local_work, 0);

        let mut tag_meta = meta("https-command-tag-list");
        tag_meta.selection = Some(crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        });
        let tag_request = transport
            .request(tag_meta.clone(), "tag".into())
            .await
            .unwrap();
        let tag_backend = tag_request.backend().clone();
        let tag_workspace = workspace.clone();
        let tag = tokio::task::spawn_blocking(move || {
            handle_tag(
                &tag_backend,
                &tag_workspace,
                crate::TagRequest {
                    meta: tag_meta,
                    op: crate::TagOp::List,
                    remote: Some("origin".into()),
                    ..Default::default()
                },
                "tag",
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(
            tag.response.meta.aggregate_status,
            crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
        ));
        assert!(
            tag.tags
                .unwrap_or_default()
                .iter()
                .any(|entry| entry.name == "fixture-tag")
        );
        assert_eq!(tag_request.finish().await.pending_local_work, 0);

        let mut pull_meta = meta("https-command-pull-head");
        pull_meta.selection = Some(crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        });
        pull_meta.policy = Some(crate::OperationPolicy {
            sync: Some(crate::SyncBehavior::FfOnly),
            ..Default::default()
        });
        let pull_request = transport
            .request(pull_meta.clone(), "pull-head".into())
            .await
            .unwrap();
        let pull_backend = pull_request.backend().clone();
        let pull_workspace = workspace.clone();
        let pull = tokio::task::spawn_blocking(move || {
            crate::workspace_ops::handle_pull_head(
                &pull_backend,
                &pull_workspace,
                crate::PullHeadRequest {
                    meta: pull_meta,
                    ..Default::default()
                },
                "pull-head",
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(
                pull.response.meta.aggregate_status,
                crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
            ),
            "{pull:?}"
        );
        assert_eq!(pull_request.finish().await.pending_local_work, 0);

        let pushed = commit_worktree(&member, "https pushed fixture\n");
        let mut push_meta = meta("https-command-push");
        push_meta.selection = Some(crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        });
        let push_request = transport
            .request(push_meta.clone(), "push".into())
            .await
            .unwrap();
        let push_backend = push_request.backend().clone();
        let push_workspace = workspace.clone();
        let push_url = server.url.clone();
        let pushed_member = member.clone();
        let push = tokio::task::spawn_blocking(move || {
            let outcome = crate::workspace_ops::handle_push(
                &push_backend,
                &push_workspace,
                crate::PushRequest {
                    meta: push_meta,
                    remote: Some("origin".into()),
                    refspec: Some("refs/heads/main:refs/heads/main".into()),
                    ..Default::default()
                },
                "push",
            )?;
            let refs = push_backend.ls_remote(&pushed_member, "origin")?;
            assert!(refs.iter().any(
                |item| item.name == "refs/heads/main" && item.target == pushed.to_string()
            ));
            assert_eq!(
                push_backend
                    .read_remote_file(&push_url, "origin", "payload")?
                    .unwrap(),
                b"https pushed fixture\n"
            );
            Ok::<_, crate::model::ModelError>(outcome)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(
                push.response.meta.aggregate_status,
                crate::AggregateStatus::Ok | crate::AggregateStatus::Noop
            ),
            "{push:?}"
        );
        assert_eq!(
            git2::Repository::open(&repository_path)
                .unwrap()
                .find_reference("refs/heads/main")
                .unwrap()
                .target(),
            Some(pushed)
        );
        assert_eq!(push_request.finish().await.pending_local_work, 0);
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
        assert!(server.connections.load(Ordering::SeqCst) >= 4);
    });
}

fn embedded_https_init(consumer: super::message_embedding_tests::Consumer) {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let mut seed = 713u64;
        let payload: Vec<u8> = (0..262_144)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let (repository, _) = repository_with_payload(root.path(), &payload);
        let repository = Arc::new(repository);
        let server = fixture::Server::start(Arc::new(move |request| {
            let repository = repository.clone();
            Box::pin(async move { git_http_backend(repository, request).await })
        }))
        .await;
        let core_home = endpoint_home(&root.path().join("core"));
        let endpoint_home = endpoint_home(&root.path().join("endpoint"));
        let https = HttpsEndpointConfig {
            tls: server.config(),
            auth: None,
        };
        let transport = TransportRuntime::with_https(
            SshEndpointConfig::fixture(core_home, None),
            https.clone(),
        )
        .unwrap();
        let core_port = transport.install_cli().unwrap();
        let (endpoint, endpoint_port) =
            CliEndpoint::with_https(SshEndpointConfig::fixture(endpoint_home, None), https)
                .unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let consumer_id = match consumer {
            super::message_embedding_tests::Consumer::Rust => "rust",
            super::message_embedding_tests::Consumer::Python => "python",
        };
        let request = crate::InitFromSourcesRequest {
            meta: RequestMeta {
                request_id: format!("https-embedded-{consumer_id}"),
                schema_version: "gwz.protocol/v0".into(),
                transport: Some(TransportOptions {
                    placement: Some(TransportPlacement::Cli),
                    ..Default::default()
                }),
                ..Default::default()
            },
            workspace_root: workspace.to_string_lossy().into_owned(),
            sources: vec![crate::SourceUrl {
                url: server.url.clone(),
                path: Some("member".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let client = endpoint.register_request(&request.meta.request_id).unwrap();
        let link = super::message_embedding_tests::Link::new(
            core_port.clone(),
            endpoint_port,
            consumer,
            request.clone(),
        );
        let scope = Arc::new(
            transport
                .request(request.meta.clone(), "init".into())
                .await
                .unwrap(),
        );
        let server_connections = server.connections.clone();
        tokio::task::spawn_blocking(move || {
            link.hold();
            let scope_worker = scope.clone();
            let workspace_worker = workspace.clone();
            let request_worker = request.clone();
            let (result_tx, result_rx) = std::sync::mpsc::channel();
            let worker = thread::spawn(move || {
                let result = handle_init_from_sources(
                    scope_worker.backend(),
                    &workspace_worker,
                    request_worker,
                    "init",
                    &NullSink,
                );
                result_tx.send(result).unwrap();
            });
            assert!(
                result_rx.recv_timeout(Duration::from_millis(40)).is_err(),
                "operation completed while envelope delivery was paused"
            );
            link.pause.store(false, Ordering::Release);
            let deadline = Instant::now() + Duration::from_secs(10);
            while link.hold_first_data.load(Ordering::Acquire)
                || !link.paused.load(Ordering::Acquire)
            {
                assert!(Instant::now() < deadline, "first data was not held");
                thread::sleep(Duration::from_millis(1));
            }
            assert!(
                result_rx.recv_timeout(Duration::from_millis(40)).is_err(),
                "operation completed before carried data resumed"
            );
            link.pause.store(false, Ordering::Release);
            let response = result_rx
                .recv_timeout(Duration::from_secs(30))
                .expect("HTTPS embedded operation stranded")
                .unwrap();
            worker.join().unwrap();
            assert_eq!(
                response.response.meta.aggregate_status,
                crate::AggregateStatus::Ok
            );
            assert_eq!(
                std::fs::read(workspace.join("member/payload")).unwrap(),
                payload
            );
            let counts: BTreeMap<String, usize> = link.counts.lock().unwrap().clone();
            for kind in [
                MessageKind::Data,
                MessageKind::Window,
                MessageKind::EndWrite,
                MessageKind::Cancel,
            ] {
                assert!(
                    counts
                        .iter()
                        .any(|(key, count)| key.ends_with(&format!(":{kind:?}")) && *count > 0),
                    "missing {kind:?}: {counts:?}"
                );
            }
            assert_eq!(
                super::driver_tests::block_on(Arc::try_unwrap(scope).ok().unwrap().finish())
                    .pending_local_work,
                0
            );
            assert_eq!(
                super::driver_tests::block_on(client.finish()).pending_local_work,
                0
            );
            assert_eq!(
                super::driver_tests::block_on(endpoint.shutdown()).pending_local_work,
                0
            );
            drop(link);
            assert_eq!(
                super::driver_tests::block_on(transport.shutdown()).pending_local_work,
                0
            );
        })
        .await
        .unwrap();
        assert!(server_connections.load(Ordering::SeqCst) >= 2);
    });
}

#[test]
fn rust_https_embedded_messages_round_trip_live_git_exchange() {
    embedded_https_init(super::message_embedding_tests::Consumer::Rust);
}

#[test]
fn python_https_embedded_messages_round_trip_live_git_exchange() {
    embedded_https_init(super::message_embedding_tests::Consumer::Python);
}
