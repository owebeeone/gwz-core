use super::*;

#[test]
fn remediation_disabled_backend_cannot_lookup_or_reuse_helper_authenticated_ssh() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for order in [[false, true, false, true], [true, false, true, false]] {
                let root = tempfile::tempdir().unwrap();
                let (bare, _) = repository(root.path());
                let server = PasswordSshd::start(
                    &root.path().join("server"),
                    "fixture-answer",
                    &["password"],
                    &[],
                );
                let home = endpoint_home(root.path());
                std::fs::write(
                    home.join(".ssh/known_hosts"),
                    server.known_host("127.0.0.1"),
                )
                .unwrap();
                let marker = root.path().join("lookups");
                let executable = root.path().join("git");
                helper_script::write_git_fixture(
                    &executable,
                    "printf x >> \"$MARKER\"\nprintf 'username=git\\npassword=fixture-answer\\n'",
                );
                let runtime = TransportRuntime::with_https(
                    SshEndpointConfig::fixture(home, None),
                    HttpsEndpointConfig {
                        tls: Default::default(),
                        auth: Some(https_auth::Config {
                            executable,
                            environment: vec![("MARKER".into(), marker.as_os_str().into())],
                        }),
                    },
                    HelperSlots::new(),
                )
                .unwrap();
                let url = format!("ssh://git@127.0.0.1:{}{}", server.port, bare.display());
                for (n, enabled) in order.into_iter().enumerate() {
                    let mut metadata = meta(&format!("enablement-{n}"));
                    metadata.policy = Some(crate::OperationPolicy {
                        max_retries: Some(0),
                        ..Default::default()
                    });
                    let request = runtime.request(metadata, "clone".into()).await.unwrap();
                    let backend = if enabled {
                        request.backend().clone()
                    } else {
                        crate::git::Git2Backend::without_credential_helpers()
                            .with_host_context(request.context.clone())
                    };
                    let before = std::fs::read(&marker).unwrap_or_default().len();
                    let attempts = server.attempts().len();
                    let target = root.path().join(format!("clone-{n}"));
                    let remote = url.clone();
                    let result =
                        tokio::task::spawn_blocking(move || backend.clone_repo(&remote, &target))
                            .await
                            .unwrap();
                    if enabled {
                        assert!(result.is_ok(), "enabled fixture must authenticate");
                    } else {
                        assert!(
                            result.is_err(),
                            "disabled operation reused or offered helper authentication"
                        );
                        assert_eq!(std::fs::read(&marker).unwrap_or_default().len(), before);
                        assert!(
                            !server.attempts()[attempts..]
                                .iter()
                                .any(|a| a.starts_with("password:"))
                        );
                    }
                    assert_eq!(request.finish().await.pending_local_work, 0);
                }
                assert_eq!(runtime.shutdown().await.pending_local_work, 0);
            }
        });
}

#[test]
fn disabled_operation_preserves_selected_key_and_isolates_pool_reuse() {
    use super::super::driver_tests::{block_on, common, endpoint_home, fixture_url, local_meta};
    let fixture = common::SshdFixture::new();
    let home = endpoint_home(&fixture);
    let executable = fixture.temp.path().join("git");
    let marker = fixture.temp.path().join("helper-ran");
    helper_script::write_git_fixture(&executable, "printf x > \"$MARKER\"\nexit 1");
    let runtime = TransportRuntime::with_https(
        SshEndpointConfig::fixture(home.clone(), None),
        HttpsEndpointConfig {
            tls: Default::default(),
            auth: Some(https_auth::Config {
                executable,
                environment: vec![("MARKER".into(), marker.as_os_str().into())],
            }),
        },
        HelperSlots::new(),
    )
    .unwrap();
    let mut enabled_connection = None;
    let mut disabled_connection = None;
    for (n, enabled) in [true, false, true, false].into_iter().enumerate() {
        let metadata = local_meta(&format!("key-enable-{n}"), &home);
        let request = block_on(runtime.request(metadata.clone(), "clone".into())).unwrap();
        let backend = if enabled {
            request.backend().clone()
        } else {
            crate::git::Git2Backend::without_credential_helpers()
                .with_host_context(request.context.clone())
        };
        let backend = backend
            .with_transport(fixture.temp.path(), metadata.transport.as_ref())
            .unwrap()
            .unwrap();
        backend
            .clone_repo(
                &fixture_url(&fixture),
                &fixture.temp.path().join(format!("clone-enable-{n}")),
            )
            .unwrap();
        let row = backend
            .transport_observations()
            .unwrap()
            .snapshot()
            .pop()
            .unwrap();
        let previous = if enabled {
            &mut enabled_connection
        } else {
            &mut disabled_connection
        };
        if let Some(connection) = previous {
            assert_eq!(row.connection_id.as_ref(), Some(&*connection));
            assert_eq!(row.reused, Some(true));
        } else {
            *previous = row.connection_id;
        }
        assert!(!marker.exists());
        assert_eq!(block_on(request.finish()).pending_local_work, 0);
    }
    assert_ne!(enabled_connection, disabled_connection);
    assert_eq!(block_on(runtime.shutdown()).pending_local_work, 0);
}
