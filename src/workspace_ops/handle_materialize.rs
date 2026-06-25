use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::artifact::{
    self, CreatedByArtifact, LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, OperationRequest, WorkspaceMutatorLock, par_map_per_host,
    resolve_jobs, resolve_per_host,
};
use crate::workspace::WORKSPACE_MANIFEST;

use super::*;

pub fn handle_snapshot<B>(
    backend: &B,
    start: &Path,
    request: crate::SnapshotRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::SnapshotResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Snapshot(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    // F13: reject a duplicate snapshot id up front, the same guard `tag` already has —
    // never silently overwrite an existing snapshot.
    if artifact::snapshot_path(&root, &request.snapshot_id).exists() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("snapshot '{}' already exists", request.snapshot_id),
        ));
    }
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let members = snapshot_member_map(
        backend,
        &root,
        &manifest,
        &lock,
        &selected,
        request.source.as_ref(),
    )?;
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

/// Capture the live observed member state into the **lock** — no worktree mutation
/// (AD3 capture direction: "record where I am now"). Each materialized member's
/// observed head/status is written; unmaterialized members carry their lock state.
pub fn handle_capture<B>(
    backend: &B,
    start: &Path,
    request: crate::CaptureRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CaptureResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Capture(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let members = observed_member_map(backend, &root, &manifest, &lock, &selected)?;
    let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
    for (member_id, state) in &members {
        next.members.insert(member_id.clone(), state.clone());
    }
    next.created_at = now_marker();
    artifact::write_lock(&root, &next)?;
    sync_workspace_boundary(backend, &root, &next)?;
    Ok(crate::CaptureResponse {
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
    if request.target.kind == crate::MaterializeTargetKind::Branch {
        return handle_materialize_branch(backend, root, manifest, request, context);
    }
    let (plans, rewrite_lock) = prepare_materialize_execution(backend, &root, &manifest, &request)?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
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
    let response = apply_materialize_plans(
        backend,
        &root,
        &manifest,
        plans,
        rewrite_lock,
        request.meta.policy.as_ref(),
        context,
        &emitter,
    );
    emitter.operation_finished();
    response
}

fn prepare_materialize_execution<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    request: &crate::MaterializeRequest,
) -> ModelResult<(Vec<MaterializePlan>, bool)>
where
    B: GitBackend,
{
    let (target_members, rewrite_lock) =
        materialize_target_members(backend, root, manifest, &request.target)?;
    let target_lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        created_at: now_marker(),
        members: target_members,
    };
    // A tag covers only the members that carry it. With the default (None) selection, restrict to
    // exactly the tagged subset (the lock we just built) rather than the full manifest — otherwise
    // resolve_locked_selection errors LockNotFound on members that lack the tag. An explicit
    // selection still validates against the tagged set (a selected-but-untagged member errors).
    let selected = match (&request.target.kind, request.meta.selection.as_ref()) {
        (crate::MaterializeTargetKind::Tag, None) => target_lock.members.keys().cloned().collect(),
        _ => resolve_locked_selection(manifest, &target_lock, request.meta.selection.as_ref())?,
    };
    let destructive_allowed = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.destructive)
        == Some(crate::DestructiveBehavior::Allow);
    let plans = materialize_preflight(
        backend,
        root,
        manifest,
        &target_lock,
        &selected,
        destructive_allowed,
    )?;
    Ok((plans, rewrite_lock))
}

