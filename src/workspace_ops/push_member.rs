use std::path::Path;

use crate::artifact::{
    self, ArtifactSourceKind,
    ManifestArtifact, ManifestMember,
};
use crate::git::{GitBackend, GitHeadState, git_host};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, NullSink, OperationRequest, par_map_per_host, resolve_jobs,
    resolve_per_host,
};
use crate::workspace::MemberPath;


use super::*;

pub fn handle_push<B>(
    backend: &B,
    start: &Path,
    request: crate::PushRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::PushResponse>
where
    B: GitBackend + Sync,
{
    handle_push_with_events(backend, start, request, operation_id, &NullSink)
}

pub fn handle_push_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::PushRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PushResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::Push(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let selected = resolve_manifest_selection(&manifest, request.meta.selection.as_ref())?;
    if request.meta.dry_run.unwrap_or(false) {
        let responses = selected
            .iter()
            .map(|member_id| {
                let member = manifest
                    .members
                    .iter()
                    .find(|member| &member.id == member_id)
                    .ok_or_else(|| {
                        ModelError::new(ErrorCode::MemberNotFound, "member not found")
                    })?;
                Ok(push_member(backend, &root, member, &request, true))
            })
            .collect::<ModelResult<Vec<_>>>()?;

        return Ok(crate::PushResponse {
            response: response_envelope(context, push_aggregate_status(&responses), responses),
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
    let responses = par_map_per_host(
        selected,
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
        |member_id| {
            manifest
                .members
                .iter()
                .find(|member| member.id == *member_id)
                .and_then(|member| push_remote_host(member, &request))
        },
        |member_id| {
            let member = manifest
                .members
                .iter()
                .find(|member| member.id == member_id)
                .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
            emitter.member_started(&member.id, &member.path);
            let response = push_member(backend, &root, member, &request, false);
            emitter.member_finished(&member.id, &member.path);
            Ok(response)
        },
    )
    .into_iter()
    .collect::<ModelResult<Vec<_>>>()?;
    emitter.operation_finished();

    Ok(crate::PushResponse {
        response: response_envelope(context, push_aggregate_status(&responses), responses),
    })
}

pub(crate) fn resolve_manifest_selection(
    manifest: &ManifestArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<String>> {
    match selection {
        None => Ok(manifest
            .members
            .iter()
            .filter(|member| member.active)
            .map(|member| member.id.clone())
            .collect::<Vec<_>>()),
        Some(selection) => resolve_explicit_locked_selection(manifest, selection),
    }
}

pub(crate) fn resolve_explicit_locked_selection(
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

pub(crate) fn find_active_member_by_id<'a>(
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

pub(crate) fn find_active_member_by_path<'a>(
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

pub(crate) fn push_remote_host(member: &ManifestMember, request: &crate::PushRequest) -> Option<String> {
    let remote = resolve_push_remote(member, request).ok()?;
    member
        .remotes
        .iter()
        .find(|candidate| candidate.name == remote)
        .and_then(|candidate| git_host(&candidate.url))
}

pub(crate) fn push_member<B>(
    backend: &B,
    root: &Path,
    member: &ManifestMember,
    request: &crate::PushRequest,
    dry_run: bool,
) -> crate::MemberResponse
where
    B: GitBackend,
{
    let source_kind = artifact_source_kind_to_protocol(member.source_kind);
    if member.source_kind != ArtifactSourceKind::Git {
        return push_policy_member_error(
            member,
            source_kind,
            request,
            ModelError::new(
                ErrorCode::UnsupportedSourceKind,
                "push supports git members only",
            ),
        );
    }

    let remote = match resolve_push_remote(member, request) {
        Ok(remote) => remote,
        Err(error) => return push_policy_member_error(member, source_kind, request, error),
    };
    let member_root = root.join(&member.path);
    let is_repo = match backend.is_repository(&member_root) {
        Ok(is_repo) => is_repo,
        Err(error) => {
            return push_member_error(member, source_kind, error, crate::MemberStatus::Failed);
        }
    };
    if !is_repo {
        return push_member_error(
            member,
            source_kind,
            ModelError::new(ErrorCode::MemberNotFound, "member is not materialized"),
            crate::MemberStatus::Rejected,
        );
    }

    let head = match backend.head(&member_root) {
        Ok(head) => head,
        Err(error) => {
            return push_member_error(member, source_kind, error, crate::MemberStatus::Failed);
        }
    };
    let refspec = match resolve_push_refspec(&head, request) {
        Ok(refspec) => refspec,
        Err(error) => return push_policy_member_error(member, source_kind, request, error),
    };
    if dry_run {
        return crate::MemberResponse {
            member_id: member.id.clone(),
            member_path: member.path.clone(),
            source_kind,
            status: crate::MemberStatus::Planned,
            error: None,
            planned: Some(crate::PlannedChange {
                action: crate::PlannedAction::Push,
                from_ref: head.commit.clone(),
                to_ref: Some(refspec),
                message: Some(format!("push to {remote}")),
            }),
            state: None,
            git_status: None,
            lock_match: None,
        };
    }

    match backend.push(&member_root, &remote, &refspec) {
        Ok(_) => crate::MemberResponse {
            member_id: member.id.clone(),
            member_path: member.path.clone(),
            source_kind,
            status: crate::MemberStatus::Ok,
            error: None,
            planned: None,
            state: None,
            git_status: None,
            lock_match: None,
        },
        Err(error) if error.code == ErrorCode::MissingRemote => {
            push_member_error(member, source_kind, error, crate::MemberStatus::Failed)
        }
        Err(error) => push_member_error(
            member,
            source_kind,
            ModelError::new(ErrorCode::RemoteRejected, error.message),
            crate::MemberStatus::Failed,
        ),
    }
}

pub(crate) fn resolve_push_remote(
    member: &ManifestMember,
    request: &crate::PushRequest,
) -> ModelResult<String> {
    request
        .remote
        .clone()
        .or_else(|| {
            request
                .meta
                .policy
                .as_ref()
                .and_then(|policy| policy.remote.clone())
        })
        .or_else(|| {
            member
                .remotes
                .iter()
                .find(|remote| remote.push)
                .map(|remote| remote.name.clone())
        })
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "member has no push remote"))
}

pub(crate) fn resolve_push_refspec(head: &GitHeadState, request: &crate::PushRequest) -> ModelResult<String> {
    if let Some(refspec) = &request.refspec {
        return Ok(refspec.clone());
    }
    let branch = head.branch.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "push refspec is required for detached members",
        )
    })?;
    if head.commit.is_none() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "cannot push a branch without commits",
        ));
    }
    Ok(format!("refs/heads/{branch}:refs/heads/{branch}"))
}

