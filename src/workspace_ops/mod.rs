use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, CreatedByArtifact, DesiredRefArtifact, LockArtifact,
    ManifestArtifact, ManifestMember, RemoteArtifact, ResolvedMemberArtifact, WorkspaceHeader,
};
use crate::git::{GitBackend, GitHeadState, GitStatus};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::OperationRequest;
use crate::workspace::{
    MemberPath, discover_workspace_root, preflight_create_workspace, validate_member_path_set,
};

pub fn handle_create_workspace(
    request: crate::CreateWorkspaceRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CreateWorkspaceResponse> {
    let context =
        OperationRequest::CreateWorkspace(request.clone()).context(operation_id.into())?;
    let root = PathBuf::from(&request.workspace_root);
    preflight_create_workspace(&root)?;
    let workspace_id = request
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws_default".to_owned());
    crate::model::WorkspaceId::parse_str(&workspace_id)?;

    artifact::write_manifest(
        &root,
        &ManifestArtifact {
            schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: workspace_id.clone(),
            },
            members: Vec::new(),
        },
    )?;
    artifact::write_lock(
        &root,
        &LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id,
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            created_at: now_marker(),
            members: BTreeMap::new(),
        },
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
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let member_path = MemberPath::parse(&request.member_path)?;
    reject_existing_member_path(&manifest, &member_path)?;
    let member_abs_path = root.join(member_path.as_str());
    ensure_member_target_available(&member_abs_path)?;

    let slug = path_slug(member_path.as_str())?;
    let member_id = request.member_id.unwrap_or_else(|| format!("mem_{slug}"));
    let source_id = request.source_id.unwrap_or_else(|| format!("src_{slug}"));
    MemberId::parse_str(&member_id)?;
    SourceId::parse_str(&source_id)?;
    reject_duplicate_member_id(&manifest, &member_id)?;

    backend.create_repo(&member_abs_path)?;
    let head = backend.head(&member_abs_path)?;
    let status = backend.status(&member_abs_path)?;
    let remotes = backend.remotes(&member_abs_path)?;

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
        remotes: remotes
            .iter()
            .map(|remote| RemoteArtifact {
                name: remote.name.clone(),
                url: remote.url.clone().unwrap_or_default(),
                fetch: true,
                push: true,
            })
            .collect(),
    };
    manifest.members.push(manifest_member.clone());
    let paths = manifest
        .members
        .iter()
        .map(|member| MemberPath::parse(&member.path))
        .collect::<ModelResult<Vec<_>>>()?;
    validate_member_path_set(&paths)?;
    artifact::write_manifest(&root, &manifest)?;

    let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
    let locked = resolved_member(&manifest_member, &head, &status);
    lock.members.insert(member_id.clone(), locked.clone());
    lock.created_at = now_marker();
    artifact::write_lock(&root, &lock)?;

    Ok(crate::CreateRepoResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            vec![crate::MemberResponse {
                member_id,
                member_path: manifest_member.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Ok,
                error: None,
                planned: None,
                state: Some(protocol_state(&manifest_member, &locked)),
                git_status: None,
                lock_match: Some(crate::LockMatch::Matches),
            }],
        ),
    })
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
    let mut manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let repo_path = PathBuf::from(&request.repository_path);
    if !backend.is_repository(&repo_path)? {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "repository_path is not a git repository",
        ));
    }

    let member_path = existing_repo_member_path(&root, &repo_path, request.member_path.as_ref())?;
    reject_existing_member_path(&manifest, &member_path)?;
    let slug = path_slug(member_path.as_str())?;
    let member_id = request.member_id.unwrap_or_else(|| format!("mem_{slug}"));
    let source_id = request.source_id.unwrap_or_else(|| format!("src_{slug}"));
    MemberId::parse_str(&member_id)?;
    SourceId::parse_str(&source_id)?;
    reject_duplicate_member_id(&manifest, &member_id)?;

    let head = backend.head(&repo_path)?;
    let status = backend.status(&repo_path)?;
    let remotes = backend.remotes(&repo_path)?;
    let manifest_member = ManifestMember {
        id: member_id.clone(),
        path: member_path.as_str().to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: source_id.clone(),
        active: true,
        desired: Some(desired_from_head(&head)),
        remotes: remotes
            .iter()
            .map(|remote| RemoteArtifact {
                name: remote.name.clone(),
                url: remote.url.clone().unwrap_or_default(),
                fetch: true,
                push: true,
            })
            .collect(),
    };
    manifest.members.push(manifest_member.clone());
    let paths = manifest
        .members
        .iter()
        .map(|member| MemberPath::parse(&member.path))
        .collect::<ModelResult<Vec<_>>>()?;
    validate_member_path_set(&paths)?;
    artifact::write_manifest(&root, &manifest)?;

    let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
    let locked = resolved_member(&manifest_member, &head, &status);
    lock.members.insert(member_id.clone(), locked.clone());
    lock.created_at = now_marker();
    artifact::write_lock(&root, &lock)?;

    Ok(crate::AddExistingRepoResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            vec![crate::MemberResponse {
                member_id,
                member_path: manifest_member.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Ok,
                error: None,
                planned: None,
                state: Some(protocol_state(&manifest_member, &locked)),
                git_status: None,
                lock_match: Some(crate::LockMatch::Matches),
            }],
        ),
    })
}