fn apply_materialize_plans<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    plans: Vec<MaterializePlan>,
    rewrite_lock: bool,
    policy: Option<&crate::OperationPolicy>,
    context: crate::operation::OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
{
    // F2: the fresh clones this op will create — rolled back on any mid-batch
    // failure (Q6 reject-partial) so no orphan repos are left behind.
    let fresh_clone_paths: Vec<_> = plans
        .iter()
        .filter(|plan| plan.clone_url.is_some())
        .map(|plan| root.join(&plan.state.path))
        .collect();
    let outcomes = par_map_per_host(
        plans,
        resolve_jobs(policy.and_then(|policy| policy.concurrency)),
        resolve_per_host(policy.and_then(|policy| policy.max_connections_per_host)),
        |plan| plan.clone_url.as_deref().and_then(git_host),
        |plan| -> ModelResult<(String, ResolvedMemberArtifact, crate::MemberResponse)> {
            let member_root = root.join(&plan.state.path);
            emitter.member_started(&plan.member_id, &plan.state.path);
            if let Some(url) = plan.clone_url.as_deref() {
                backend.clone_repo_with_progress(url, &member_root, &|progress| {
                    emitter.member_progress(&plan.member_id, &plan.state.path, progress)
                })?;
            }
            if let Some(commit) = &plan.state.commit {
                // AD3(c): restore onto the saved branch when there was one; detach only
                // when the saved state was genuinely detached. checkout_branch refuses
                // to silently reset a branch that has diverged.
                match &plan.state.branch {
                    Some(branch) if plan.state.detached != Some(true) => {
                        // AD3(c): restore onto the saved branch when safe (creatable or
                        // already at the target). If the branch has diverged, DETACH at
                        // the target instead of resetting it — never orphan its work.
                        match backend.checkout_branch(&member_root, branch, commit) {
                            Ok(_) => {}
                            Err(error) if error.code == ErrorCode::DivergedMember => {
                                backend.checkout_commit(&member_root, commit)?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    _ => {
                        backend.checkout_commit(&member_root, commit)?;
                    }
                }
            }
            emitter.member_finished(&plan.member_id, &plan.state.path);
            // F1: record the OBSERVED post-mutation state (re-read head/status),
            // not the planned target — materialize detaches HEAD at the commit, so
            // the planned branch/detached flags would misdescribe the worktree.
            let member = manifest
                .members
                .iter()
                .find(|member| member.id == plan.member_id)
                .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            let observed = resolved_member(member, &head, &status);
            let response = materialized_response(member, &plan.state, &observed);
            Ok((plan.member_id.clone(), observed, response))
        },
    );
    let mut responses = Vec::with_capacity(outcomes.len());
    let mut observed_states: Vec<(String, ResolvedMemberArtifact)> = Vec::new();
    let mut first_error = None;
    for outcome in outcomes {
        match outcome {
            Ok((member_id, observed, response)) => {
                observed_states.push((member_id, observed));
                responses.push(response);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if let Some(error) = first_error {
        // F2/Q6 reject-partial: a member failed mid-batch. Roll back this op's
        // fresh clones so no orphan repos remain, and write no (stale) lock —
        // failed = nothing changed for the clones this op created.
        for path in &fresh_clone_paths {
            let _ = std::fs::remove_dir_all(path);
        }
        return Err(error);
    }

    if rewrite_lock {
        // F1: write the lock from the observed post-mutation state, not the plan.
        let mut lock = read_lock_or_empty(&root, &manifest.workspace.id)?;
        for (member_id, observed) in &observed_states {
            lock.members.insert(member_id.clone(), observed.clone());
        }
        lock.created_at = now_marker();
        artifact::write_lock(&root, &lock)?;
    }

    // Refresh the workspace boundary (member + tmp excludes) from the authoritative
    // on-disk lock (rewritten above, or the existing one for a lock target).
    let lock = artifact::read_lock(&root)?;
    sync_workspace_boundary(backend, &root, &lock)?;

    Ok(crate::MaterializeResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, responses),
    })
}

fn handle_materialize_branch<B>(
    backend: &B,
    root: PathBuf,
    manifest: ManifestArtifact,
    request: crate::MaterializeRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend,
{
    let branch = request
        .target
        .name
        .as_ref()
        .ok_or_else(|| invalid("branch target requires a name"))?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let plans = branch_switch_preflight(backend, &root, &manifest, &lock, &selected, branch)?;
    if request.meta.dry_run.unwrap_or(false) {
        return Ok(crate::MaterializeResponse {
            response: response_envelope(context, crate::AggregateStatus::Accepted, plans),
        });
    }
    let _guard = WorkspaceMutatorLock::try_acquire(&root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let mut observed_states = Vec::with_capacity(selected.len());
    let mut responses = Vec::with_capacity(selected.len());
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        backend.switch_branch(&member_root, branch)?;
        let head = backend.head(&member_root)?;
        let status = backend.status(&member_root)?;
        let observed = resolved_member(member, &head, &status);
        responses.push(materialized_response(member, &observed, &observed));
        observed_states.push((member_id.clone(), observed));
    }

    let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
    for (member_id, observed) in &observed_states {
        next.members.insert(member_id.clone(), observed.clone());
    }
    next.created_at = now_marker();
    artifact::write_lock(&root, &next)?;
    sync_workspace_boundary(backend, &root, &next)?;

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
/// checking out their locked commits.
pub fn handle_clone_workspace_request<B>(
    backend: &B,
    request: crate::CloneWorkspaceRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::CloneWorkspaceResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::CloneWorkspace(request.clone()).context(operation_id.into())?;
    if request.meta.dry_run.unwrap_or(false) {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "--dry-run is not supported for clone",
        ));
    }
    let target_path = PathBuf::from(&request.target);
    // Refuse to clone over an existing workspace rather than corrupt it.
    if target_path.join(WORKSPACE_MANIFEST).exists() {
        return Err(ModelError::new(
            ErrorCode::WorkspaceAlreadyExists,
            "clone target already contains a GWZ workspace",
        ));
    }
    let progress_interval = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.progress_min_interval_ms)
        .unwrap_or(0);
    let emitter = EventEmitter::new(&context, events, progress_interval);
    emitter.operation_started();
    let response = clone_workspace_with_emitter(backend, request, target_path, context, &emitter);
    emitter.operation_finished();
    response
}

fn clone_workspace_with_emitter<B>(
    backend: &B,
    request: crate::CloneWorkspaceRequest,
    target_path: PathBuf,
    context: crate::operation::OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::CloneWorkspaceResponse>
where
    B: GitBackend + Sync,
{
    let target_display = target_path.to_string_lossy().into_owned();
    // Emit the root repository clone as a member-like lifecycle so consumers can render
    // the full one-shot clone operation without a separate event schema.
    emitter.member_started("workspace_root", &target_display);
    backend.clone_repo_with_progress(&request.url, &target_path, &|progress| {
        emitter.member_progress("workspace_root", &target_display, progress)
    })?;
    emitter.member_finished("workspace_root", &target_display);

    // Verify the cloned repository really is a GWZ workspace before mutating it.
    if !target_path.join(WORKSPACE_MANIFEST).is_file() {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            format!("cloned repository is not a GWZ workspace: {WORKSPACE_MANIFEST} missing"),
        ));
    }

    // Complete the clone: materialize members to the committed lock using the same
    // emitter so event sequence numbers remain monotonic for subscribers.
    let workspace_id = request
        .meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_id.clone());
    let materialize = crate::MaterializeRequest {
        meta: crate::RequestMeta {
            workspace: Some(crate::WorkspaceRef {
                root: Some(target_display),
                workspace_id,
            }),
            ..request.meta
        },
        target: crate::MaterializeTarget {
            kind: crate::MaterializeTargetKind::Lock,
            name: None,
            commit: None,
        },
    };
    let manifest = artifact::read_manifest(&target_path)?;
    assert_workspace_id(&manifest, materialize.meta.workspace.as_ref())?;
    let (plans, rewrite_lock) =
        prepare_materialize_execution(backend, &target_path, &manifest, &materialize)?;
    let response = apply_materialize_plans(
        backend,
        &target_path,
        &manifest,
        plans,
        rewrite_lock,
        materialize.meta.policy.as_ref(),
        context,
        emitter,
    )?;
    Ok(crate::CloneWorkspaceResponse {
        response: response.response,
    })
}

/// Compatibility wrapper for the Rust CLI command path.
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
    handle_clone_workspace_request(
        backend,
        crate::CloneWorkspaceRequest {
            meta,
            url: url.to_owned(),
            target: target.to_owned(),
        },
        operation_id,
        events,
    )
    .map(|response| crate::MaterializeResponse {
        response: response.response,
    })
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

pub(crate) fn observed_member_map<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    // F3 + AD3: capture each member's LIVE observed state (head/status). A member that
    // isn't materialized can't be observed, so carry its existing lock state (AD3 b) —
    // the capture/snapshot stays complete and restorable rather than failing. Dirty
    // state is recorded honestly, never rejected (AD3 a).
    let mut members = BTreeMap::new();
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        if member_root.exists() && backend.is_repository(&member_root)? {
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            members.insert(member_id.clone(), resolved_member(member, &head, &status));
        } else if let Some(state) = lock.members.get(member_id) {
            members.insert(member_id.clone(), state.clone());
        } else {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized and has no lock state"),
            ));
        }
    }
    Ok(members)
}

