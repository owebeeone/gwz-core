use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::artifact::{
    self, CreatedByArtifact, LockArtifact,
    ManifestArtifact, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, OperationRequest, par_map_per_host, resolve_jobs,
    resolve_per_host,
};
use crate::workspace::WORKSPACE_MANIFEST;


use super::*;

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
            "GWZ tag already exists",
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
    events: &dyn EventSink,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
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

    let progress_interval = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.progress_min_interval_ms)
        .unwrap_or(0);
    let emitter = EventEmitter::new(&context, events, progress_interval);
    emitter.operation_started();
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
        |plan| plan.clone_url.as_deref().and_then(git_host),
        |plan| -> ModelResult<crate::MemberResponse> {
            emitter.member_started(&plan.member_id, &plan.state.path);
            if let Some(url) = plan.clone_url.as_deref() {
                backend.clone_repo_with_progress(
                    url,
                    &root.join(&plan.state.path),
                    &|progress| {
                        emitter.member_progress(&plan.member_id, &plan.state.path, progress)
                    },
                )?;
            }
            if let Some(commit) = &plan.state.commit {
                backend.checkout_commit(&root.join(&plan.state.path), commit)?;
            }
            emitter.member_finished(&plan.member_id, &plan.state.path);
            Ok(materialized_response(
                &manifest,
                &plan.member_id,
                &plan.state,
            ))
        },
    );
    let mut responses = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        responses.push(outcome?);
    }
    emitter.operation_finished();

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

/// Clone a workspace from its root repository URL and complete it.
///
/// This is the one-shot form of `git clone <url> <target>` followed by
/// `gwz materialize --lock`: it clones the workspace root (the git repository
/// that owns `gwz.conf/`), verifies it is a GWZ workspace, then materializes
/// every member to the committed lock — cloning missing member repositories and
/// checking out their locked commits. The recorded operation is a lock
/// materialization; no new wire request type is introduced.
pub fn handle_clone_workspace<B>(
    backend: &B,
    meta: crate::RequestMeta,
    url: &str,
    target: &str,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
{
    let target_path = PathBuf::from(target);
    // Refuse to clone over an existing workspace rather than corrupt it.
    if target_path.join(WORKSPACE_MANIFEST).exists() {
        return Err(ModelError::new(
            ErrorCode::WorkspaceAlreadyExists,
            "clone target already contains a GWZ workspace",
        ));
    }
    // Clone the workspace root repository — the step the CLI cannot perform.
    backend.clone_repo(url, &target_path)?;
    // Verify the cloned repository really is a GWZ workspace before mutating it.
    if !target_path.join(WORKSPACE_MANIFEST).is_file() {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            format!("cloned repository is not a GWZ workspace: {WORKSPACE_MANIFEST} missing"),
        ));
    }
    // Complete the clone: materialize members to the committed lock.
    let workspace_id = meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_id.clone());
    let materialize = crate::MaterializeRequest {
        meta: crate::RequestMeta {
            workspace: Some(crate::WorkspaceRef {
                root: Some(target_path.to_string_lossy().into_owned()),
                workspace_id,
            }),
            ..meta
        },
        target: crate::MaterializeTarget {
            kind: crate::MaterializeTargetKind::Lock,
            name: None,
            commit: None,
        },
    };
    handle_materialize(backend, &target_path, materialize, operation_id, events)
}

pub fn handle_pull_snapshot<B>(
    backend: &B,
    start: &Path,
    request: crate::PullSnapshotRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PullSnapshotResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::PullSnapshot(request.clone()).context(operation_id.into())?;
    let materialize = crate::MaterializeRequest {
        meta: request.meta,
        target: crate::MaterializeTarget {
            kind: crate::MaterializeTargetKind::Snapshot,
            name: Some(request.snapshot_id),
            commit: None,
        },
    };
    let mut response = handle_materialize(
        backend,
        start,
        materialize,
        context.operation_id.clone(),
        events,
    )?
    .response;
    response.meta = crate::ResponseMeta {
        request_id: context.request_id,
        schema_version: context.schema_version,
        action: context.action.into(),
        aggregate_status: response.meta.aggregate_status,
        operation_id: Some(context.operation_id),
        message: response.meta.message,
        attribution: context.attribution.as_ref().map(Into::into),
    };
    Ok(crate::PullSnapshotResponse { response })
}

pub(crate) fn resolve_locked_selection(
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<String>> {
    let selected = resolve_manifest_selection(manifest, selection)?;
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

pub(crate) fn selected_member_map(
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

pub(crate) fn locked_member_responses(
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

pub(crate) fn created_by(context: &crate::operation::OperationContext) -> CreatedByArtifact {
    CreatedByArtifact {
        actor_id: context
            .attribution
            .as_ref()
            .and_then(|attribution| attribution.actor.as_ref())
            .map(|actor| actor.actor_id.clone())
            .unwrap_or_else(|| "unknown".to_owned()),
    }
}

pub(crate) fn materialize_target_members(
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

pub(crate) fn materialized_response(
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

pub(crate) fn tag_error(error: ModelError) -> ModelError {
    if matches!(
        error.code,
        ErrorCode::InvalidRequest | ErrorCode::TagInvalid
    ) {
        ModelError::new(ErrorCode::TagInvalid, error.message)
    } else {
        error
    }
}