pub fn handle_init_from_sources(
    start: &Path,
    request: crate::InitFromSourcesRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::InitFromSourcesResponse> {
    let context =
        OperationRequest::InitFromSources(request.clone()).context(operation_id.into())?;
    let root = if request.workspace_root.trim().is_empty() {
        start.to_path_buf()
    } else {
        PathBuf::from(&request.workspace_root)
    };
    let manifest = artifact::read_manifest(&root)?;
    if let Some(expected) = &request.workspace_id {
        if expected != &manifest.workspace.id {
            return Err(ModelError::new(
                ErrorCode::WorkspaceNotFound,
                "workspace id does not match manifest",
            ));
        }
    }
    if request.sources.is_empty() {
        return Err(invalid("init from sources requires at least one source"));
    }

    let mut paths = Vec::with_capacity(manifest.members.len() + request.sources.len());
    for member in &manifest.members {
        paths.push(MemberPath::parse(&member.path)?);
    }
    let mut members = Vec::with_capacity(request.sources.len());
    for source in &request.sources {
        let path = source
            .path
            .clone()
            .unwrap_or_else(|| format!("repos/{}", repo_name_from_url(&source.url)));
        let member_path = MemberPath::parse(&path)?;
        paths.push(member_path.clone());
        let slug = path_slug(member_path.as_str())?;
        let member_id = format!("mem_{slug}");
        members.push(crate::MemberResponse {
            member_id,
            member_path: member_path.as_str().to_owned(),
            source_kind: crate::SourceKind::Git,
            status: crate::MemberStatus::Planned,
            error: None,
            planned: Some(crate::PlannedChange {
                action: crate::PlannedAction::Clone,
                from_ref: None,
                to_ref: source.branch.clone(),
                message: Some(format!(
                    "clone {} as {}",
                    source.url,
                    source.remote_name.as_deref().unwrap_or("origin")
                )),
            }),
            state: None,
            git_status: None,
            lock_match: None,
        });
    }
    validate_member_path_set(&paths)?;

    Ok(crate::InitFromSourcesResponse {
        response: response_envelope(context, crate::AggregateStatus::Accepted, members),
    })
}

pub fn handle_snapshot(
    start: &Path,
    request: crate::SnapshotRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::SnapshotResponse> {
    let context = OperationRequest::Snapshot(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let members = selected_member_map(&lock, &selected)?;
    artifact::write_snapshot(
        &root,
        &artifact::SnapshotArtifact {
            schema: artifact::SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: manifest.workspace.id.clone(),
            snapshot_id: request.snapshot_id,
            created_at: now_marker(),
            created_by: created_by(&context),
            selected_members: selected.clone(),
            members: members.clone(),
        },
    )?;

    Ok(crate::SnapshotResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            locked_member_responses(&manifest, &members),
        ),
    })
}

pub fn handle_tag(
    start: &Path,
    request: crate::TagRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::TagResponse> {
    let context = OperationRequest::Tag(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let tag_path = root
        .join(artifact::TAG_DIR)
        .join(format!("{}.yml", request.tag_name));
    if tag_path.exists() {
        return Err(ModelError::new(
            ErrorCode::TagInvalid,
            "GWS tag already exists",
        ));
    }

    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let members = selected_member_map(&lock, &selected)?;
    let tag = artifact::TagArtifact {
        schema: artifact::TAG_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        tag: request.tag_name,
        created_at: now_marker(),
        created_by: created_by(&context),
        selected_members: selected.clone(),
        members: members.clone(),
    };
    artifact::write_tag(&root, &tag).map_err(tag_error)?;

    Ok(crate::TagResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            locked_member_responses(&manifest, &members),
        ),
    })
}

pub fn load_snapshot_target(
    root: &Path,
    snapshot_id: &str,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    Ok(artifact::read_snapshot(root, snapshot_id)?.members)
}

pub fn load_tag_target(
    root: &Path,
    tag_name: &str,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    Ok(artifact::read_tag(root, tag_name)?.members)
}

pub fn handle_materialize<B>(
    backend: &B,
    start: &Path,
    request: crate::MaterializeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Materialize(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let (target_members, rewrite_lock) = materialize_target_members(&root, &request.target)?;
    let target_lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        created_at: now_marker(),
        members: target_members,
    };
    let selected =
        resolve_locked_selection(&manifest, &target_lock, request.meta.selection.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let destructive_allowed = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.destructive)
        == Some(crate::DestructiveBehavior::Allow);

    let plans = materialize_preflight(
        backend,
        &root,
        &manifest,
        &target_lock,
        &selected,
        destructive_allowed,
    )?;
    if dry_run {
        return Ok(crate::MaterializeResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                plans.into_iter().map(|plan| plan.response).collect(),
            ),
        });
    }

    let mut responses = Vec::with_capacity(plans.len());
    for plan in plans {
        if plan.clone_url.is_some() {
            backend.clone_repo(
                plan.clone_url.as_deref().unwrap(),
                &root.join(&plan.state.path),
            )?;
        }
        if let Some(commit) = &plan.state.commit {
            backend.checkout_commit(&root.join(&plan.state.path), commit)?;
        }
        responses.push(materialized_response(
            &manifest,
            &plan.member_id,
            &plan.state,
        ));
    }

    if rewrite_lock {
        let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
        for member_id in &selected {
            if let Some(state) = target_lock.members.get(member_id) {
                lock.members.insert(member_id.clone(), state.clone());
            }
        }
        lock.created_at = now_marker();
        artifact::write_lock(&root, &lock)?;
    }

    Ok(crate::MaterializeResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, responses),
    })
}

