use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{
    self, ArtifactSourceKind, DesiredRefArtifact, LockArtifact, ManifestArtifact, ManifestMember,
    RemoteArtifact, ResolvedMemberArtifact, WorkspaceHeader,
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::artifact::{read_lock, read_manifest};
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
