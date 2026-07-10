use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, DesiredRefArtifact, LockArtifact, ManifestArtifact, ManifestMember,
    RemoteArtifact, ResolvedMemberArtifact, WorkspaceHeader,
};
use crate::git::{Git2Backend, GitBackend, GitHeadState, GitRemote, GitStatus};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::{OperationRequest, WorkspaceMutatorLock};
use crate::workspace::{
    MemberPath, discover_workspace_root, preflight_create_workspace, validate_member_path_set,
};

use super::*;

pub fn handle_create_workspace(
    request: crate::CreateWorkspaceRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateWorkspaceResponse> {
    let context =
        OperationRequest::CreateWorkspace(request.clone()).context(operation_id.into())?;
    let root = PathBuf::from(&request.workspace_root);
    preflight_create_workspace(&root)?;
    preflight_workspace_bootstrap_files(&root, force_bootstrap_overwrite(&request.meta))?;
    let workspace_id = request
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws_default".to_owned());
    crate::model::WorkspaceId::parse_str(&workspace_id)?;
    ensure_workspace_git_repo(&root)?;
    let backend = Git2Backend::new();
    let _guard = WorkspaceMutatorLock::acquire(&root)?;

    let manifest = ManifestArtifact {
        schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: workspace_id.clone(),
        },
        members: Vec::new(),
    };
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id,
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members: BTreeMap::new(),
    };
    artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
    sync_workspace_boundary(&backend, &root, &manifest, &lock)?;
    ensure_workspace_bootstrap_files(
        &backend,
        &root,
        false,
        force_bootstrap_overwrite(&request.meta),
    )?;

    Ok(crate::CreateWorkspaceResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
    })
}

