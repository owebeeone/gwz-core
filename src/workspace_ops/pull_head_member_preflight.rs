use std::path::Path;

use crate::artifact::{
    self, LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, NullSink, OperationRequest, WorkspaceMutatorLock, par_map_per_host,
    resolve_jobs, resolve_per_host,
};

use super::*;

pub fn handle_pull_head<B>(
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + Sync,
{
    handle_pull_head_with_events(backend, start, request, operation_id, &NullSink)
}

pub fn handle_pull_head_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::PullHead(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let _guard = if dry_run {
        None
    } else {
        Some(WorkspaceMutatorLock::acquire(&root)?)
    };
    let manifest_for_selection = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest_for_selection, request.meta.workspace.as_ref())?;
    let selected_for_root = resolve_targets(
        &manifest_for_selection,
        request.meta.selection.as_ref(),
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;
    let pull_root_selected = selected_for_root
        .iter()
        .any(|target| matches!(target, SelectedTarget::Root));
    let root_changed = if dry_run || !pull_root_selected {
        false
    } else {
        pull_workspace_root(backend, &root, request.meta.policy.as_ref())?
    };
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let mut lock = artifact::read_lock(&root)?;
    let selected_targets = resolve_targets(
        &manifest,
        request.meta.selection.as_ref(),
        CommandDefaultTargets::All,
        RootSelectionPolicy::Allow,
    )?;
    let mut selected = Vec::new();
    for target in selected_targets {
        if let SelectedTarget::Member(member) = target {
            if !lock.members.contains_key(&member.id) {
                return Err(ModelError::new(
                    ErrorCode::LockNotFound,
                    format!("lock record missing for member '{}'", member.id),
                ));
            }
            selected.push(member.id.clone());
        }
    }
    if dry_run {
        let plans = pull_head_preflight(
            backend,
            &root,
            &manifest,
            &lock,
            &selected,
            request.meta.policy.as_ref(),
            None,
        )?;
        return Ok(crate::PullHeadResponse {
            response: response_envelope(
                context,
                pull_aggregate_status(&plans),
                plans.iter().map(PullHeadPlan::planned_response).collect(),
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
    let plans = pull_head_preflight(
        backend,
        &root,
        &manifest,
        &lock,
        &selected,
        request.meta.policy.as_ref(),
        Some(&emitter),
    )?;
    let mut responses = Vec::with_capacity(plans.len());
    for plan in plans {
        let member_root = root.join(&plan.state.path);
        let conflicts = apply_pull_action(backend, &member_root, &plan)?;
        let member = manifest
            .members
            .iter()
            .find(|member| member.id == plan.member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let state = if backend.is_repository(&member_root)? {
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            resolved_member(member, &head, &status)
        } else {
            plan.state.clone()
        };
        lock.members.insert(plan.member_id.clone(), state.clone());
        responses.push(pull_result_response(
            member,
            &state,
            &plan.action,
            &conflicts,
        ));
    }
    artifact::write_lock(&root, &lock)?;
    sync_workspace_boundary(backend, &root, &manifest, &lock)?;
    emitter.operation_finished();

    Ok(crate::PullHeadResponse {
        response: response_envelope(
            context,
            pull_response_aggregate(&responses, root_changed),
            responses,
        ),
    })
}

fn pull_workspace_root<B>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<bool>
where
    B: GitBackend,
{
    if !backend.is_repository(root)? {
        return Ok(false);
    }
    let Some(remote) = pull_root_remote_name(backend, root, policy)? else {
        return Ok(false);
    };
    let head = backend.head(root)?;
    if head.is_detached {
        return Err(ModelError::new(
            ErrorCode::BranchDetachedHead,
            "workspace root is detached; root pull requires an attached branch",
        ));
    }
    let branch = head.branch.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::BranchUnbornHead,
            "workspace root has no current branch",
        )
    })?;
    let local_commit = head.commit.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::BranchUnbornHead,
            "workspace root has an unborn HEAD",
        )
    })?;
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    let status = backend.status(root)?;
    pull_dirty_guard(sync, &status, policy, "workspace root")?;

    let manifest_before = artifact::read_manifest(root)?;
    let fallback_lock = read_lock_or_empty(root, &manifest_before.workspace.id)?;

    backend.fetch(root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(root, &remote_ref)?
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "root remote branch not found"))?;
    if local_commit == remote_commit {
        return Ok(false);
    }
    let behind = backend.is_ancestor(root, &local_commit, &remote_commit)?;
    match sync {
        crate::SyncBehavior::FetchOnly => Ok(false),
        crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
            if !behind {
                if root_remote_changes_are_auto_repairable(
                    backend,
                    root,
                    &local_commit,
                    &remote_commit,
                )? {
                    let result = backend.merge_upstream(root, &branch, &remote_ref)?;
                    if result.conflicts.is_empty() {
                        rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                        return Ok(true);
                    }
                    resolve_repairable_root_conflicts(
                        backend,
                        root,
                        &branch,
                        &remote_ref,
                        &fallback_lock,
                        &result.conflicts,
                    )?;
                    return Ok(true);
                } else {
                    return Err(ModelError::new(
                        ErrorCode::DivergedMember,
                        "workspace root has diverged from remote; rerun with --sync merge, rebase, or reset",
                    ));
                }
            }
            backend.fast_forward(root, &branch, &remote_ref)?;
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
        crate::SyncBehavior::Merge => {
            if behind {
                backend.fast_forward(root, &branch, &remote_ref)?;
                rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                return Ok(true);
            }
            let result = backend.merge_upstream(root, &branch, &remote_ref)?;
            if result.conflicts.is_empty() {
                rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                return Ok(true);
            }
            resolve_repairable_root_conflicts(
                backend,
                root,
                &branch,
                &remote_ref,
                &fallback_lock,
                &result.conflicts,
            )?;
            Ok(true)
        }
        crate::SyncBehavior::Rebase => {
            if behind {
                backend.fast_forward(root, &branch, &remote_ref)?;
            } else {
                let result = backend.rebase_onto(root, &branch, &remote_ref)?;
                if !result.conflicts.is_empty() {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!(
                            "workspace root rebase left conflicted paths: {}",
                            result.conflicts.join(", ")
                        ),
                    ));
                }
            }
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
        crate::SyncBehavior::Reset => {
            backend.reset_hard(root, &branch, &remote_ref)?;
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
    }
}