pub(crate) fn push_policy_member_error(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    request: &crate::PushRequest,
    error: ModelError,
) -> crate::MemberResponse {
    if request
        .meta
        .policy
        .as_ref()
        .and_then(|policy| policy.unsupported_member)
        == Some(crate::UnsupportedMemberBehavior::Skip)
    {
        push_member_error(member, source_kind, error, crate::MemberStatus::Skipped)
    } else {
        push_member_error(member, source_kind, error, crate::MemberStatus::Rejected)
    }
}

pub(crate) fn push_member_error(
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
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        lock_match: None,
    }
}

pub(crate) fn push_aggregate_status(responses: &[crate::MemberResponse]) -> crate::AggregateStatus {
    let has_ok = responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Ok);
    let has_failed = responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Failed);
    let has_rejected = responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Rejected);
    let has_skipped = responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Skipped);
    if has_ok && (has_failed || has_rejected || has_skipped) {
        crate::AggregateStatus::Partial
    } else if has_failed {
        crate::AggregateStatus::Failed
    } else if has_rejected {
        crate::AggregateStatus::Rejected
    } else if has_skipped
        || responses
            .iter()
            .all(|response| response.status == crate::MemberStatus::Noop)
    {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Ok
    }
}

pub(crate) fn artifact_source_kind_to_protocol(source_kind: ArtifactSourceKind) -> crate::SourceKind {
    match source_kind {
        ArtifactSourceKind::Git => crate::SourceKind::Git,
        ArtifactSourceKind::Archive => crate::SourceKind::Archive,
        ArtifactSourceKind::Package => crate::SourceKind::Package,
        ArtifactSourceKind::Local => crate::SourceKind::Local,
        ArtifactSourceKind::Generated => crate::SourceKind::Generated,
    }
}