pub fn handle_create_repo<B>(
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

    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let mut manifest = artifact::read_manifest(&root)?;
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
    ensure_member_target_available(&member_abs_path)?;

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
            let _ = fs::remove_dir_all(&member_abs_path);
            return Err(error);
        }
    };

    let manifest_member = ManifestMember {
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
        let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
        lock.members.insert(member_id.clone(), locked.clone());
        Ok::<_, ModelError>(lock)
    })();
    let lock = match lock {
        Ok(lock) => lock,
        Err(error) => {
            let _ = fs::remove_dir_all(&member_abs_path);
            return Err(error);
        }
    };
    if let Err(error) = artifact::write_manifest_and_lock(&root, &manifest, &lock) {
        let published = artifact::read_manifest(&root)
            .map(|current| current.members.iter().any(|item| item.id == member_id))
            .unwrap_or(false);
        if !published {
            let _ = fs::remove_dir_all(&member_abs_path);
        }
        return Err(error);
    }
    sync_workspace_boundary(backend, &root, &manifest, &lock)?;

    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![ok_member(
            &manifest_member,
            &locked,
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

pub fn handle_add_existing_repo<B>(
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
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let repo_path = resolve_input_path(start, &request.repository_path);
    if !backend.is_repository(&repo_path)? {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "repository_path is not a git repository",
        ));
    }

    let member_path = existing_repo_member_path(&root, &repo_path, request.member_path.as_ref())?;
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
                let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
                lock.members
                    .insert(prepared.member.id.clone(), prepared.locked.clone());
                artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
                sync_workspace_boundary(backend, &root, &manifest, &lock)?;
                let mut response = response_envelope(
                    context,
                    crate::AggregateStatus::Ok,
                    vec![ok_member(
                        &prepared.member,
                        &prepared.locked,
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
    let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
    let locked = resolved_member(&manifest_member, &head, &status);
    lock.members.insert(member_id.clone(), locked.clone());
    artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
    sync_workspace_boundary(backend, &root, &manifest, &lock)?;

    let mut response = response_envelope(
        context,
        crate::AggregateStatus::Ok,
        vec![ok_member(
            &manifest_member,
            &locked,
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

pub fn handle_repo_sync<B>(
    backend: &B,
    start: &Path,
    request: crate::RepoSyncRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::RepoSyncResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::RepoSync(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let selected = resolve_manifest_selection(&manifest, request.meta.selection.as_ref())?;

    let mut plans = Vec::new();
    let mut responses = Vec::new();
    for member_id in selected {
        let Some((index, member)) = manifest
            .members
            .iter()
            .enumerate()
            .find(|(_, member)| member.id == member_id)
        else {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                "member not found",
            ));
        };
        match repo_sync_plan_member(backend, &root, member, dry_run) {
            Ok(plan) => {
                responses.push(plan.response.clone());
                plans.push((index, plan));
            }
            Err(response) => responses.push(*response),
        }
    }

    if dry_run
        || responses.iter().any(|response| {
            matches!(
                response.status,
                crate::MemberStatus::Rejected | crate::MemberStatus::Failed
            )
        })
    {
        return Ok(crate::RepoSyncResponse {
            response: response_envelope(context, repo_sync_aggregate(&responses), responses),
        });
    }

    if plans.iter().any(|(_, plan)| plan.changed) {
        let mut next = manifest;
        for (index, plan) in plans {
            if plan.changed {
                next.members[index] = plan.member;
            }
        }
        next.validate()?;
        artifact::write_manifest(&root, &next)?;
    }

    Ok(crate::RepoSyncResponse {
        response: response_envelope(context, repo_sync_aggregate(&responses), responses),
    })
}

#[derive(Clone, Debug)]
struct RepoSyncPlan {
    member: ManifestMember,
    changed: bool,
    response: crate::MemberResponse,
}

fn repo_sync_plan_member<B>(
    backend: &B,
    root: &Path,
    member: &ManifestMember,
    dry_run: bool,
) -> Result<RepoSyncPlan, Box<crate::MemberResponse>>
where
    B: GitBackend,
{
    let source_kind = artifact_source_kind_to_protocol(member.source_kind);
    if member.source_kind != ArtifactSourceKind::Git {
        return Err(Box::new(repo_sync_member_error(
            member,
            source_kind,
            ModelError::new(
                ErrorCode::UnsupportedSourceKind,
                "repo sync supports git members only",
            ),
            crate::MemberStatus::Rejected,
        )));
    }

    let member_root = root.join(&member.path);
    match backend.is_repository(&member_root) {
        Ok(true) => {}
        Ok(false) => {
            return Err(Box::new(repo_sync_member_error(
                member,
                source_kind,
                ModelError::new(ErrorCode::MemberNotFound, "member is not materialized"),
                crate::MemberStatus::Rejected,
            )));
        }
        Err(error) => {
            return Err(Box::new(repo_sync_member_error(
                member,
                source_kind,
                error,
                crate::MemberStatus::Failed,
            )));
        }
    }

    let head = backend.head(&member_root).map_err(|error| {
        Box::new(repo_sync_member_error(
            member,
            source_kind,
            error,
            crate::MemberStatus::Failed,
        ))
    })?;
    let status = backend.status(&member_root).map_err(|error| {
        Box::new(repo_sync_member_error(
            member,
            source_kind,
            error,
            crate::MemberStatus::Failed,
        ))
    })?;
    let git_remotes = backend.remotes(&member_root).map_err(|error| {
        Box::new(repo_sync_member_error(
            member,
            source_kind,
            error,
            crate::MemberStatus::Failed,
        ))
    })?;

    let mut next = member.clone();
    next.remotes = sync_member_remotes(&member.remotes, &git_remotes);
    if !next.remotes.is_empty() {
        next.desired = Some(desired_from_head(&head));
    }
    let changed = &next != member;
    let state = resolved_member(&next, &head, &status);
    let response_status = if dry_run && changed {
        crate::MemberStatus::Planned
    } else if changed {
        crate::MemberStatus::Ok
    } else {
        crate::MemberStatus::Noop
    };
    let planned = (dry_run && changed).then(|| crate::PlannedChange {
        action: crate::PlannedAction::WriteManifest,
        from_ref: None,
        to_ref: head.branch.clone().or(head.commit.clone()),
        message: Some("sync repository metadata from local git config".to_owned()),
    });

    Ok(RepoSyncPlan {
        member: next.clone(),
        changed,
        response: crate::MemberResponse {
            member_id: next.id.clone(),
            member_path: next.path.clone(),
            source_kind,
            status: response_status,
            error: None,
            planned,
            state: Some(protocol_state(&next, &state)),
            git_status: None,
            target_kind: Some(crate::TargetKind::Member),
            lock_match: None,
        },
    })
}

fn sync_member_remotes(existing: &[RemoteArtifact], observed: &[GitRemote]) -> Vec<RemoteArtifact> {
    let observed_by_name = observed
        .iter()
        .map(|remote| (remote.name.as_str(), remote))
        .collect::<BTreeMap<_, _>>();
    let mut synced = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for remote in existing {
        if let Some(observed) = observed_by_name.get(remote.name.as_str()) {
            synced.push(RemoteArtifact {
                name: remote.name.clone(),
                url: observed.url.clone().unwrap_or_else(|| remote.url.clone()),
                fetch: remote.fetch,
                push: remote.push,
            });
        } else {
            synced.push(remote.clone());
        }
        seen.insert(remote.name.clone());
    }
    for remote in observed {
        if seen.insert(remote.name.clone()) {
            synced.push(RemoteArtifact {
                name: remote.name.clone(),
                url: remote.url.clone().unwrap_or_default(),
                fetch: true,
                push: true,
            });
        }
    }
    synced
}

fn repo_sync_member_error(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    error: ModelError,
    status: crate::MemberStatus,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status,
        error: Some(crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some(member.id.clone()),
            member_path: Some(member.path.clone()),
            target_kind: Some(crate::TargetKind::Member),
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: None,
    }
}

fn repo_sync_aggregate(responses: &[crate::MemberResponse]) -> crate::AggregateStatus {
    if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Failed)
    {
        crate::AggregateStatus::Failed
    } else if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Rejected)
    {
        crate::AggregateStatus::Rejected
    } else if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Planned)
    {
        crate::AggregateStatus::Accepted
    } else if responses
        .iter()
        .all(|response| response.status == crate::MemberStatus::Noop)
    {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Ok
    }
}

pub fn resolve_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<PathBuf> {
    if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        Ok(PathBuf::from(root))
    } else {
        discover_workspace_root(start)
    }
}

pub(crate) fn assert_workspace_id(
    manifest: &ManifestArtifact,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<()> {
    if let Some(expected) = workspace.and_then(|workspace| workspace.workspace_id.as_ref())
        && expected != &manifest.workspace.id
    {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            "workspace id does not match manifest",
        ));
    }
    Ok(())
}

pub(crate) fn reject_existing_active_member_path_overlap(
    manifest: &ManifestArtifact,
    path: &MemberPath,
) -> ModelResult<()> {
    let mut active_paths = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| MemberPath::parse(&member.path))
        .collect::<ModelResult<Vec<_>>>()?;
    active_paths.push(path.clone());
    validate_member_path_set(&active_paths)
}