fn root_remote_changes_are_auto_repairable<B>(
    backend: &B,
    root: &Path,
    local_commit: &str,
    remote_commit: &str,
) -> ModelResult<bool>
where
    B: GitBackend,
{
    let Some(base) = backend.merge_base(root, local_commit, remote_commit)? else {
        return Ok(false);
    };
    let remote_paths = backend.changed_paths_between(root, &base, remote_commit)?;
    Ok(!remote_paths.is_empty() && remote_paths.iter().all(|path| path == artifact::LOCK_PATH))
}

fn pull_root_remote_name<B>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<Option<String>>
where
    B: GitBackend,
{
    if let Some(remote) = policy.and_then(|policy| policy.remote.clone()) {
        return Ok(Some(remote));
    }
    let remotes = backend.remotes(root)?;
    Ok(remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first())
        .map(|remote| remote.name.clone()))
}

fn resolve_repairable_root_conflicts<B>(
    backend: &B,
    root: &Path,
    branch: &str,
    remote_ref: &str,
    fallback_lock: &LockArtifact,
    conflicts: &[String],
) -> ModelResult<()>
where
    B: GitBackend,
{
    let supported = conflicts
        .iter()
        .all(|path| path == artifact::LOCK_PATH || path == ".gitignore");
    if !supported {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "workspace root merge left non-GWZ conflicted paths: {}",
                conflicts.join(", ")
            ),
        ));
    }

    if conflicts.iter().any(|path| path == artifact::LOCK_PATH) {
        rewrite_root_lock_from_live_members_allowing_other_conflicts(backend, root, fallback_lock)?;
    }
    let remaining = conflicts
        .iter()
        .filter(|path| path.as_str() != artifact::LOCK_PATH)
        .cloned()
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "workspace root merge left user-resolved conflicted paths: {}",
                remaining.join(", ")
            ),
        ));
    }

    backend.commit_merge_resolution(root, &format!("Merge {remote_ref} into {branch}"))?;
    Ok(())
}

