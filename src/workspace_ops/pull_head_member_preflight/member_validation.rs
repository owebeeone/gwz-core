use std::path::Path;

use crate::artifact::{LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, par_map_per_host, resolve_jobs, resolve_per_host};

use super::super::pull_head_plan::PullHeadPlan;
use super::member_preflight::{
    pull_fetch_remote_name, pull_head_member_preflight, pull_remote_host,
};

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
