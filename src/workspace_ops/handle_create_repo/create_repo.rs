use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, DesiredRefArtifact, ManifestMember};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::{OpenMergeCommand, OperationRequest};
use crate::workspace::MemberPath;

use super::super::*;
use super::invocation::invocation_start;
use super::lock_state::{
    default_source_id, members_with_source_id, read_lock_or_empty_in, resolved_member,
};
use super::response::{path_slug, response_envelope};
use super::validation::{
    assert_workspace_id, ensure_member_target_available_in, reject_duplicate_member_id,
    reject_existing_active_member_path_overlap,
};

pub fn handle_create_repo<B>(
    backend: &B,
    start: &Path,
    request: crate::CreateRepoRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateRepoResponse>
where
    B: GitBackend + MergeAuthorityBackend,
{
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let start = invocation_start(start, &request.meta)?;
    handle_create_repo_in(&services, backend, &start, request, operation_id)
}

pub(crate) fn handle_create_repo_in<B>(
    services: &crate::operation_context::OperationServices,
    backend: &B,
    start: &Path,
    request: crate::CreateRepoRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateRepoResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::CreateRepo(request.clone()).context(operation_id.into())?;
    if request
        .initial_branch
        .as_ref()
        .is_some_and(|branch| branch != "main")
    {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "custom initial branches are not supported in v0",
        ));
    }

    let dry_run = request.meta.dry_run.unwrap_or(false);
    let (_guard, root) = guarded_workspace_root_for_request_in(
        services,
        start,
        &request.meta,
        OpenMergeCommand::RepoMutate,
        dry_run,
    )?;
    assert_conf_unmodified_for(
        backend,
        &root,
        OpenMergeCommand::RepoMutate,
        reconcile_authority(_guard.as_ref(), dry_run),
    )?;
    let mut manifest = artifact::read_manifest_in(services.filesystem(), &root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let member_path = MemberPath::parse(&request.member_path)?;
    reject_existing_active_member_path_overlap(&manifest, &member_path)?;
    if request.member_id.is_none()
        && manifest
            .members
            .iter()
            .any(|member| !member.active && member.path == member_path.as_str())
    {
        return Err(invalid(
            "member path has inactive history; pass a new --member-id or use gwz repo attach <id>",
        ));
    }
    let member_abs_path = root.join(member_path.as_str());
    ensure_member_target_available_in(services.filesystem(), &member_abs_path)?;

    let slug = path_slug(member_path.as_str())?;
    let member_id = request
        .member_id
        .clone()
        .unwrap_or_else(|| format!("mem_{slug}"));
    MemberId::parse_str(&member_id)?;
    let source_id = request
        .source_id
        .clone()
        .unwrap_or_else(|| default_source_id(&member_id));
    SourceId::parse_str(&source_id)?;
    reject_duplicate_member_id(&manifest, &member_id)?;
    let reused_source_members = members_with_source_id(&manifest, &source_id);
    if request.source_id.is_none() && !reused_source_members.is_empty() {
        return Err(invalid(format!(
            "source id {source_id} already exists; pass --source-id {source_id} to confirm reuse"
        )));
    }

    if dry_run {
        return Ok(crate::CreateRepoResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                vec![planned_member(
                    &member_id,
                    member_path.as_str(),
                    crate::PlannedAction::InitRepo,
                    "create and register a Git repository".to_owned(),
                )],
            ),
        });
    }

    let inspected = (|| {
        backend.create_repo(&member_abs_path)?;
        let head = backend.head(&member_abs_path)?;
        let status = backend.status(&member_abs_path)?;
        let remotes = backend.remotes(&member_abs_path)?;
        let (verified, warning) = verify_source_identity_reuse(
            backend,
            &root,
            &member_abs_path,
            &source_id,
            &reused_source_members,
        )?;
        Ok::<_, ModelError>((head, status, remotes, verified, warning))
    })();
    let (head, status, remotes, verified_commits, warning) = match inspected {
        Ok(inspected) => inspected,
        Err(error) => {
            let _ = services.filesystem().remove_tree(&member_abs_path);
            return Err(error);
        }
    };

    let manifest_member = ManifestMember {
        private: false,
        id: member_id.clone(),
        path: member_path.as_str().to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: source_id.clone(),
        active: true,
        desired: Some(DesiredRefArtifact {
            local_only: Some(true),
            ..Default::default()
        }),
        remotes: observed_remotes(&remotes),
    };
    manifest.members.push(manifest_member.clone());
    let locked = resolved_member(&manifest_member, &head, &status);
    let lock = (|| {
        manifest.validate()?;
        let mut lock = read_lock_or_empty_in(services.filesystem(), &root, &manifest.workspace.id)?;
        lock.members.insert(member_id.clone(), locked.clone());
        Ok::<_, ModelError>(lock)
    })();
    let lock = match lock {
        Ok(lock) => lock,
        Err(error) => {
            let _ = services.filesystem().remove_tree(&member_abs_path);
            return Err(error);
        }
    };
    if let Err(error) =
        artifact::write_manifest_and_lock_in(services.filesystem(), &root, &manifest, &lock)
    {
        let published = artifact::read_manifest_in(services.filesystem(), &root)
            .map(|current| current.members.iter().any(|item| item.id == member_id))
            .unwrap_or(false);
        if !published {
            let _ = services.filesystem().remove_tree(&member_abs_path);
        }
        return Err(error);
    }
    sync_workspace_boundary_in(services.filesystem(), backend, &root, &manifest, &lock)?;

    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![ok_member(
            &manifest_member,
            &locked,
            Some(&head),
            Some(&status),
            crate::MemberStatus::Ok,
        )],
    );
    response.meta.message = warning.or_else(|| {
        (!reused_source_members.is_empty()).then(|| {
            format!(
                "created {member_id}; verified {verified_commits} historical commit(s) for source identity {source_id}"
            )
        })
    });
    Ok(crate::CreateRepoResponse { response })
}