fn snapshot_member_map<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    source: Option<&crate::SnapshotSource>,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    match source.map(|source| source.kind) {
        None => observed_member_map(backend, root, manifest, lock, selected),
        Some(crate::SnapshotSourceKind::Current) => {
            let members = observed_member_map(backend, root, manifest, lock, selected)?;
            validate_current_snapshot_source(&members)?;
            Ok(members)
        }
        Some(crate::SnapshotSourceKind::Branch) => {
            let branch = source
                .and_then(|source| source.branch.as_ref())
                .ok_or_else(|| invalid("branch snapshot source requires a branch name"))?;
            named_branch_snapshot_members(backend, root, manifest, lock, selected, branch)
        }
    }
}

fn validate_current_snapshot_source(
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> ModelResult<()> {
    let mut branch: Option<&str> = None;
    for (member_id, state) in members {
        if state.detached == Some(true) {
            return Err(ModelError::new(
                ErrorCode::BranchDetachedHead,
                format!("member '{member_id}' is detached"),
            ));
        }
        let Some(current) = state.branch.as_deref() else {
            return Err(ModelError::new(
                ErrorCode::BranchUnbornHead,
                format!("member '{member_id}' has no attached branch"),
            ));
        };
        if state.commit.is_none() {
            return Err(ModelError::new(
                ErrorCode::BranchUnbornHead,
                format!("member '{member_id}' has an unborn HEAD"),
            ));
        }
        if let Some(first) = branch {
            if first != current {
                return Err(ModelError::new(
                    ErrorCode::BranchMixed,
                    "selected members are on different branches",
                ));
            }
        } else {
            branch = Some(current);
        }
    }
    Ok(())
}

fn named_branch_snapshot_members<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    branch: &str,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    let mut members = BTreeMap::new();
    let ref_name = format!("refs/heads/{branch}");
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let lock_state = lock.members.get(member_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{member_id}'"),
            )
        })?;
        let member_root = root.join(&member.path);
        if !member_root.exists() || !backend.is_repository(&member_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized"),
            ));
        }
        let commit = backend.read_ref(&member_root, &ref_name)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("branch '{branch}' not found for member '{member_id}'"),
            )
        })?;
        members.insert(
            member_id.clone(),
            ResolvedMemberArtifact {
                path: lock_state.path.clone(),
                source_id: Some(member.source_id.clone()),
                source_kind: lock_state.source_kind,
                commit: Some(commit),
                branch: Some(branch.to_owned()),
                detached: Some(false),
                upstream: None,
                dirty: Some(false),
                materialized: Some(true),
            },
        );
    }
    Ok(members)
}