fn resolve_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<PathBuf> {
    if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        Ok(PathBuf::from(root))
    } else {
        discover_workspace_root(start)
    }
}

fn assert_workspace_id(
    manifest: &ManifestArtifact,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<()> {
    if let Some(expected) = workspace.and_then(|workspace| workspace.workspace_id.as_ref()) {
        if expected != &manifest.workspace.id {
            return Err(ModelError::new(
                ErrorCode::WorkspaceNotFound,
                "workspace id does not match manifest",
            ));
        }
    }
    Ok(())
}

fn resolve_locked_selection(
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<String>> {
    let selected = match selection {
        None => manifest
            .members
            .iter()
            .filter(|member| member.active)
            .map(|member| member.id.clone())
            .collect::<Vec<_>>(),
        Some(selection) => resolve_explicit_locked_selection(manifest, selection)?,
    };
    for member_id in &selected {
        if !lock.members.contains_key(member_id) {
            return Err(ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{member_id}'"),
            ));
        }
    }
    Ok(selected)
}

fn resolve_explicit_locked_selection(
    manifest: &ManifestArtifact,
    selection: &crate::Selection,
) -> ModelResult<Vec<String>> {
    let has_filters = !selection.member_ids.is_empty() || !selection.paths.is_empty();
    if selection.all == Some(true) {
        if has_filters {
            return Err(invalid(
                "selection cannot combine all=true with member filters",
            ));
        }
        return Ok(manifest
            .members
            .iter()
            .filter(|member| member.active)
            .map(|member| member.id.clone())
            .collect());
    }
    if !has_filters {
        return Err(invalid(
            "selection must include all=true, member_ids, or paths",
        ));
    }

    let mut selected = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for member_id in &selection.member_ids {
        MemberId::parse_str(member_id)?;
        let member = find_active_member_by_id(manifest, member_id)?;
        if !seen.insert(member.id.clone()) {
            return Err(invalid("selection resolves the same member more than once"));
        }
        selected.push(member.id.clone());
    }
    for path in &selection.paths {
        MemberPath::parse(path)?;
        let member = find_active_member_by_path(manifest, path)?;
        if !seen.insert(member.id.clone()) {
            return Err(invalid("selection resolves the same member more than once"));
        }
        selected.push(member.id.clone());
    }
    Ok(selected)
}

fn find_active_member_by_id<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;
    if member.active {
        Ok(member)
    } else {
        Err(ModelError::new(
            ErrorCode::MemberInactive,
            "selected member is inactive",
        ))
    }
}

fn find_active_member_by_path<'a>(
    manifest: &'a ManifestArtifact,
    path: &str,
) -> ModelResult<&'a ManifestMember> {
    let member = manifest
        .members
        .iter()
        .find(|member| member.path == path)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member path not found"))?;
    if member.active {
        Ok(member)
    } else {
        Err(ModelError::new(
            ErrorCode::MemberInactive,
            "selected member is inactive",
        ))
    }
}

