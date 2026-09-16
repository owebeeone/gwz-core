use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, ManifestMember};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::{OpenMergeCommand, OperationRequest};
use crate::workspace::MemberPath;

use super::super::*;
use super::invocation::{invocation_start, resolve_input_path};
use super::lock_state::{
    default_source_id, members_with_source_id, read_lock_or_empty_in, resolved_member,
};
use super::response::{path_slug, response_envelope};
use super::validation::{
    assert_workspace_id, reject_duplicate_member_id, reject_existing_active_member_path_overlap,
};

pub fn handle_add_existing_repo<B>(
    backend: &B,
    start: &Path,
    request: crate::AddExistingRepoRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::AddExistingRepoResponse>
where
    B: GitBackend + MergeAuthorityBackend,
{
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let start = invocation_start(start, &request.meta)?;
    handle_add_existing_repo_in(&services, backend, &start, request, operation_id)
}

pub(crate) fn handle_add_existing_repo_in<B>(
    services: &crate::operation_context::OperationServices,
    backend: &B,
    start: &Path,
    request: crate::AddExistingRepoRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::AddExistingRepoResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::AddExistingRepo(request.clone()).context(operation_id.into())?;
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
    let repo_path = resolve_input_path(start, &request.repository_path);
    if !backend.is_repository(&repo_path)? {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "repository operand {:?} resolved to {} from caller directory {}; it is not a Git repository. --root selects the workspace; it does not change the base of relative operands.",
                request.repository_path,
                repo_path.display(),
                start.display()
            ),
        ));
    }

    let member_path = existing_repo_member_path(&root, &repo_path, request.member_path.as_ref())
        .map_err(|mut error| {
            if error.code == ErrorCode::PathEscape {
                error.message.push_str(&format!(
                    "; repository operand {:?} resolved to {} from caller directory {}; allowed workspace root is {}. --root selects the workspace; it does not change the base of relative operands.",
                    request.repository_path, repo_path.display(), start.display(), root.display()
                ));
            }
            error
        })?;
    reject_existing_active_member_path_overlap(&manifest, &member_path)?;

    if let Some(requested_id) = request.member_id.as_ref()
        && let Some(existing) = manifest
            .members
            .iter()
            .find(|member| member.id == *requested_id)
    {
        let guidance = if existing.active {
            "member id is already active"
        } else {
            "member id already exists; use gwz repo attach <id> to reactivate it"
        };
        return Err(invalid(guidance));
    }

    if request.member_id.is_none() {
        let candidates = manifest
            .members
            .iter()
            .filter(|member| !member.active && member.path == member_path.as_str())
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            let evidence = historical_member_commits(&root, &candidates)?;
            let mut matches = Vec::new();
            let mut mismatch_details = Vec::new();
            for candidate in &candidates {
                let candidate_evidence = evidence.get(candidate).map(Vec::as_slice).unwrap_or(&[]);
                if candidate_evidence.is_empty() {
                    mismatch_details.push(format!("{candidate}: no historical commit evidence"));
                    continue;
                }
                match verify_historical_identity(backend, &repo_path, candidate_evidence) {
                    Ok(count) => matches.push((candidate.clone(), count)),
                    Err(error) if error.code == ErrorCode::SourceIdentityMismatch => {
                        mismatch_details.push(format!("{candidate}: {}", error.message));
                    }
                    Err(error) => return Err(error),
                }
            }
            if matches.len() == 1 {
                let (member_id, _) = matches.pop().expect("one verified candidate");
                let prepared = prepare_attach(backend, &root, &manifest, &member_id)?;
                if dry_run {
                    let mut response = response_envelope(
                        context,
                        crate::AggregateStatus::Accepted,
                        vec![planned_member(
                            &prepared.member.id,
                            &prepared.member.path,
                            crate::PlannedAction::AttachMember,
                            format!(
                                "reattach after verifying {} historical commit(s)",
                                prepared.verified_commits
                            ),
                        )],
                    );
                    response.meta.message = Some(format!(
                        "would reattach {}; verified {} historical commit(s)",
                        prepared.member.id, prepared.verified_commits
                    ));
                    return Ok(crate::AddExistingRepoResponse { response });
                }
                apply_prepared_attach(&mut manifest, &prepared)?;
                let mut lock =
                    read_lock_or_empty_in(services.filesystem(), &root, &manifest.workspace.id)?;
                lock.members
                    .insert(prepared.member.id.clone(), prepared.locked.clone());
                artifact::write_manifest_and_lock_in(
                    services.filesystem(),
                    &root,
                    &manifest,
                    &lock,
                )?;
                sync_workspace_boundary_in(
                    services.filesystem(),
                    backend,
                    &root,
                    &manifest,
                    &lock,
                )?;
                let mut response = response_envelope(
                    context,
                    crate::AggregateStatus::Ok,
                    vec![ok_member(
                        &prepared.member,
                        &prepared.locked,
                        Some(&backend.head(&repo_path)?),
                        Some(&backend.status(&repo_path)?),
                        crate::MemberStatus::Ok,
                    )],
                );
                response.meta.message = Some(format!(
                    "reattached {}; verified {} historical commit(s)",
                    prepared.member.id, prepared.verified_commits
                ));
                return Ok(crate::AddExistingRepoResponse { response });
            }
            let match_ids = matches
                .iter()
                .map(|(member_id, _)| member_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid(format!(
                "cannot infer an inactive designation for {}; verified matches: [{}]; {}; use gwz repo attach <id> or pass a new --member-id",
                member_path.as_str(),
                match_ids,
                mismatch_details.join("; ")
            )));
        }
    }

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

    let head = backend.head(&repo_path)?;
    let status = backend.status(&repo_path)?;
    let remotes = backend.remotes(&repo_path)?;
    let (verified_commits, warning) = verify_source_identity_reuse(
        backend,
        &root,
        &repo_path,
        &source_id,
        &reused_source_members,
    )?;
    let manifest_member = ManifestMember {
        private: false,
        id: member_id.clone(),
        path: member_path.as_str().to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: source_id.clone(),
        active: true,
        desired: Some(desired_from_head(&head)),
        remotes: observed_remotes(&remotes),
    };

    if dry_run {
        let mut response = response_envelope(
            context,
            crate::AggregateStatus::Accepted,
            vec![planned_member(
                &member_id,
                member_path.as_str(),
                crate::PlannedAction::AddManifestMember,
                "register existing Git repository as a new designation".to_owned(),
            )],
        );
        response.meta.message = warning;
        return Ok(crate::AddExistingRepoResponse { response });
    }

    manifest.members.push(manifest_member.clone());
    manifest.validate()?;
    let mut lock = read_lock_or_empty_in(services.filesystem(), &root, &manifest.workspace.id)?;
    let locked = resolved_member(&manifest_member, &head, &status);
    lock.members.insert(member_id.clone(), locked.clone());
    artifact::write_manifest_and_lock_in(services.filesystem(), &root, &manifest, &lock)?;
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
                "added {member_id}; verified {verified_commits} historical commit(s) for source identity {source_id}"
            )
        })
    });
    Ok(crate::AddExistingRepoResponse { response })
}

pub(crate) fn existing_repo_member_path(
    root: &Path,
    repo_path: &Path,
    requested: Option<&String>,
) -> ModelResult<MemberPath> {
    let root = normalize_path(root);
    let repo_path = normalize_path(repo_path);
    let member_path = if let Some(path) = requested {
        MemberPath::parse(path)?
    } else {
        let relative = repo_path.strip_prefix(&root).map_err(|_| {
            ModelError::new(
                ErrorCode::PathEscape,
                "repository_path must be inside the workspace when member_path is omitted",
            )
        })?;
        MemberPath::parse(&relative.to_string_lossy())?
    };
    if normalize_path(&root.join(member_path.as_str())) != repo_path {
        return Err(ModelError::new(
            ErrorCode::PathEscape,
            "member_path must point at repository_path",
        ));
    }
    Ok(member_path)
}