pub(crate) fn reject_duplicate_member_id(
    manifest: &ManifestArtifact,
    member_id: &str,
) -> ModelResult<()> {
    if manifest.members.iter().any(|member| member.id == member_id) {
        Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "member id is already registered",
        ))
    } else {
        Ok(())
    }
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

pub(crate) fn resolve_input_path(start: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&start_dir(start).join(path))
    }
}

pub(crate) fn start_dir(start: &Path) -> &Path {
    if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    }
}

pub(crate) fn ensure_member_target_available(path: &Path) -> ModelResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "member path exists and is not a directory",
        ));
    }
    if fs::read_dir(path)
        .map_err(io_error)?
        .next()
        .transpose()
        .map_err(io_error)?
        .is_some()
    {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "member path is not empty",
        ));
    }
    Ok(())
}

pub(crate) fn read_lock_or_empty(root: &Path, workspace_id: &str) -> ModelResult<LockArtifact> {
    if root.join(artifact::LOCK_PATH).exists() {
        artifact::read_lock(root)
    } else {
        Ok(LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: workspace_id.to_owned(),
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            members: BTreeMap::new(),
        })
    }
}

pub(crate) fn resolved_member(
    member: &ManifestMember,
    head: &GitHeadState,
    status: &GitStatus,
) -> ResolvedMemberArtifact {
    ResolvedMemberArtifact {
        path: member.path.clone(),
        source_id: Some(member.source_id.clone()),
        source_kind: ArtifactSourceKind::Git,
        commit: head.commit.clone(),
        branch: head.branch.clone(),
        detached: Some(head.is_detached),
        upstream: None,
        dirty: Some(status.is_dirty),
        materialized: Some(true),
    }
}

pub(crate) fn protocol_state(
    member: &ManifestMember,
    state: &ResolvedMemberArtifact,
) -> crate::ResolvedMemberState {
    crate::ResolvedMemberState {
        member_id: member.id.clone(),
        path: state.path.clone(),
        source_id: member.source_id.clone(),
        source_kind: crate::SourceKind::Git,
        commit: state.commit.clone(),
        branch: state.branch.clone(),
        detached: state.detached,
        upstream: state.upstream.clone(),
        dirty: state.dirty,
        materialized: state.materialized.unwrap_or(false),
        remotes: member
            .remotes
            .iter()
            .map(|remote| crate::RemoteSpec {
                name: remote.name.clone(),
                url: remote.url.clone(),
                fetch: Some(remote.fetch),
                push: Some(remote.push),
            })
            .collect(),
    }
}

pub(crate) fn response_envelope(
    context: crate::operation::OperationContext,
    aggregate_status: crate::AggregateStatus,
    members: Vec<crate::MemberResponse>,
) -> crate::ResponseEnvelope {
    crate::ResponseEnvelope {
        meta: crate::ResponseMeta {
            request_id: context.request_id,
            schema_version: context.schema_version,
            action: context.action.into(),
            aggregate_status,
            operation_id: Some(context.operation_id),
            message: None,
            attribution: context.attribution.as_ref().map(Into::into),
        },
        members,
        errors: Vec::new(),
    }
}

pub(crate) fn path_slug(path: &str) -> ModelResult<String> {
    let leaf = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("member path must have a final component"))?;
    let slug = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();
    if slug.is_empty() {
        Err(invalid("member path does not contain a usable id slug"))
    } else {
        Ok(slug)
    }
}

pub(crate) fn default_source_id(member_id: &str) -> String {
    format!(
        "src_{}",
        member_id
            .strip_prefix("mem_")
            .expect("validated member id has mem_ prefix")
    )
}

pub(crate) fn members_with_source_id(manifest: &ManifestArtifact, source_id: &str) -> Vec<String> {
    manifest
        .members
        .iter()
        .filter(|member| member.source_id == source_id)
        .map(|member| member.id.clone())
        .collect()
}

pub(crate) fn now_marker() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("unix-ms:{millis}")
}

pub(crate) fn ensure_workspace_git_repo(root: &Path) -> ModelResult<()> {
    if root.join(".git").exists() {
        Ok(())
    } else {
        Git2Backend::new().create_repo(root).map(|_| ())
    }
}

pub(crate) fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}