fn selected_member_map(
    lock: &LockArtifact,
    selected: &[String],
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    let mut members = BTreeMap::new();
    for member_id in selected {
        let member = lock.members.get(member_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{member_id}'"),
            )
        })?;
        members.insert(member_id.clone(), member.clone());
    }
    Ok(members)
}

fn locked_member_responses(
    manifest: &ManifestArtifact,
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> Vec<crate::MemberResponse> {
    members
        .iter()
        .map(|(member_id, state)| {
            let manifest_member = manifest
                .members
                .iter()
                .find(|member| &member.id == member_id);
            crate::MemberResponse {
                member_id: member_id.clone(),
                member_path: state.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Ok,
                error: None,
                planned: None,
                state: manifest_member.map(|member| protocol_state(member, state)),
                git_status: None,
                lock_match: Some(crate::LockMatch::Matches),
            }
        })
        .collect()
}

fn created_by(context: &crate::operation::OperationContext) -> CreatedByArtifact {
    CreatedByArtifact {
        actor_id: context
            .attribution
            .as_ref()
            .and_then(|attribution| attribution.actor.as_ref())
            .map(|actor| actor.actor_id.clone())
            .unwrap_or_else(|| "unknown".to_owned()),
    }
}

fn reject_existing_member_path(manifest: &ManifestArtifact, path: &MemberPath) -> ModelResult<()> {
    if manifest
        .members
        .iter()
        .any(|member| member.path == path.as_str())
    {
        Err(ModelError::new(
            ErrorCode::PathCollision,
            "member path is already registered",
        ))
    } else {
        Ok(())
    }
}

fn reject_duplicate_member_id(manifest: &ManifestArtifact, member_id: &str) -> ModelResult<()> {
    if manifest.members.iter().any(|member| member.id == member_id) {
        Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "member id is already registered",
        ))
    } else {
        Ok(())
    }
}

fn existing_repo_member_path(
    root: &Path,
    repo_path: &Path,
    requested: Option<&String>,
) -> ModelResult<MemberPath> {
    let member_path = if let Some(path) = requested {
        MemberPath::parse(path)?
    } else {
        let relative = repo_path.strip_prefix(root).map_err(|_| {
            ModelError::new(
                ErrorCode::PathEscape,
                "repository_path must be inside the workspace when member_path is omitted",
            )
        })?;
        MemberPath::parse(&relative.to_string_lossy())?
    };
    if root.join(member_path.as_str()) != repo_path {
        return Err(ModelError::new(
            ErrorCode::PathEscape,
            "member_path must point at repository_path",
        ));
    }
    Ok(member_path)
}

fn desired_from_head(head: &GitHeadState) -> DesiredRefArtifact {
    if let Some(branch) = &head.branch {
        DesiredRefArtifact {
            branch: Some(branch.clone()),
            ..Default::default()
        }
    } else if let Some(commit) = &head.commit {
        DesiredRefArtifact {
            commit: Some(commit.clone()),
            ..Default::default()
        }
    } else {
        DesiredRefArtifact {
            local_only: Some(true),
            ..Default::default()
        }
    }
}

fn ensure_member_target_available(path: &Path) -> ModelResult<()> {
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

fn read_lock_or_empty(root: &Path, workspace_id: &str) -> ModelResult<LockArtifact> {
    if root.join(artifact::LOCK_PATH).exists() {
        artifact::read_lock(root)
    } else {
        Ok(LockArtifact {
            schema: artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: workspace_id.to_owned(),
            manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
            created_at: now_marker(),
            members: BTreeMap::new(),
        })
    }
}

fn materialize_target_members(
    root: &Path,
    target: &crate::MaterializeTarget,
) -> ModelResult<(BTreeMap<String, ResolvedMemberArtifact>, bool)> {
    match target.kind {
        crate::MaterializeTargetKind::Lock => {
            if !root.join(artifact::LOCK_PATH).exists() {
                return Err(ModelError::new(ErrorCode::LockNotFound, "lock not found"));
            }
            Ok((artifact::read_lock(root)?.members, false))
        }
        crate::MaterializeTargetKind::Snapshot => {
            let name = target
                .name
                .as_ref()
                .ok_or_else(|| invalid("snapshot target requires a name"))?;
            if !root
                .join(artifact::SNAPSHOT_DIR)
                .join(format!("{name}.yaml"))
                .exists()
            {
                return Err(ModelError::new(
                    ErrorCode::SnapshotNotFound,
                    "snapshot not found",
                ));
            }
            Ok((load_snapshot_target(root, name)?, true))
        }
        crate::MaterializeTargetKind::Tag => {
            let name = target
                .name
                .as_ref()
                .ok_or_else(|| invalid("tag target requires a name"))?;
            if !root
                .join(artifact::TAG_DIR)
                .join(format!("{name}.yml"))
                .exists()
            {
                return Err(ModelError::new(ErrorCode::TagNotFound, "tag not found"));
            }
            Ok((load_tag_target(root, name)?, true))
        }
        crate::MaterializeTargetKind::Commit | crate::MaterializeTargetKind::Head => {
            Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                "target is not supported here",
            ))
        }
    }
}

