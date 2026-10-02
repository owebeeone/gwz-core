use super::https_tests::{endpoint_home, fixture, meta, repository};
use super::*;
use crate::{git::endpoint::helper_script, operation::NullSink};

#[test]
fn negotiate_after_four_schemes_is_loud_for_real_private_materialize_without_helper() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = tempfile::tempdir().unwrap();
            let (bare, _) = repository(root.path());
            let server = fixture::Server::start(Arc::new(|request| {
                Box::pin(async move {
                    assert!(request.headers().get("Authorization").is_none());
                    hyper::Response::builder()
                        .status(401)
                        .header(
                            "WWW-Authenticate",
                            "One, Two, Three, Four, nEgOtIaTe realm=\"realm-sentinel\"",
                        )
                        .body(http_body_util::Full::new(bytes::Bytes::new()))
                        .unwrap()
                })
            }))
            .await;
            let marker = root.path().join("helper-ran");
            let executable = root.path().join("git");
            helper_script::write_git_fixture(&executable, "printf x > \"$MARKER\"\nexit 1");
            let runtime = TransportRuntime::with_https(
                SshEndpointConfig::fixture(endpoint_home(root.path()), None),
                HttpsEndpointConfig {
                    tls: server.config(),
                    auth: Some(crate::git::endpoint::https_auth::Config {
                        executable,
                        environment: vec![("MARKER".into(), marker.as_os_str().into())],
                    }),
                },
                HelperSlots::new(),
            )
            .unwrap();
            let workspace = root.path().join("workspace");
            std::fs::create_dir(&workspace).unwrap();
            let initial = workspace.clone();
            let source = bare.to_string_lossy().into_owned();
            tokio::task::spawn_blocking(move || {
                crate::workspace_ops::handle_init_from_sources(
                    &crate::git::Git2Backend::new(),
                    &initial,
                    crate::InitFromSourcesRequest {
                        meta: meta("init"),
                        workspace_root: initial.to_string_lossy().into(),
                        sources: vec![crate::SourceUrl {
                            url: source,
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
            let mut manifest = crate::artifact::read_manifest(&workspace).unwrap();
            manifest.members[0].private = true;
            manifest.members[0].remotes[0].url = server.url.clone();
            crate::artifact::write_manifest(&workspace, &manifest).unwrap();
            std::fs::remove_dir_all(workspace.join("member")).unwrap();
            let mut metadata = meta("private-negotiate");
            metadata.policy = Some(crate::OperationPolicy {
                max_retries: Some(0),
                ..Default::default()
            });
            let request = runtime
                .request(metadata.clone(), "materialize".into())
                .await
                .unwrap();
            let backend = request.backend().clone();
            let operation_root = workspace.clone();
            let error = tokio::task::spawn_blocking(move || {
                crate::workspace_ops::handle_materialize(
                    &backend,
                    &operation_root,
                    crate::MaterializeRequest {
                        meta: metadata,
                        target: crate::MaterializeTarget {
                            kind: crate::MaterializeTargetKind::Lock,
                            ..Default::default()
                        },
                    },
                    "materialize",
                    &NullSink,
                )
            })
            .await
            .unwrap()
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::GitCommandFailed);
            assert!(!error.message.contains("sentinel"));
            assert!(!marker.exists());
            assert!(!workspace.join("member").exists());
            assert_eq!(request.finish().await.pending_local_work, 0);
            assert_eq!(runtime.shutdown().await.pending_local_work, 0);
        });
}
