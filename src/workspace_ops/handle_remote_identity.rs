use std::path::Path;

use super::*;
use crate::git::{GitBackend, resolve_ssh_identity_path, validate_ssh_identity_file};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

pub fn handle_remote_identity<B: GitBackend>(
    backend: &B,
    start: &Path,
    request: crate::RemoteIdentityRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::RemoteIdentityResponse> {
    let context = OperationRequest::RemoteIdentity(request.clone()).context(operation_id)?;
    let dry = request.meta.dry_run.unwrap_or(false);
    let access = acquire_workspace_mutation_guard(
        start,
        request.meta.workspace.as_ref(),
        OpenMergeCommand::RemoteIdentity,
        dry || request.op == crate::RemoteIdentityOp::Get,
    )?;
    let root = access.root();
    let manifest = crate::artifact::read_manifest(root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    if request.remote.trim().is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "identity configuration requires a remote name",
        ));
    }
    let desired = match (request.op, request.private_key_path.as_deref()) {
        (crate::RemoteIdentityOp::Set, Some(value)) => {
            let path = resolve_ssh_identity_path(start, value)?;
            validate_ssh_identity_file(&path)?;
            Some(
                path.to_str()
                    .ok_or_else(|| {
                        ModelError::new(ErrorCode::InvalidRequest, "identity path is not UTF-8")
                    })?
                    .to_owned(),
            )
        }
        (crate::RemoteIdentityOp::Get | crate::RemoteIdentityOp::Unset, None) => None,
        _ => {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "only identity set accepts and requires a key path",
            ));
        }
    };
    let mut plans = Vec::new();
    for target in resolve_action_targets(
        &manifest,
        request.meta.selection.as_ref(),
        crate::ActionKind::RemoteIdentity,
    )? {
        let (id, path, kind) = match target {
            SelectedTarget::Root => ("@root".to_owned(), ".".to_owned(), crate::TargetKind::Root),
            SelectedTarget::Member(member) => {
                if member.source_kind != crate::artifact::ArtifactSourceKind::Git {
                    return Err(ModelError::new(
                        ErrorCode::UnsupportedSourceKind,
                        "identity configuration requires a Git repository",
                    )
                    .with_member(&member.id, &member.path));
                }
                (
                    member.id.clone(),
                    member.path.clone(),
                    crate::TargetKind::Member,
                )
            }
        };
        let repo = root.join(&path);
        let current = backend
            .remote_identity(&repo, &request.remote)
            .map_err(|error| error.with_member(&id, &path))?;
        plans.push((id, path, kind, repo, current));
    }
    let mut rows = Vec::new();
    let mut identities = Vec::new();
    for (id, path, kind, repo, current) in plans {
        let value = if request.op == crate::RemoteIdentityOp::Get {
            current.clone()
        } else {
            desired.clone()
        };
        let changed = request.op != crate::RemoteIdentityOp::Get && current != value;
        let result = if changed && !dry {
            backend.set_remote_identity(&repo, &request.remote, value.as_deref())
        } else {
            Ok(())
        };
        let status = if result.is_err() {
            crate::MemberStatus::Failed
        } else if dry && changed {
            crate::MemberStatus::Planned
        } else if changed {
            crate::MemberStatus::Ok
        } else {
            crate::MemberStatus::Noop
        };
        if result.is_ok() {
            identities.push(crate::RemoteIdentityEntry {
                member_id: id.clone(),
                member_path: path.clone(),
                remote: request.remote.clone(),
                private_key_path: value,
            });
        }
        rows.push(crate::MemberResponse {
            member_id: id.clone(),
            member_path: path.clone(),
            source_kind: crate::SourceKind::Git,
            status,
            target_kind: Some(kind),
            error: result
                .err()
                .map(|error| crate::GwzError::from(&error.with_member(id, path))),
            ..Default::default()
        });
    }
    let failed = rows
        .iter()
        .any(|row| row.status == crate::MemberStatus::Failed);
    let success = rows.iter().any(|row| row.status == crate::MemberStatus::Ok);
    let aggregate = if failed && success {
        crate::AggregateStatus::Partial
    } else if failed {
        crate::AggregateStatus::Failed
    } else {
        crate::AggregateStatus::Ok
    };
    Ok(crate::RemoteIdentityResponse {
        response: response_envelope(context, aggregate, rows),
        identities,
    })
}