struct MaterializePlan {
    member_id: String,
    state: ResolvedMemberArtifact,
    clone_url: Option<String>,
    response: crate::MemberResponse,
}

fn materialize_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    target_lock: &LockArtifact,
    selected: &[String],
    destructive_allowed: bool,
) -> ModelResult<Vec<MaterializePlan>>
where
    B: GitBackend,
{
    let mut plans = Vec::with_capacity(selected.len());
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let state = target_lock.members.get(member_id).cloned().ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("target state missing for member '{member_id}'"),
            )
        })?;
        let member_root = root.join(&state.path);
        let is_repo = member_root.exists() && backend.is_repository(&member_root)?;
        let clone_url = if is_repo {
            let status = backend.status(&member_root)?;
            if status.is_dirty && !destructive_allowed {
                return Err(ModelError::new(
                    ErrorCode::DirtyMember,
                    format!("member '{member_id}' has uncommitted changes"),
                ));
            }
            None
        } else {
            Some(first_remote_url(member)?)
        };
        let action = if clone_url.is_some() {
            crate::PlannedAction::Clone
        } else if state.commit.is_some() {
            crate::PlannedAction::Checkout
        } else {
            crate::PlannedAction::Noop
        };
        plans.push(MaterializePlan {
            member_id: member_id.clone(),
            state: state.clone(),
            clone_url,
            response: crate::MemberResponse {
                member_id: member_id.clone(),
                member_path: state.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Planned,
                error: None,
                planned: Some(crate::PlannedChange {
                    action,
                    from_ref: None,
                    to_ref: state.commit.clone(),
                    message: None,
                }),
                state: Some(protocol_state(member, &state)),
                git_status: None,
                lock_match: Some(crate::LockMatch::Differs),
            },
        });
    }
    Ok(plans)
}

fn first_remote_url(member: &ManifestMember) -> ModelResult<String> {
    member
        .remotes
        .iter()
        .find(|remote| remote.fetch)
        .map(|remote| remote.url.clone())
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "member has no fetch remote"))
}

fn materialized_response(
    manifest: &ManifestArtifact,
    member_id: &str,
    state: &ResolvedMemberArtifact,
) -> crate::MemberResponse {
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id);
    crate::MemberResponse {
        member_id: member_id.to_owned(),
        member_path: state.path.clone(),
        source_kind: crate::SourceKind::Git,
        status: crate::MemberStatus::Ok,
        error: None,
        planned: None,
        state: member.map(|member| protocol_state(member, state)),
        git_status: None,
        lock_match: Some(crate::LockMatch::Matches),
    }
}

