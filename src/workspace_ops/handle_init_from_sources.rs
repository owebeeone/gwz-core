use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::artifact::{
    self, ArtifactSourceKind, DesiredRefArtifact, LockArtifact, ManifestArtifact, ManifestMember,
    RemoteArtifact, ResolvedMemberArtifact, WorkspaceHeader,
};
use crate::git::{GitBackend, GitHeadState, git_host};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult, SourceId};
use crate::operation::{
    EventEmitter, EventSink, OperationRequest, par_map_per_host, resolve_jobs, resolve_per_host,
};
use crate::workspace::{
    MemberPath, WORKSPACE_MANIFEST, preflight_create_workspace, validate_member_path_set,
};

use super::*;

pub fn handle_init_from_sources<B>(
    backend: &B,
    start: &Path,
    request: crate::InitFromSourcesRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::InitFromSourcesResponse>
where
    B: GitBackend + Sync,
{
    let context =
        OperationRequest::InitFromSources(request.clone()).context(operation_id.into())?;
    let root = if request.workspace_root.trim().is_empty() {
        start.to_path_buf()
    } else {
        PathBuf::from(&request.workspace_root)
    };
    if request.sources.is_empty() {
        return Err(invalid("init from sources requires at least one source"));
    }
    assert_init_target_is_head(request.target.as_ref())?;
    let force_bootstrap = force_bootstrap_overwrite(&request.meta);

    if root.join(WORKSPACE_MANIFEST).exists() {
        let manifest = artifact::read_manifest(&root)?;
        if let Some(expected) = &request.workspace_id
            && expected != &manifest.workspace.id
        {
            return Err(ModelError::new(
                ErrorCode::WorkspaceNotFound,
                "workspace id does not match manifest",
            ));
        }
        let plans = init_source_plans(&manifest, &request.sources)?;
        return Ok(crate::InitFromSourcesResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                plans.iter().map(InitSourcePlan::planned_response).collect(),
            ),
        });
    }

    preflight_create_workspace(&root)?;
    preflight_workspace_bootstrap_files(&root, force_bootstrap)?;
    let workspace_id = request
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws_default".to_owned());
    crate::model::WorkspaceId::parse_str(&workspace_id)?;
    let mut manifest = ManifestArtifact {
        schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: workspace_id.clone(),
        },
        members: Vec::new(),
    };
    let plans = init_source_plans(&manifest, &request.sources)?;
    preflight_init_execution_targets(&root, &plans)?;

    if request.meta.dry_run.unwrap_or(false) {
        return Ok(crate::InitFromSourcesResponse {
            response: response_envelope(
                context,
                crate::AggregateStatus::Accepted,
                plans.iter().map(InitSourcePlan::planned_response).collect(),
            ),
        });
    }

    ensure_workspace_git_repo(&root)?;
    let mut lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id,
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        created_at: now_marker(),
        members: BTreeMap::new(),
    };
    let progress_interval = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.progress_min_interval_ms)
        .unwrap_or(0);
    let emitter = EventEmitter::new(&context, events, progress_interval);
    emitter.operation_started();
    // F2: every init member is a fresh clone — rolled back on any mid-batch
    // failure (Q6 reject-partial) so no orphan repos are left behind.
    let fresh_clone_paths: Vec<_> = plans
        .iter()
        .map(|plan| root.join(plan.path.as_str()))
        .collect();
    type InitOutcome = (
        ManifestMember,
        ResolvedMemberArtifact,
        crate::MemberResponse,
    );
    let outcomes = par_map_per_host(
        plans,
        resolve_jobs(
            request
                .meta
                .policy
                .as_ref()
                .and_then(|policy| policy.concurrency),
        ),
        resolve_per_host(
            request
                .meta
                .policy
                .as_ref()
                .and_then(|policy| policy.max_connections_per_host),
        ),
        |plan| git_host(&plan.source.url),
        |plan| -> ModelResult<InitOutcome> {
            let member_root = root.join(plan.path.as_str());
            emitter.member_started(&plan.member_id, plan.path.as_str());
            backend.clone_repo_with_progress(&plan.source.url, &member_root, &|progress| {
                emitter.member_progress(&plan.member_id, plan.path.as_str(), progress)
            })?;
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            emitter.member_finished(&plan.member_id, plan.path.as_str());
            let remotes = backend.remotes(&member_root)?;
            let manifest_member = ManifestMember {
                id: plan.member_id.clone(),
                path: plan.path.as_str().to_owned(),
                source_kind: ArtifactSourceKind::Git,
                source_id: plan.source_id.clone(),
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
            let locked = resolved_member(&manifest_member, &head, &status);
            let response = crate::MemberResponse {
                member_id: plan.member_id,
                member_path: manifest_member.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Ok,
                error: None,
                planned: None,
                state: Some(protocol_state(&manifest_member, &locked)),
                git_status: None,
                lock_match: Some(crate::LockMatch::Matches),
            };
            Ok((manifest_member, locked, response))
        },
    );
    let mut members = Vec::with_capacity(outcomes.len());
    let mut first_error = None;
    for outcome in outcomes {
        match outcome {
            Ok((manifest_member, locked, response)) => {
                lock.members.insert(manifest_member.id.clone(), locked);
                members.push(response);
                manifest.members.push(manifest_member);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        // F2/Q6 reject-partial: a source failed mid-batch. Roll back this op's
        // fresh clones and write no manifest/lock — failed = nothing changed.
        for path in &fresh_clone_paths {
            let _ = std::fs::remove_dir_all(path);
        }
        emitter.operation_finished();
        return Err(error);
    }
    lock.created_at = now_marker();
    artifact::write_manifest_and_lock(&root, &manifest, &lock)?;
    sync_workspace_boundary(backend, &root, &lock)?;
    ensure_workspace_bootstrap_files(backend, &root, false, force_bootstrap)?;
    emitter.operation_finished();

    Ok(crate::InitFromSourcesResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, members),
    })
}