fn rewrite_root_lock_from_live_members_allowing_other_conflicts<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let selected = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.id.clone())
        .collect::<Vec<_>>();
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let members = observed_member_map(backend, root, &manifest, &lock_fallback, &selected)?;
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members,
    };
    artifact::write_lock(root, &lock)?;
    backend.stage_paths_allowing_other_conflicts(root, &[artifact::LOCK_PATH])?;
    Ok(lock)
}

fn rewrite_root_lock_from_live_members<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let selected = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.id.clone())
        .collect::<Vec<_>>();
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let members = observed_member_map(backend, root, &manifest, &lock_fallback, &selected)?;
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members,
    };
    artifact::write_lock(root, &lock)?;
    sync_workspace_boundary(backend, root, &manifest, &lock)?;
    Ok(lock)
}

pub(crate) const NO_FETCH_REMOTE_PULL_MESSAGE: &str = "no fetch remote configured; skipping pull";
pub(crate) const FETCH_ONLY_PULL_MESSAGE: &str = "fetched; not integrated (fetch-only)";

pub(crate) enum PullHeadAction {
    Noop,
    SkipNoFetchRemote,
    /// Fetched but deliberately not integrated (`--sync fetch-only`).
    FetchOnly,
    FastForward {
        remote_ref: String,
    },
    Merge {
        remote_ref: String,
    },
    Rebase {
        remote_ref: String,
    },
    Reset {
        remote_ref: String,
    },
}

impl PullHeadAction {
    pub(crate) fn is_noop(&self) -> bool {
        matches!(self, Self::Noop | Self::SkipNoFetchRemote)
    }

    pub(crate) fn planned_message(&self) -> Option<String> {
        match self {
            Self::SkipNoFetchRemote => Some(NO_FETCH_REMOTE_PULL_MESSAGE.to_owned()),
            Self::FetchOnly => Some(FETCH_ONLY_PULL_MESSAGE.to_owned()),
            Self::Noop
            | Self::FastForward { .. }
            | Self::Merge { .. }
            | Self::Rebase { .. }
            | Self::Reset { .. } => None,
        }
    }
}

pub(crate) struct PullHeadPlan {
    pub(crate) member_id: String,
    pub(crate) branch: String,
    pub(crate) state: ResolvedMemberArtifact,
    pub(crate) action: PullHeadAction,
}

impl PullHeadPlan {
    pub(crate) fn planned_response(&self) -> crate::MemberResponse {
        crate::MemberResponse {
            member_id: self.member_id.clone(),
            member_path: self.state.path.clone(),
            source_kind: crate::SourceKind::Git,
            status: match self.action {
                PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => {
                    crate::MemberStatus::Noop
                }
                PullHeadAction::FetchOnly
                | PullHeadAction::FastForward { .. }
                | PullHeadAction::Merge { .. }
                | PullHeadAction::Rebase { .. }
                | PullHeadAction::Reset { .. } => crate::MemberStatus::Planned,
            },
            error: None,
            planned: Some(crate::PlannedChange {
                action: match self.action {
                    PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => {
                        crate::PlannedAction::Noop
                    }
                    PullHeadAction::FetchOnly => crate::PlannedAction::Fetch,
                    PullHeadAction::FastForward { .. } => crate::PlannedAction::FastForward,
                    PullHeadAction::Merge { .. } => crate::PlannedAction::Merge,
                    PullHeadAction::Rebase { .. } => crate::PlannedAction::Rebase,
                    PullHeadAction::Reset { .. } => crate::PlannedAction::Reset,
                },
                from_ref: self.state.commit.clone(),
                to_ref: None,
                message: self.action.planned_message(),
            }),
            state: None,
            git_status: None,
            target_kind: Some(crate::TargetKind::Member),
            lock_match: None,
        }
    }
}

