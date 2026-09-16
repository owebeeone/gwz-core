use std::path::Path;

use crate::artifact::{self, ManifestArtifact, ManifestMember, ResolvedMemberArtifact};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, par_map_per_host, resolve_jobs, resolve_per_host};

use super::super::*;
use super::materialize::MaterializeApplyOptions;

pub(super) fn apply_materialize_plans<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    plans: Vec<MaterializePlan>,
    options: MaterializeApplyOptions<'_>,
    context: crate::operation::OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend + Sync,
{
    let MaterializeApplyOptions {
        rewrite_lock,
        skip_private_access,
        follow_branch_head,
        policy,
        url_scheme,
    } = options;
    // F2: the fresh clones this op will create — rolled back on any mid-batch
    // failure (Q6 reject-partial) so no orphan repos are left behind.
    let mut fresh_clone_paths = Vec::new();
    for plan in plans.iter().filter(|plan| plan.clone_url.is_some()) {
        let path = root.join(&plan.state.path);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fresh_clone_paths.push(path);
            }
            Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
        }
    }
    let outcomes = par_map_per_host(
        plans,
        resolve_jobs(policy.and_then(|policy| policy.concurrency)),
        resolve_per_host(policy.and_then(|policy| policy.max_connections_per_host)),
        |plan| plan.clone_url.as_deref().and_then(git_host),
        |plan| -> ModelResult<Option<(String, ResolvedMemberArtifact, crate::MemberResponse)>> {
            let member_root = root.join(&plan.state.path);
            let member = manifest
                .members
                .iter()
                .find(|member| member.id == plan.member_id)
                .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
            let quiet_clone = skip_private_access && member.private && plan.clone_url.is_some();
            if !quiet_clone {
                emitter.member_started(&plan.member_id, &plan.state.path);
            }
            if let Some(url) = plan.clone_url.as_deref() {
                let remote = materialize_clone_remote(manifest, &plan.member_id)?;
                let existed_before = !fresh_clone_paths.contains(&member_root);
                let clone =
                    backend.clone_repo_named(url, &member_root, &remote.name, &|progress| {
                        if !quiet_clone {
                            emitter.member_progress(&plan.member_id, &plan.state.path, progress);
                        }
                    });
                match clone {
                    Err(error)
                        if quiet_clone
                            && !existed_before
                            && error.code == ErrorCode::RemoteRejected =>
                    {
                        match std::fs::remove_dir_all(&member_root) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => {
                                return Err(ModelError::new(ErrorCode::IoError, error.to_string()));
                            }
                        }
                        if let Some(observations) = backend.transport_observations() {
                            observations.forget_private_clone(&member_root);
                        }
                        return Ok(None);
                    }
                    Err(error) => {
                        return Err(with_member_context(
                            error,
                            member,
                            plan.clone_url.as_deref(),
                            url_scheme,
                        ));
                    }
                    Ok(_) => {}
                }
                if quiet_clone {
                    emitter.member_started(&plan.member_id, &plan.state.path);
                }
            }
            match &plan.state.branch {
                Some(branch) if plan.state.detached != Some(true) => {
                    // AD3(c): the member tracks `branch` (detached:false). Pick the checkout
                    // target by intent:
                    //   - follow_branch_head (lock / clone): attach at the branch's UPSTREAM
                    //     HEAD — the live tip. The lock `commit` is an observation that may
                    //     lag the branch (a stale lock), so honoring it would detach; honoring
                    //     the branch is what makes `clone` land on the branch like git does.
                    //   - else (snapshot / tag): pin the recorded `commit` exactly so the
                    //     captured point stays reproducible.
                    // Fall back to the lock commit when no upstream ref resolves (e.g. a
                    // local-only branch with no remote).
                    let target = if follow_branch_head {
                        let fetch_remote = member
                            .remotes
                            .iter()
                            .find(|remote| remote.fetch)
                            .map(|remote| remote.name.as_str())
                            .unwrap_or("origin");
                        match backend.read_ref(&member_root, &format!("{fetch_remote}/{branch}"))? {
                            Some(head) => Some(head),
                            None => plan.state.commit.clone(),
                        }
                    } else {
                        plan.state.commit.clone()
                    };
                    if let Some(target) = target {
                        // AD3(c) orphan-safety: attach the branch at the target when safe
                        // (creatable or already there). If the LOCAL branch has diverged —
                        // unpushed work not at the target — DETACH at the target rather than
                        // reset it, never orphaning that work.
                        match backend.checkout_branch(&member_root, branch, &target) {
                            Ok(_) => {}
                            Err(error) if error.code == ErrorCode::DivergedMember => {
                                backend.checkout_commit(&member_root, &target)?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                // detached:true or no branch → pinned: reproduce the exact lock commit.
                _ => {
                    if let Some(commit) = &plan.state.commit {
                        backend.checkout_commit(&member_root, commit)?;
                    }
                }
            }
            emitter.member_finished(&plan.member_id, &plan.state.path);
            // F1: record the OBSERVED post-mutation state (re-read head/status), not the
            // planned target — the worktree may attach to the branch head or detach, so the
            // planned branch/detached flags would misdescribe it.
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            let observed = resolved_member(member, &head, &status);
            let response =
                materialized_response(member, &plan.state, &observed, plan.url_resolution.clone());
            Ok(Some((plan.member_id.clone(), observed, response)))
        },
    );
    let mut responses = Vec::with_capacity(outcomes.len());
    let mut observed_states: Vec<(String, ResolvedMemberArtifact)> = Vec::new();
    let mut first_error = None;
    for outcome in outcomes {
        match outcome {
            Ok(Some((member_id, observed, response))) => {
                observed_states.push((member_id, observed));
                responses.push(response);
            }
            Ok(None) => {}
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
        let mut lock = read_lock_or_empty(root, &manifest.workspace.id)?;
        for (member_id, observed) in &observed_states {
            lock.members.insert(member_id.clone(), observed.clone());
        }
        artifact::write_lock(root, &lock)?;
    }

    // Refresh the workspace boundary (member + tmp excludes) from the authoritative
    // on-disk lock (rewritten above, or the existing one for a lock target).
    let lock = artifact::read_lock(root)?;
    sync_workspace_boundary(backend, root, manifest, &lock)?;

    Ok(crate::MaterializeResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, responses),
    })
}

pub(crate) fn materialized_response(
    member: &ManifestMember,
    planned: &ResolvedMemberArtifact,
    observed: &ResolvedMemberArtifact,
    url_resolution: Option<crate::MemberUrlResolution>,
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
        target_kind: Some(crate::TargetKind::Member),
        lock_match: Some(lock_match),
        lock_difference_reasons: None,
        url_resolution,
    }
}

/// Attributes a member clone failure to its member and appends the URL-scheme
/// remedy when the failure is one the alternate scheme would sidestep.
fn with_member_context(
    mut error: ModelError,
    member: &ManifestMember,
    clone_url: Option<&str>,
    url_scheme: crate::git::UrlScheme,
) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(member.id.clone());
    }
    if error.member_path.is_none() {
        error.member_path = Some(member.path.clone());
    }
    if let Some(url) = clone_url {
        error.message = append_url_scheme_hint(std::mem::take(&mut error.message), url, url_scheme);
    }
    error
}

pub(super) fn materialize_clone_remote<'a>(
    manifest: &'a ManifestArtifact,
    id: &str,
) -> ModelResult<&'a crate::artifact::RemoteArtifact> {
    manifest
        .members
        .iter()
        .find(|member| member.id == id)
        .and_then(|member| member.remotes.iter().find(|remote| remote.fetch))
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MissingRemote,
                "materialize clone has no declared fetch remote",
            )
        })
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