fn branch_switch_preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    branch: &str,
) -> ModelResult<Vec<crate::MemberResponse>> {
    let mut plans = Vec::with_capacity(selected.len());
    let ref_name = format!("refs/heads/{branch}");
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let state = lock.members.get(member_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{member_id}'"),
            )
        })?;
        let member_root = root.join(&member.path);
        if !member_root.exists() || !backend.is_repository(&member_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized"),
            ));
        }
        let status = backend.status(&member_root)?;
        if status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                format!("member '{member_id}' has uncommitted changes"),
            ));
        }
        let commit = backend.read_ref(&member_root, &ref_name)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("branch '{branch}' not found for member '{member_id}'"),
            )
        })?;
        plans.push(crate::MemberResponse {
            member_id: member_id.clone(),
            member_path: state.path.clone(),
            source_kind: crate::SourceKind::Git,
            status: crate::MemberStatus::Planned,
            error: None,
            planned: Some(crate::PlannedChange {
                action: crate::PlannedAction::Checkout,
                from_ref: state.branch.clone(),
                to_ref: Some(branch.to_owned()),
                message: None,
            }),
            state: Some(protocol_state(
                member,
                &ResolvedMemberArtifact {
                    commit: Some(commit),
                    branch: Some(branch.to_owned()),
                    detached: Some(false),
                    dirty: Some(false),
                    materialized: Some(true),
                    ..state.clone()
                },
            )),
            git_status: None,
            lock_match: Some(crate::LockMatch::Differs),
        });
    }
    Ok(plans)
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

pub(crate) fn materialize_target_members<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
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
            // Re-meaned (GWZTagPlan): materialize each member to the commit its git tag
            // `refs/tags/<name>` points at. Members lacking the tag are skipped.
            let name = target
                .name
                .as_ref()
                .ok_or_else(|| invalid("tag target requires a name"))?;
            let tag_ref = format!("refs/tags/{name}^{{commit}}");
            let mut targets = BTreeMap::new();
            for member in &manifest.members {
                let member_root = root.join(&member.path);
                if !backend.is_repository(&member_root)? {
                    continue;
                }
                if let Some(commit) = backend.read_ref(&member_root, &tag_ref)? {
                    targets.insert(
                        member.id.clone(),
                        ResolvedMemberArtifact {
                            path: member.path.clone(),
                            commit: Some(commit),
                            ..Default::default()
                        },
                    );
                }
            }
            if targets.is_empty() {
                return Err(ModelError::new(
                    ErrorCode::TagNotFound,
                    format!("tag '{name}' not found in any member"),
                ));
            }
            Ok((targets, true))
        }
        crate::MaterializeTargetKind::Commit
        | crate::MaterializeTargetKind::Head
        | crate::MaterializeTargetKind::Branch => Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "target is not supported here",
        )),
    }
}

pub(crate) fn materialized_response(
    member: &ManifestMember,
    planned: &ResolvedMemberArtifact,
    observed: &ResolvedMemberArtifact,
) -> crate::MemberResponse {
    // F1: lock_match is computed from the observed commit vs the planned target,
    // never claimed unverified.
    let lock_match = if observed.commit == planned.commit {
        crate::LockMatch::Matches
    } else {
        crate::LockMatch::Differs
    };
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: observed.path.clone(),
        source_kind: crate::SourceKind::Git,
        status: crate::MemberStatus::Ok,
        error: None,
        planned: None,
        state: Some(protocol_state(member, observed)),
        git_status: None,
        lock_match: Some(lock_match),
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