pub(crate) fn pull_head_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    policy: Option<&crate::OperationPolicy>,
    emitter: Option<&EventEmitter<'_>>,
) -> ModelResult<Vec<PullHeadPlan>>
where
    B: GitBackend + Sync,
{
    let global = resolve_jobs(policy.and_then(|policy| policy.concurrency));
    let per_host = resolve_per_host(policy.and_then(|policy| policy.max_connections_per_host));

    // Q1: probe every member's remote with non-mutating `ls_remote` and BARRIER before any
    // fetch. A batch rejected here — unreachable remote, missing branch, dirty, or
    // unmaterialized member — has advanced no remote-tracking refs. (Divergence needs the
    // fetched objects, so a diverged-member rejection in the fetch pass below may already
    // have fetched — that residual is acknowledged, not prevented.)
    par_map_per_host(
        selected.to_vec(),
        global,
        per_host,
        |member_id| {
            manifest
                .members
                .iter()
                .find(|member| member.id == *member_id)
                .and_then(|member| pull_remote_host(member, policy))
        },
        |member_id| pull_validate_member(backend, root, manifest, lock, member_id, policy),
    )
    .into_iter()
    .collect::<ModelResult<Vec<()>>>()?;

    par_map_per_host(
        selected.to_vec(),
        global,
        per_host,
        |member_id| {
            manifest
                .members
                .iter()
                .find(|member| member.id == *member_id)
                .and_then(|member| pull_remote_host(member, policy))
        },
        |member_id| {
            pull_head_member_preflight(backend, root, manifest, lock, member_id, policy, emitter)
        },
    )
    .into_iter()
    .collect()
}

/// Validate a member for pull WITHOUT fetching (Q1): confirm it is materialized, passes the
/// dirty guard, and — via non-mutating `ls_remote` — that its remote is reachable and
/// advertises the target branch. Local-only / no-fetch-remote members are valid no-ops.
pub(crate) fn pull_validate_member<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    member_id: String,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<()>
where
    B: GitBackend,
{
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
    let state = lock.members.get(&member_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::LockNotFound,
            format!("lock record missing for member '{member_id}'"),
        )
    })?;
    if member
        .desired
        .as_ref()
        .and_then(|desired| desired.local_only)
        == Some(true)
    {
        return Ok(());
    }
    let member_root = root.join(&state.path);
    if !backend.is_repository(&member_root)? {
        return Err(ModelError::new(
            ErrorCode::MemberNotFound,
            format!("member '{member_id}' is not materialized"),
        ));
    }
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    let status = backend.status(&member_root)?;
    pull_dirty_guard(sync, &status, policy, &member_id)?;
    let Some(remote) = pull_fetch_remote_name(member, policy) else {
        return Ok(());
    };
    let branch = pull_branch(member, state);
    let branch_ref = format!("refs/heads/{branch}");
    if !backend
        .ls_remote(&member_root, &remote)?
        .iter()
        .any(|advertised| advertised.name == branch_ref)
    {
        return Err(ModelError::new(
            ErrorCode::MissingRemote,
            format!(
                "remote '{remote}' does not advertise branch '{branch}' for member '{member_id}'"
            ),
        ));
    }
    Ok(())
}

/// Resolve the branch a member is pinned to: the locked branch, else the desired branch,
/// else `main`.
pub(crate) fn pull_branch(member: &ManifestMember, state: &ResolvedMemberArtifact) -> String {
    state
        .branch
        .clone()
        .or_else(|| {
            member
                .desired
                .as_ref()
                .and_then(|desired| desired.branch.clone())
        })
        .unwrap_or_else(|| "main".to_owned())
}

