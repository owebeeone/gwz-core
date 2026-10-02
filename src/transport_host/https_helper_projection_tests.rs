//! Accepted TR1.6 §11 and configuration-view amendment outcome projection.
use super::https_tests::{
    commit_worktree, endpoint_home, fixture, git_http_backend, meta, repository,
};
use super::*;
use crate::{git::GitBackend, operation::NullSink};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    MissingGit,
    Interaction,
    ConfigurationSize,
}
impl Fault {
    fn config(self, root: &Path) -> Option<crate::git::endpoint::https_auth::Config> {
        use crate::git::endpoint::{helper_script, https_auth};
        match self {
            Self::MissingGit => None,
            Self::Interaction => {
                let executable = root.join("slow-git");
                helper_script::write_git_fixture(
                    &executable,
                    "cat >/dev/null\nprintf started > \"$HOME/started\"\nsleep 30\n",
                );
                Some(https_auth::Config {
                    executable,
                    environment: vec![("HOME".into(), root.as_os_str().into())],
                })
            }
            Self::ConfigurationSize => {
                let helper = root.join("helper");
                helper_script::write_helper_script(
                    &helper,
                    "printf started > \"$HOME/started\"\nprintf 'username=fixture\\npassword=fixture-answer\\n'\n",
                );
                let global = root.join("global");
                let mut source = format!(
                    "[credential]\n helper = {}\n[spike]\n large = ",
                    helper.display()
                )
                .into_bytes();
                // Escaping expands this below the adopted cap, above OS env limits.
                source.resize(source.len() + 525_000, b'\'');
                source.push(b'\n');
                std::fs::write(&global, source).unwrap();
                Some(https_auth::Config {
                    executable: "/usr/bin/git".into(),
                    environment: vec![
                        ("HOME".into(), root.as_os_str().into()),
                        ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
                        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ],
                })
            }
        }
    }
    fn public_code(self) -> crate::GwzErrorCode {
        match self {
            Self::MissingGit => crate::GwzErrorCode::ExternalToolMissing,
            Self::Interaction => crate::GwzErrorCode::CredentialHelperTimeout,
            Self::ConfigurationSize => crate::GwzErrorCode::RemoteRejected,
        }
    }
    fn message(self) -> &'static str {
        match self {
            Self::MissingGit => "HTTPS authentication needs `git` on PATH",
            Self::Interaction => "No credential helper answered within 0.5 seconds",
            Self::ConfigurationSize => {
                "The server asked for an HTTPS credential, and no credential helper gave one"
            }
        }
    }
}
fn member_meta(name: &str) -> RequestMeta {
    let mut meta = meta(name);
    meta.selection = Some(crate::Selection {
        targets: vec!["@all".into()],
        exclude_targets: vec!["@root".into()],
        ..Default::default()
    });
    meta
}

