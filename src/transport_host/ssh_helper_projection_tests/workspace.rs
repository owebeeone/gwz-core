use super::*;
use crate::operation::NullSink;
use std::path::Path;

pub(super) async fn check(
    runtime: &TransportRuntime,
    root: &Path,
    bare: &Path,
    url: &str,
    fault: Fault,
) {
    let workspace = root.join("workspace");
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
    manifest.members[0].remotes[0].url = url.into();
    manifest.members[0].private = true;
    crate::artifact::write_manifest(&workspace, &manifest).unwrap();
    git2::Repository::open(workspace.join("member"))
        .unwrap()
        .remote_set_url("origin", url)
        .unwrap();
    super::super::https_tests::commit_worktree(&workspace.join("member"), "unpublished update\n");
    let expected = match fault {
        Fault::MissingGit | Fault::UnexecutableGit => crate::GwzErrorCode::ExternalToolMissing,
        _ => crate::GwzErrorCode::CredentialHelperTimeout,
    };
    for operation in ["fetch", "push", "materialize"] {
        if operation == "materialize" {
            std::fs::remove_dir_all(workspace.join("member")).unwrap();
        }
        let mut request_meta = meta(&format!("public-{operation}"));
        request_meta.policy = Some(crate::OperationPolicy {
            max_retries: Some(0),
            ..Default::default()
        });
        request_meta.selection = Some(crate::Selection {
            targets: vec!["@all".into()],
            exclude_targets: vec!["@root".into()],
            ..Default::default()
        });
        let request = runtime
            .request(request_meta.clone(), operation.into())
            .await
            .unwrap();
        let backend = request.backend().clone();
        let workspace = workspace.clone();
        let result = tokio::task::spawn_blocking(move || match operation {
            "fetch" => crate::workspace_ops::handle_fetch(
                &backend,
                &workspace,
                crate::FetchRequest { meta: request_meta },
                operation,
            )
            .map(|r| r.response),
            "push" => crate::workspace_ops::handle_push(
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
            .map(|r| r.response),
            _ => crate::workspace_ops::handle_materialize(
                &backend,
                &workspace,
                crate::MaterializeRequest {
                    meta: request_meta,
                    target: crate::MaterializeTarget {
                        kind: crate::MaterializeTargetKind::Lock,
                        ..Default::default()
                    },
                },
                operation,
                &NullSink,
            )
            .map(|r| r.response),
        })
        .await
        .unwrap();
        let message = if operation == "materialize" {
            let error = result.unwrap_err();
            assert_eq!(crate::GwzErrorCode::from(error.code), expected);
            error.message
        } else {
            let result = result.unwrap();
            assert_eq!(result.members.len(), 1);
            let error = result.members[0].error.as_ref().unwrap();
            assert_eq!(error.code, expected);
            error.message.clone()
        };
        assert!(!message.contains("(attempt "));
        assert!(!message.contains("sentinel"));
        assert_eq!(request.finish().await.pending_local_work, 0);
    }
    assert!(
        !workspace.join("member").exists(),
        "private clone M1/75 must not be quietly suppressed"
    );
}