/// The sync-mode-aware dirty guard: fetch-only tolerates a dirty member (it never touches
/// the worktree), reset is gated on the destructive policy, and the integrating modes refuse
/// (matching porcelain git's "local changes would be overwritten").
pub(crate) fn pull_dirty_guard(
    sync: crate::SyncBehavior,
    status: &crate::git::GitStatus,
    policy: Option<&crate::OperationPolicy>,
    member_id: &str,
) -> ModelResult<()> {
    if !status.is_dirty {
        return Ok(());
    }
    match sync {
        crate::SyncBehavior::FetchOnly => Ok(()),
        crate::SyncBehavior::Reset
            if policy.and_then(|policy| policy.destructive)
                == Some(crate::DestructiveBehavior::Allow) =>
        {
            Ok(())
        }
        crate::SyncBehavior::Reset => Err(ModelError::new(
            ErrorCode::DirtyMember,
            format!("member '{member_id}' has uncommitted changes; reset would discard them"),
        )),
        _ => Err(ModelError::new(
            ErrorCode::DirtyMember,
            format!("member '{member_id}' has uncommitted changes"),
        )),
    }
}

pub(crate) fn pull_head_member_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    member_id: String,
    policy: Option<&crate::OperationPolicy>,
    emitter: Option<&EventEmitter<'_>>,
) -> ModelResult<PullHeadPlan>
where
    B: GitBackend,
{
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
    let state = lock.members.get(&member_id).cloned().ok_or_else(|| {
        ModelError::new(
            ErrorCode::LockNotFound,
            format!("lock record missing for member '{member_id}'"),
        )
    })?;
    let branch = pull_branch(member, &state);
    if let Some(emitter) = emitter {
        emitter.member_started(&member.id, &state.path);
    }
    if member
        .desired
        .as_ref()
        .and_then(|desired| desired.local_only)
        == Some(true)
    {
        if let Some(emitter) = emitter {
            emitter.member_finished(&member.id, &state.path);
        }
        return Ok(PullHeadPlan {
            member_id,
            branch,
            state,
            action: PullHeadAction::Noop,
        });
    }

    let member_root = root.join(&state.path);
    if !backend.is_repository(&member_root)? {
        return Err(ModelError::new(
            ErrorCode::MemberNotFound,
            format!("member '{member_id}' is not materialized"),
        ));
    }
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    let status = backend.status(&member_root)?;
    pull_dirty_guard(sync, &status, policy, &member_id)?;
    let Some(remote) = pull_fetch_remote_name(member, policy) else {
        if let Some(emitter) = emitter {
            emitter.member_finished(&member.id, &state.path);
        }
        return Ok(PullHeadPlan {
            member_id,
            branch,
            state,
            action: PullHeadAction::SkipNoFetchRemote,
        });
    };
    backend.fetch(&member_root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(&member_root, &remote_ref)?
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "remote branch not found"))?;
    let head = backend.head(&member_root)?;
    let Some(local_commit) = head.commit.clone() else {
        return Err(ModelError::new(
            ErrorCode::DivergedMember,
            "cannot fast-forward unborn member",
        ));
    };
    let action = if local_commit == remote_commit {
        PullHeadAction::Noop
    } else {
        // Strictly behind ⇒ a fast-forward is always available, and the integration
        // modes take it too (git merge/rebase fast-forward by default). Diverged ⇒
        // the chosen sync mode decides; fetch-only never integrates either way.
        let behind = backend.is_ancestor(&member_root, &local_commit, &remote_commit)?;
        match sync {
            crate::SyncBehavior::FetchOnly => PullHeadAction::FetchOnly,
            crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    return Err(ModelError::new(
                        ErrorCode::DivergedMember,
                        format!("member '{member_id}' has diverged from remote"),
                    ));
                }
            }
            crate::SyncBehavior::Merge => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    PullHeadAction::Merge { remote_ref }
                }
            }
            crate::SyncBehavior::Rebase => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    PullHeadAction::Rebase { remote_ref }
                }
            }
            crate::SyncBehavior::Reset => PullHeadAction::Reset { remote_ref },
        }
    };
    if let Some(emitter) = emitter {
        emitter.member_finished(&member.id, &state.path);
    }
    Ok(PullHeadPlan {
        member_id,
        branch,
        state,
        action,
    })
}

