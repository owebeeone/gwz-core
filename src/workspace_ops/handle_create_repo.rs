use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, DesiredRefArtifact, LockArtifact,
    ManifestArtifact, ManifestMember, RemoteArtifact, ResolvedMemberArtifact, WorkspaceHeader,
};
use crate::git::{Git2Backend, GitBackend, GitHeadState, GitStatus};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::OperationRequest;
use crate::workspace::{
    MemberPath, discover_workspace_root,
    preflight_create_workspace, validate_member_path_set,
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
    let workspace_id = request
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws_default".to_owned());
    crate::model::WorkspaceId::parse_str(&workspace_id)?;
    ensure_workspace_git_repo(&root)?;

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
    sync_workspace_git_metadata(&root, &[])?;

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
    sync_workspace_git_metadata(&root, &manifest.members)?;

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
    let repo_path = resolve_input_path(start, &request.repository_path);
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
    sync_workspace_git_metadata(&root, &manifest.members)?;

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

pub(crate) fn resolve_workspace_root(
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

pub(crate) fn reject_existing_member_path(manifest: &ManifestArtifact, path: &MemberPath) -> ModelResult<()> {
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

pub(crate) fn reject_duplicate_member_id(manifest: &ManifestArtifact, member_id: &str) -> ModelResult<()> {
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
            created_at: now_marker(),
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

pub(crate) fn sync_workspace_git_metadata(root: &Path, members: &[ManifestMember]) -> ModelResult<()> {
    update_workspace_gitignore(root, members)?;
    stage_workspace_git_metadata(root)
}

pub(crate) fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