#[cfg(test)]
mod tests {
    use crate::git::{Git2Backend, GitBackend};
    use crate::workspace_ops::tests::*;

    #[test]
    fn remote_identity_configuration_is_local_and_preflighted() {
        let temp = TempDir::new("remote-identity-config");
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
        backend
            .add_remote(temp.path(), "origin", "ssh://git@example.invalid/root")
            .unwrap();
        let before = std::fs::read(temp.path().join("gwz.conf/gwz.yml")).unwrap();
        std::fs::write(temp.path().join("key=one"), "fixture key path only").unwrap();
        let mut request = crate::RemoteIdentityRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@root".into()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: "origin".into(),
            op: crate::RemoteIdentityOp::Set,
            private_key_path: Some("key=one".into()),
        };
        request.meta.dry_run = Some(true);
        super::handle_remote_identity(&backend, temp.path(), request.clone(), "dry").unwrap();
        let repo = git2::Repository::open(temp.path()).unwrap();
        assert!(
            repo.config()
                .unwrap()
                .get_string("remote.origin.gwzSshIdentity")
                .is_err()
        );
        request.meta.dry_run = None;
        let response =
            super::handle_remote_identity(&backend, temp.path(), request.clone(), "set").unwrap();
        let expected = temp.path().join("key=one").to_str().unwrap().to_owned();
        assert_eq!(
            repo.config()
                .unwrap()
                .get_string("remote.origin.gwzSshIdentity")
                .unwrap(),
            expected
        );
        assert_eq!(
            response.identities[0].private_key_path.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            std::fs::read(temp.path().join("gwz.conf/gwz.yml")).unwrap(),
            before
        );
        request.private_key_path = None;
        request.op = crate::RemoteIdentityOp::Get;
        assert_eq!(
            super::handle_remote_identity(&backend, temp.path(), request.clone(), "get")
                .unwrap()
                .identities[0]
                .private_key_path
                .as_deref(),
            Some(expected.as_str())
        );
        request.op = crate::RemoteIdentityOp::Unset;
        super::handle_remote_identity(&backend, temp.path(), request, "unset").unwrap();
        assert!(
            repo.config()
                .unwrap()
                .get_string("remote.origin.gwzSshIdentity")
                .is_err()
        );
    }

    #[test]
    fn one_invalid_target_prevents_all_identity_writes() {
        let temp = TempDir::new("remote-identity-preflight");
        let backend = Git2Backend::without_credential_helpers();
        handle_create_workspace(create_workspace_request(temp.path()), "create").unwrap();
        backend
            .add_remote(temp.path(), "origin", "ssh://git@example.invalid/root")
            .unwrap();
        write_pull_fixture(
            temp.path(),
            vec![(
                "mem_app",
                "absent",
                "ssh://git@example.invalid/app",
                &"0".repeat(40),
            )],
        );
        std::fs::write(temp.path().join("key"), "fixture").unwrap();
        let request = crate::RemoteIdentityRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@all".into()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: "origin".into(),
            op: crate::RemoteIdentityOp::Set,
            private_key_path: Some("key".into()),
        };
        assert!(super::handle_remote_identity(&backend, temp.path(), request, "set").is_err());
        assert!(
            git2::Repository::open(temp.path())
                .unwrap()
                .config()
                .unwrap()
                .get_string("remote.origin.gwzSshIdentity")
                .is_err()
        );
    }
}