pub(crate) fn pull_fetch_remote_name(
    member: &ManifestMember,
    policy: Option<&crate::OperationPolicy>,
) -> Option<String> {
    policy
        .and_then(|policy| policy.remote.as_ref())
        .cloned()
        .or_else(|| {
            member
                .remotes
                .iter()
                .find(|remote| remote.fetch)
                .map(|remote| remote.name.clone())
        })
}

pub(crate) fn pull_remote_host(
    member: &ManifestMember,
    policy: Option<&crate::OperationPolicy>,
) -> Option<String> {
    let remote = pull_fetch_remote_name(member, policy)?;
    member
        .remotes
        .iter()
        .find(|candidate| candidate.name == remote)
        .and_then(|candidate| git_host(&candidate.url))
}

/// Execute a planned pull action against the materialized member, returning any
/// conflicted paths (empty when clean or non-integrating). Merge/rebase conflicts are
/// RETURNED, not errored — the worktree is left `--continue`-able for the developer.
pub(crate) fn apply_pull_action<B>(
    backend: &B,
    member_root: &Path,
    plan: &PullHeadPlan,
) -> ModelResult<Vec<String>>
where
    B: GitBackend,
{
    match &plan.action {
        PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote | PullHeadAction::FetchOnly => {
            Ok(Vec::new())
        }
        PullHeadAction::FastForward { remote_ref } => {
            backend.fast_forward(member_root, &plan.branch, remote_ref)?;
            Ok(Vec::new())
        }
        PullHeadAction::Merge { remote_ref } => Ok(backend
            .merge_upstream(member_root, &plan.branch, remote_ref)?
            .conflicts),
        PullHeadAction::Rebase { remote_ref } => Ok(backend
            .rebase_onto(member_root, &plan.branch, remote_ref)?
            .conflicts),
        PullHeadAction::Reset { remote_ref } => {
            backend.reset_hard(member_root, &plan.branch, remote_ref)?;
            Ok(Vec::new())
        }
    }
}

pub(crate) fn pull_result_response(
    member: &ManifestMember,
    state: &ResolvedMemberArtifact,
    action: &PullHeadAction,
    conflicts: &[String],
) -> crate::MemberResponse {
    let status = if conflicts.is_empty() {
        match action {
            PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => crate::MemberStatus::Noop,
            PullHeadAction::FetchOnly
            | PullHeadAction::FastForward { .. }
            | PullHeadAction::Merge { .. }
            | PullHeadAction::Rebase { .. }
            | PullHeadAction::Reset { .. } => crate::MemberStatus::Ok,
        }
    } else {
        crate::MemberStatus::Conflicted
    };
    let message = if conflicts.is_empty() {
        action.planned_message()
    } else {
        Some(format!(
            "integration left {} conflicted path(s); resolve and continue: {}",
            conflicts.len(),
            conflicts.join(", ")
        ))
    };
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: state.path.clone(),
        source_kind: crate::SourceKind::Git,
        status,
        error: None,
        planned: message.map(|message| crate::PlannedChange {
            action: crate::PlannedAction::Noop,
            from_ref: state.commit.clone(),
            to_ref: None,
            message: Some(message),
        }),
        state: Some(protocol_state(member, state)),
        git_status: None,
        // A conflicted worktree no longer matches the lock; don't claim it does.
        target_kind: Some(crate::TargetKind::Member),
        lock_match: conflicts.is_empty().then_some(crate::LockMatch::Matches),
    }
}

pub(crate) fn pull_aggregate_status(plans: &[PullHeadPlan]) -> crate::AggregateStatus {
    if plans.iter().all(|plan| plan.action.is_noop()) {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Accepted
    }
}

pub(crate) fn pull_response_aggregate(
    responses: &[crate::MemberResponse],
    root_changed: bool,
) -> crate::AggregateStatus {
    if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Conflicted)
    {
        crate::AggregateStatus::Conflicted
    } else if responses
        .iter()
        .all(|response| response.status == crate::MemberStatus::Noop)
    {
        if root_changed {
            crate::AggregateStatus::Ok
        } else {
            crate::AggregateStatus::Noop
        }
    } else {
        crate::AggregateStatus::Ok
    }
}
