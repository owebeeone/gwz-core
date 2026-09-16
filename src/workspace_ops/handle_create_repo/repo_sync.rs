use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{
    self, ArtifactSourceKind, ManifestMember, RemoteArtifact, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, GitRemote, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

use super::super::*;
use super::invocation::invocation_start;
use super::lock_state::resolved_member;
use super::response::{protocol_state, response_envelope};
use super::validation::assert_workspace_id;

pub fn handle_repo_sync<B>(
    backend: &B,
    start: &Path,
    request: crate::RepoSyncRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::RepoSyncResponse>
where
    B: GitBackend + MergeAuthorityBackend,
{
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let start = invocation_start(start, &request.meta)?;
    handle_repo_sync_in(&services, backend, &start, request, operation_id)
}

pub(crate) fn handle_repo_sync_in<B>(
    services: &crate::operation_context::OperationServices,
    backend: &B,
    start: &Path,
    request: crate::RepoSyncRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::RepoSyncResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::RepoSync(request.clone()).context(operation_id.into())?;
    let dry_run = request.meta.dry_run.unwrap_or(false);
    let (_guard, root) = guarded_workspace_root_for_request_in(
        services,
        start,
        &request.meta,
        OpenMergeCommand::RepoMutate,
        dry_run,
    )?;
    assert_conf_unmodified_for(
        backend,
        &root,
        OpenMergeCommand::RepoMutate,
        reconcile_authority(_guard.as_ref(), dry_run),
    )?;
    let manifest = artifact::read_manifest_in(services.filesystem(), &root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock_in(services.filesystem(), &root)?;
    let selected = resolve_action_ids(
        &manifest,
        request.meta.selection.as_ref(),
        crate::ActionKind::RepoSync,
    )?;
    let destructive_allowed = request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.destructive)
        == Some(crate::DestructiveBehavior::Allow);

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
        match repo_sync_plan_member(
            backend,
            &root,
            member,
            lock.members.get(&member.id),
            dry_run,
            request.private,
            destructive_allowed,
        ) {
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
        artifact::write_manifest_in(services.filesystem(), &root, &next)?;
    }

    let aggregate_status = repo_sync_aggregate(&responses);
    let scheme_only = responses
        .iter()
        .filter(|response| response.url_resolution.is_some())
        .count();
    let mut response = response_envelope(context, aggregate_status, responses);
    if scheme_only > 0 {
        response.meta.message = Some(format!(
            "{scheme_only} member remote(s) differ from the manifest only by URL scheme, a clone-time preference, and keep the recorded URL; pass --force to record the configured form"
        ));
    } else if aggregate_status == crate::AggregateStatus::Noop {
        response.meta.message = Some(
            "Repository metadata already matches local Git configuration; sync does not change worktree contents."
                .to_owned(),
        );
    }
    Ok(crate::RepoSyncResponse { response })
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
    locked: Option<&ResolvedMemberArtifact>,
    dry_run: bool,
    private: Option<bool>,
    destructive_allowed: bool,
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
    if let Some(private) = private {
        next.private = private;
    }
    next.remotes = sync_member_remotes(&member.remotes, &git_remotes);
    // A configured remote that differs from the manifest only by URL scheme on a
    // known host is the mark of a `--url-scheme` clone, not a different
    // repository: keep the manifest URL and report the difference. `--force`
    // records the configured form as before.
    let mut scheme_only = None;
    if !destructive_allowed {
        for synced in &mut next.remotes {
            let Some(recorded) = member
                .remotes
                .iter()
                .find(|remote| remote.name == synced.name)
            else {
                continue;
            };
            if crate::git::scheme_only_difference(&recorded.url, &synced.url) {
                if scheme_only.is_none()
                    && let Some(scheme) = crate::git::written_scheme(&synced.url)
                {
                    scheme_only = Some(crate::MemberUrlResolution {
                        manifest_url: recorded.url.clone(),
                        effective_url: synced.url.clone(),
                        scheme: scheme.into(),
                        source: crate::UrlSchemeSource::Default,
                        derived: true,
                        host_known: true,
                    });
                }
                synced.url = recorded.url.clone();
            }
        }
    }
    if !next.remotes.is_empty() {
        next.desired = Some(desired_from_head(&head));
    }
    let changed = &next != member;
    let state = resolved_member(&next, &head, &status);
    let comparison = crate::status::lock_comparison(locked, Some(&head), Some(&status));
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
            lock_match: Some(comparison.lock_match),
            lock_difference_reasons: (!comparison.reasons.is_empty()).then_some(comparison.reasons),
            url_resolution: scheme_only,
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
            record_context: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Member),
        lock_match: None,
        lock_difference_reasons: None,
        url_resolution: None,
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