fn resolved_member(
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

fn protocol_state(
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

fn response_envelope(
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

fn path_slug(path: &str) -> ModelResult<String> {
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

fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let tail = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    tail.strip_suffix(".git").unwrap_or(tail).to_owned()
}

fn now_marker() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("unix-ms:{millis}")
}

fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn tag_error(error: ModelError) -> ModelError {
    if matches!(
        error.code,
        ErrorCode::InvalidRequest | ErrorCode::TagInvalid
    ) {
        ModelError::new(ErrorCode::TagInvalid, error.message)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::artifact::{read_lock, read_manifest, read_snapshot, read_tag};
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;

    use super::*;

    #[test]
    fn create_workspace_writes_empty_manifest_and_lock() {
        let temp = TempDir::new("create-workspace");
        let response =
            handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert!(response.response.members.is_empty());
        assert_eq!(read_manifest(temp.path()).unwrap().members.len(), 0);
        assert_eq!(read_lock(temp.path()).unwrap().members.len(), 0);
    }

    #[test]
    fn create_workspace_rejects_existing_and_nested_workspaces() {
        let temp = TempDir::new("reject-workspace");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        assert_eq!(
            handle_create_workspace(create_workspace_request(temp.path()), "op_create")
                .unwrap_err()
                .code,
            ErrorCode::WorkspaceAlreadyExists
        );

        let child = temp.path().join("repos/child");
        fs::create_dir_all(&child).unwrap();
        assert_eq!(
            handle_create_workspace(create_workspace_request(&child), "op_create")
                .unwrap_err()
                .code,
            ErrorCode::NestedWorkspace
        );
    }

    #[test]
    fn create_repo_writes_manifest_lock_and_empty_git_repo() {
        let temp = TempDir::new("create-repo");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let response = handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();

        let member = response.response.members.single();
        assert_eq!(member.status, crate::MemberStatus::Ok);
        assert_eq!(member.state.as_ref().unwrap().member_id, "mem_app");
        assert_eq!(member.state.as_ref().unwrap().commit, None);
        assert_eq!(
            member.state.as_ref().unwrap().branch,
            Some("main".to_owned())
        );
        assert!(
            backend
                .is_repository(&temp.path().join("repos/app"))
                .unwrap()
        );

        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(manifest.members.len(), 1);
        assert_eq!(manifest.members[0].id, "mem_app");
        assert_eq!(manifest.members[0].source_id, "src_app");
        assert_eq!(
            manifest.members[0]
                .desired
                .as_ref()
                .and_then(|desired| desired.local_only),
            Some(true)
        );

        let lock = read_lock(temp.path()).unwrap();
        let locked = lock.members.get("mem_app").unwrap();
        assert_eq!(locked.commit, None);
        assert_eq!(locked.branch, Some("main".to_owned()));
        assert_eq!(locked.dirty, Some(false));
        assert_eq!(locked.materialized, Some(true));
    }

    #[test]
    fn add_existing_repo_records_current_git_state_and_remotes_without_reclone() {
        let temp = TempDir::new("add-existing");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let repo_path = temp.path().join("repos/existing");
        backend.create_repo(&repo_path).unwrap();
        let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        backend
            .add_remote(&repo_path, "origin", "file:///tmp/existing.git")
            .unwrap();
        fs::write(repo_path.join("README.md"), "dirty").unwrap();

        let response = handle_add_existing_repo(
            &backend,
            temp.path(),
            crate::AddExistingRepoRequest {
                meta: request_meta_with_workspace(),
                repository_path: repo_path.to_string_lossy().into_owned(),
                member_path: None,
                member_id: None,
                source_id: None,
            },
            "op_add",
        )
        .unwrap();

        let member = response.response.members.single();
        assert_eq!(member.member_path, "repos/existing");
        assert_eq!(member.state.as_ref().unwrap().commit, Some(commit.clone()));
        assert_eq!(member.state.as_ref().unwrap().dirty, Some(true));
        assert!(repo_path.join(".git").is_dir());

        let manifest = read_manifest(temp.path()).unwrap();
        assert_eq!(
            manifest.members[0].remotes[0].url,
            "file:///tmp/existing.git"
        );
        let locked = read_lock(temp.path())
            .unwrap()
            .members
            .get("mem_existing")
            .cloned()
            .unwrap();
        assert_eq!(locked.commit, Some(commit));
        assert_eq!(locked.dirty, Some(true));
    }

    #[test]
    fn init_from_sources_derives_default_paths_and_rejects_collisions() {
        let temp = TempDir::new("init-sources");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let response = handle_init_from_sources(
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![
                    crate::SourceUrl {
                        url: "git@github.com:org/repo-a.git".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                    crate::SourceUrl {
                        url: "https://github.com/org/repo-b".to_owned(),
                        path: None,
                        remote_name: Some("github".to_owned()),
                        branch: Some("main".to_owned()),
                    },
                ],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
        )
        .unwrap();

        assert_eq!(response.response.members[0].member_path, "repos/repo-a");
        assert_eq!(response.response.members[1].member_path, "repos/repo-b");
        assert_eq!(
            response.response.members[0]
                .planned
                .as_ref()
                .unwrap()
                .action,
            crate::PlannedAction::Clone
        );

        let collision = handle_init_from_sources(
            temp.path(),
            crate::InitFromSourcesRequest {
                meta: request_meta(),
                workspace_root: temp.path().to_string_lossy().into_owned(),
                sources: vec![
                    crate::SourceUrl {
                        url: "https://example.invalid/dup.git".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                    crate::SourceUrl {
                        url: "ssh://example.invalid/dup".to_owned(),
                        path: None,
                        remote_name: None,
                        branch: None,
                    },
                ],
                target: None,
                workspace_id: Some("ws_ops".to_owned()),
            },
            "op_init",
        )
        .unwrap_err();
        assert_eq!(collision.code, ErrorCode::PathCollision);
    }

    #[test]
    fn snapshot_and_tag_write_selected_member_records_with_attribution() {
        let temp = TempDir::new("snapshot-tag");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();
        let lock_before = read_lock(temp.path()).unwrap();

        let snapshot_response = handle_snapshot(
            temp.path(),
            crate::SnapshotRequest {
                meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
                snapshot_id: "snap_one".to_owned(),
            },
            "op_snapshot",
        )
        .unwrap();
        let tag_response = handle_tag(
            temp.path(),
            crate::TagRequest {
                meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
                tag_name: "release-one".to_owned(),
            },
            "op_tag",
        )
        .unwrap();

        assert_eq!(
            snapshot_response.response.members.single().member_id,
            "mem_app"
        );
        assert_eq!(tag_response.response.members.single().member_id, "mem_app");
        let snapshot = read_snapshot(temp.path(), "snap_one").unwrap();
        assert_eq!(snapshot.created_by.actor_id, "agent://tester");
        assert_eq!(snapshot.selected_members, vec!["mem_app"]);
        assert!(snapshot.members.contains_key("mem_app"));
        let tag = read_tag(temp.path(), "release-one").unwrap();
        assert_eq!(tag.created_by.actor_id, "agent://tester");
        assert!(tag.members.contains_key("mem_app"));
        assert_eq!(read_lock(temp.path()).unwrap(), lock_before);
    }

    #[test]
    fn duplicate_and_invalid_gws_tags_fail_cleanly() {
        let temp = TempDir::new("tag-errors");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();
        let request = crate::TagRequest {
            meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
            tag_name: "release-one".to_owned(),
        };
        handle_tag(temp.path(), request.clone(), "op_tag").unwrap();

        assert_eq!(
            handle_tag(temp.path(), request, "op_tag").unwrap_err().code,
            ErrorCode::TagInvalid
        );
        assert_eq!(
            handle_tag(
                temp.path(),
                crate::TagRequest {
                    meta: request_meta_with_actor_selection("agent://tester", &["mem_app"]),
                    tag_name: "bad/name".to_owned(),
                },
                "op_tag",
            )
            .unwrap_err()
            .code,
            ErrorCode::TagInvalid
        );
    }

    #[test]
    fn materialize_lock_clones_missing_member_and_checks_out_recorded_commit() {
        let temp = TempDir::new("materialize-clone");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("clone-source");
        let commit = fixture.commit_and_push("README.md", "one", "initial", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &commit);

        let response = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
        )
        .unwrap();

        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(commit)
        );
    }

    #[test]
    fn materialize_lock_blocks_dirty_member_by_default() {
        let temp = TempDir::new("materialize-dirty");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("dirty-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let second = fixture.commit_and_push("README.md", "two", "second", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        fs::write(temp.path().join("repos/app/README.md"), "dirty").unwrap();

        let err = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::DirtyMember);
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(second)
        );
    }

    #[test]
    fn materialize_lock_moves_clean_member_and_dry_run_does_not_mutate() {
        let temp = TempDir::new("materialize-clean");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("clean-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        let second = fixture.commit_and_push("README.md", "two", "second", &backend);
        write_materialize_fixture(temp.path(), fixture.remote_url(), &first);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();

        let dry_run = handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(true),
            "op_materialize",
        )
        .unwrap();
        assert_eq!(
            dry_run
                .response
                .members
                .single()
                .planned
                .as_ref()
                .unwrap()
                .action,
            crate::PlannedAction::Checkout
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(second)
        );

        handle_materialize(
            &backend,
            temp.path(),
            materialize_lock_request(false),
            "op_materialize",
        )
        .unwrap();
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(first)
        );
    }

    fn create_workspace_request(root: &Path) -> crate::CreateWorkspaceRequest {
        crate::CreateWorkspaceRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            workspace_id: Some("ws_ops".to_owned()),
        }
    }

    fn create_repo_request(
        member_path: &str,
        member_id: Option<&str>,
        source_id: Option<&str>,
    ) -> crate::CreateRepoRequest {
        crate::CreateRepoRequest {
            meta: request_meta_with_workspace(),
            member_path: member_path.to_owned(),
            initial_branch: None,
            member_id: member_id.map(ToOwned::to_owned),
            source_id: source_id.map(ToOwned::to_owned),
        }
    }

    fn commit_file(
        repo_path: &Path,
        relative_path: &str,
        content: &str,
        message: &str,
        parents: &[git2::Oid],
    ) -> Result<String, git2::Error> {
        fs::write(repo_path.join(relative_path), content).unwrap();
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(relative_path))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWS Test", "gws@example.invalid")?;
        let parent_commits = parents
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        Ok(repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )?
            .to_string())
    }

    fn request_meta() -> crate::RequestMeta {
        crate::RequestMeta {
            request_id: "req_ops".to_owned(),
            schema_version: "gws.protocol/v0".to_owned(),
            ..Default::default()
        }
    }

    fn request_meta_with_workspace() -> crate::RequestMeta {
        crate::RequestMeta {
            workspace: Some(crate::WorkspaceRef {
                root: None,
                workspace_id: Some("ws_ops".to_owned()),
            }),
            ..request_meta()
        }
    }

    fn request_meta_with_actor_selection(
        actor_id: &str,
        member_ids: &[&str],
    ) -> crate::RequestMeta {
        crate::RequestMeta {
            selection: Some(crate::Selection {
                all: Some(false),
                member_ids: member_ids.iter().map(|value| (*value).to_owned()).collect(),
                paths: Vec::new(),
            }),
            attribution: Some(crate::OperationAttribution {
                actor: Some(crate::OperationActor {
                    actor_id: actor_id.to_owned(),
                    display_name: None,
                    email: None,
                    authority: None,
                }),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        }
    }

    fn materialize_lock_request(dry_run: bool) -> crate::MaterializeRequest {
        crate::MaterializeRequest {
            meta: crate::RequestMeta {
                dry_run: Some(dry_run),
                ..request_meta_with_workspace()
            },
            target: crate::MaterializeTarget {
                kind: crate::MaterializeTargetKind::Lock,
                name: None,
                commit: None,
            },
        }
    }

    fn write_materialize_fixture(root: &Path, remote_url: &str, commit: &str) {
        crate::artifact::write_manifest(
            root,
            &crate::artifact::ManifestArtifact {
                schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
                workspace: crate::artifact::WorkspaceHeader {
                    id: "ws_ops".to_owned(),
                },
                members: vec![crate::artifact::ManifestMember {
                    id: "mem_app".to_owned(),
                    path: "repos/app".to_owned(),
                    source_kind: crate::artifact::ArtifactSourceKind::Git,
                    source_id: "src_app".to_owned(),
                    active: true,
                    desired: Some(crate::artifact::DesiredRefArtifact {
                        branch: Some("main".to_owned()),
                        ..Default::default()
                    }),
                    remotes: vec![crate::artifact::RemoteArtifact {
                        name: "origin".to_owned(),
                        url: remote_url.to_owned(),
                        fetch: true,
                        push: true,
                    }],
                }],
            },
        )
        .unwrap();
        crate::artifact::write_lock(
            root,
            &test_lock("mem_app", "repos/app", Some(commit.to_owned()), false),
        )
        .unwrap();
    }

    fn test_lock(
        member_id: &str,
        path: &str,
        commit: Option<String>,
        dirty: bool,
    ) -> crate::artifact::LockArtifact {
        crate::artifact::LockArtifact {
            schema: crate::artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_ops".to_owned(),
            manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            created_at: "2026-06-15T00:00:00Z".to_owned(),
            members: std::collections::BTreeMap::from([(
                member_id.to_owned(),
                crate::artifact::ResolvedMemberArtifact {
                    path: path.to_owned(),
                    source_id: Some("src_app".to_owned()),
                    source_kind: crate::artifact::ArtifactSourceKind::Git,
                    commit,
                    branch: Some("main".to_owned()),
                    detached: Some(false),
                    upstream: None,
                    dirty: Some(dirty),
                    materialized: Some(true),
                },
            )]),
        }
    }

    struct RemoteFixture {
        _temp: TempDir,
        source: PathBuf,
        remote: PathBuf,
    }

    impl RemoteFixture {
        fn new(prefix: &str) -> Self {
            let temp = TempDir::new(prefix);
            let source = temp.path().join("source");
            let remote = temp.path().join("remote.git");
            Git2Backend::new().create_repo(&source).unwrap();
            git2::Repository::init_bare(&remote).unwrap();
            Git2Backend::new()
                .add_remote(&source, "origin", remote.to_str().unwrap())
                .unwrap();
            Self {
                _temp: temp,
                source,
                remote,
            }
        }

        fn remote_url(&self) -> &str {
            self.remote.to_str().unwrap()
        }

        fn commit_and_push(
            &self,
            relative_path: &str,
            content: &str,
            message: &str,
            backend: &Git2Backend,
        ) -> String {
            let parent = backend
                .head(&self.source)
                .unwrap()
                .commit
                .and_then(|commit| git2::Oid::from_str(&commit).ok());
            let parents = parent.into_iter().collect::<Vec<_>>();
            let commit =
                commit_file(&self.source, relative_path, content, message, &parents).unwrap();
            backend
                .push(&self.source, "origin", "refs/heads/main:refs/heads/main")
                .unwrap();
            commit
        }
    }

    trait Single<T> {
        fn single(&self) -> &T;
    }

    impl<T> Single<T> for Vec<T> {
        fn single(&self) -> &T {
            assert_eq!(self.len(), 1);
            &self[0]
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gws-core-ops-{prefix}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