#[test]
fn real_fetch_push_and_private_clone_project_helper_outcomes_without_suppression_drift() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for fault in [
                Fault::MissingGit,
                Fault::Interaction,
                Fault::ConfigurationSize,
            ] {
                let root = tempfile::tempdir().unwrap();
                let (repository, _) = repository(root.path());
                let repository = Arc::new(repository);
                let challenge = Arc::new(AtomicBool::new(false));
                let challenged = challenge.clone();
                let server = fixture::Server::start(Arc::new(move |request| {
                    let repository = repository.clone();
                    let challenged = challenged.clone();
                    Box::pin(async move {
                        if challenged.load(Ordering::Acquire)
                            && !request.headers().contains_key("authorization")
                        {
                            return fixture::response(
                                401,
                                gwz_transport::protocol::GitService::UploadPackAdvertisement,
                                "private response sentinel",
                            );
                        }
                        git_http_backend(repository, request).await
                    })
                }))
                .await;
                let mut local = SshEndpointConfig::fixture(endpoint_home(root.path()), None);
                local.pool.interaction_timeout_ms = if matches!(fault, Fault::Interaction) {
                    500
                } else {
                    5_000
                };
                let runtime = TransportRuntime::with_https(
                    local,
                    HttpsEndpointConfig {
                        tls: server.config(),
                        auth: fault.config(root.path()),
                    },
                    HelperSlots::new(),
                )
                .unwrap();
                let workspace = root.path().join("workspace");
                std::fs::create_dir(&workspace).unwrap();
                let init_meta = meta("projection-init");
                let init_request = runtime
                    .request(init_meta.clone(), "init".into())
                    .await
                    .unwrap();
                let backend = init_request.backend().clone();
                let init_root = workspace.clone();
                let url = server.url.clone();
                tokio::task::spawn_blocking(move || {
                    crate::workspace_ops::handle_init_from_sources(
                        &backend,
                        &init_root,
                        crate::InitFromSourcesRequest {
                            meta: init_meta,
                            workspace_root: init_root.to_string_lossy().into(),
                            sources: vec![crate::SourceUrl {
                                url,
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
                assert_eq!(init_request.finish().await.pending_local_work, 0);
                commit_worktree(&workspace.join("member"), "local unpublished update\n");
                challenge.store(true, Ordering::Release);

                for push in [false, true] {
                    let operation = if push { "push" } else { "fetch" };
                    let request_meta = member_meta(operation);
                    let request = runtime
                        .request(request_meta.clone(), operation.into())
                        .await
                        .unwrap();
                    let backend = request.backend().clone();
                    let workspace = workspace.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        if push {
                            crate::workspace_ops::handle_push(
                                &backend,
                                &workspace,
                                crate::PushRequest {
                                    meta: request_meta,
                                    remote: Some("origin".into()),
                                    refspec: Some("refs/heads/main:refs/heads/main".into()),
                                    ..Default::default()
                                },
                                operation,
                            )
                            .map(|r| r.response)
                        } else {
                            crate::workspace_ops::handle_fetch(
                                &backend,
                                &workspace,
                                crate::FetchRequest { meta: request_meta },
                                operation,
                            )
                            .map(|r| r.response)
                        }
                    })
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(result.members.len(), 1, "{fault:?}/{operation}: {result:?}");
                    let error = result.members[0]
                        .error
                        .as_ref()
                        .unwrap_or_else(|| panic!("{fault:?}/{operation}: {result:?}"));
                    assert_eq!(
                        error.code,
                        fault.public_code(),
                        "{fault:?}/{operation}: {result:?}"
                    );
                    assert!(
                        error.message.contains(fault.message()),
                        "{fault:?}/{operation}: {error:?}"
                    );
                    assert!(!error.message.contains("sentinel"));
                    assert_eq!(request.finish().await.pending_local_work, 0);
                }

                let mut manifest = crate::artifact::read_manifest(&workspace).unwrap();
                manifest.members[0].private = true;
                crate::artifact::write_manifest(&workspace, &manifest).unwrap();
                std::fs::remove_dir_all(workspace.join("member")).unwrap();
                let clone_meta = member_meta("private-clone");
                let request = runtime
                    .request(clone_meta.clone(), "materialize".into())
                    .await
                    .unwrap();
                let backend = request.backend().clone();
                let clone_root = workspace.clone();
                let cloned = tokio::task::spawn_blocking(move || {
                    crate::workspace_ops::handle_materialize(
                        &backend,
                        &clone_root,
                        crate::MaterializeRequest {
                            meta: clone_meta,
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
                .unwrap();
                assert!(!workspace.join("member").exists());
                if matches!(fault, Fault::ConfigurationSize) {
                    let cloned = cloned.unwrap();
                    assert!(cloned.response.members.is_empty(), "{cloned:?}");
                    assert!(
                        !root.path().join("started").exists(),
                        "E2BIG must not execute the helper"
                    );
                } else {
                    let error = cloned.unwrap_err();
                    assert_eq!(crate::GwzErrorCode::from(error.code), fault.public_code());
                    assert!(error.message.contains(fault.message()), "{error:?}");
                }
                assert_eq!(request.finish().await.pending_local_work, 0);

                if matches!(fault, Fault::ConfigurationSize) {
                    let request = runtime
                        .request(meta("repaired-view"), "clone".into())
                        .await
                        .unwrap();
                    let backend = request.backend().clone();
                    let broken_backend = backend.clone();
                    let target = root.path().join("refused-view");
                    let broken_url = server.url.replace("/repo", "/refused/repo");
                    let refused = tokio::task::spawn_blocking(move || {
                        broken_backend.clone_repo(&broken_url, &target)
                    })
                    .await
                    .unwrap()
                    .unwrap_err();
                    assert_eq!(refused.code, crate::model::ErrorCode::RemoteRejected);
                    assert!(refused.message.contains(fault.message()));
                    assert!(!root.path().join("started").exists());
                    std::fs::write(
                        root.path().join("global"),
                        format!(
                            "[credential]\n helper = {}\n",
                            root.path().join("helper").display()
                        ),
                    )
                    .unwrap();
                    let target = root.path().join("repaired");
                    let url = server.url.clone();
                    tokio::task::spawn_blocking(move || backend.clone_repo(&url, &target))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(
                        root.path().join("started").exists(),
                        "configuration refusal creates no missing-Git latch"
                    );
                    assert_eq!(request.finish().await.pending_local_work, 0);
                }
                assert_eq!(runtime.shutdown().await.pending_local_work, 0);
            }
        });
}