pub(crate) fn desired_from_head(head: &GitHeadState) -> DesiredRefArtifact {
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

pub(crate) struct InitSourcePlan {
    pub(crate) source: crate::SourceUrl,
    pub(crate) path: MemberPath,
    pub(crate) member_id: String,
    pub(crate) source_id: String,
}

impl InitSourcePlan {
    pub(crate) fn planned_response(&self) -> crate::MemberResponse {
        crate::MemberResponse {
            member_id: self.member_id.clone(),
            member_path: self.path.as_str().to_owned(),
            source_kind: crate::SourceKind::Git,
            status: crate::MemberStatus::Planned,
            error: None,
            planned: Some(crate::PlannedChange {
                action: crate::PlannedAction::Clone,
                from_ref: None,
                to_ref: self.source.branch.clone(),
                message: Some(format!(
                    "clone {} as {}",
                    self.source.url,
                    self.source.remote_name.as_deref().unwrap_or("origin")
                )),
            }),
            state: None,
            git_status: None,
            lock_match: None,
        }
    }
}

pub(crate) fn init_source_plans(
    manifest: &ManifestArtifact,
    sources: &[crate::SourceUrl],
) -> ModelResult<Vec<InitSourcePlan>> {
    let mut paths = Vec::with_capacity(manifest.members.len() + sources.len());
    let mut member_ids = manifest
        .members
        .iter()
        .map(|member| member.id.clone())
        .collect::<BTreeSet<_>>();
    let mut source_ids = manifest
        .members
        .iter()
        .map(|member| member.source_id.clone())
        .collect::<BTreeSet<_>>();
    for member in &manifest.members {
        paths.push(MemberPath::parse(&member.path)?);
    }

    let mut plans = Vec::with_capacity(sources.len());
    for source in sources {
        let path = source
            .path
            .clone()
            .unwrap_or_else(|| repo_name_from_url(&source.url));
        let member_path = MemberPath::parse(&path)?;
        paths.push(member_path.clone());
        let slug = path_slug(member_path.as_str())?;
        let member_id = format!("mem_{slug}");
        let source_id = format!("src_{slug}");
        MemberId::parse_str(&member_id)?;
        SourceId::parse_str(&source_id)?;
        plans.push(InitSourcePlan {
            source: source.clone(),
            path: member_path,
            member_id,
            source_id,
        });
    }
    validate_member_path_set(&paths)?;

    for plan in &plans {
        if !member_ids.insert(plan.member_id.clone()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "member id is already registered",
            ));
        }
        if !source_ids.insert(plan.source_id.clone()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "source id is already registered",
            ));
        }
    }
    Ok(plans)
}

pub(crate) fn assert_init_target_is_head(
    target: Option<&crate::MaterializeTarget>,
) -> ModelResult<()> {
    match target {
        None => Ok(()),
        Some(target)
            if target.kind == crate::MaterializeTargetKind::Head
                && target.name.is_none()
                && target.commit.is_none() =>
        {
            Ok(())
        }
        Some(_) => Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "init from sources only supports the default head target in v0",
        )),
    }
}

pub(crate) fn preflight_init_execution_targets(
    root: &Path,
    plans: &[InitSourcePlan],
) -> ModelResult<()> {
    for plan in plans {
        if plan.source.branch.is_some() {
            return Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                "fresh init branch selection is not supported in v0",
            ));
        }
        if plan
            .source
            .remote_name
            .as_ref()
            .is_some_and(|name| name != "origin")
        {
            return Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                "fresh init custom remote names are not supported in v0",
            ));
        }
        ensure_member_target_available(&root.join(plan.path.as_str()))?;
    }
    Ok(())
}

pub(crate) fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches(['/', '\\']);
    let tail = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed);
    tail.strip_suffix(".git").unwrap_or(tail).to_owned()
}

pub(crate) fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}
