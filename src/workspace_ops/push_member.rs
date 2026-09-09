use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, ManifestMember};
use crate::git::{GitBackend, GitHeadState, MergeAuthorityBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, NullSink, OpenMergeCommand, OperationRequest, par_map_per_host,
    resolve_jobs, resolve_per_host,
};

use super::*;

pub fn handle_push<B>(
    backend: &B,
    start: &Path,
    request: crate::PushRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::PushResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
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
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    let start = invocation_start(start, &request.meta)?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let services = crate::operation_context::OperationServices::for_merge(backend);
    handle_push_with_events_in(&services, backend, &start, request, operation_id, events)
}

pub(crate) fn handle_push_with_events_in<B>(
    services: &crate::operation_context::OperationServices,
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
    let error_context = context.clone();
    let result: ModelResult<crate::PushResponse> = (|| {
        let (_guard, root) = guarded_workspace_root_for_request_in(
            services,
            start,
            &request.meta,
            OpenMergeCommand::Push,
            request.meta.dry_run.unwrap_or(false),
        )?;
        assert_conf_unmodified_for(
            backend,
            &root,
            OpenMergeCommand::Push,
            reconcile_authority(_guard.as_ref(), request.meta.dry_run.unwrap_or(false)),
        )?;
        let manifest = artifact::read_manifest(&root)?;
        assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
        let selected = resolve_action_targets(
            &manifest,
            request.meta.selection.as_ref(),
            crate::ActionKind::Push,
        )?;
        let mut selected_members = Vec::new();
        let mut push_root_selected = false;
        for target in selected {
            match target {
                SelectedTarget::Root => push_root_selected = true,
                SelectedTarget::Member(member) => selected_members.push(member.id.clone()),
            }
        }
        if request
            .meta
            .transport
            .as_ref()
            .is_some_and(|options| !options.remote_identities.is_empty())
        {
            let mut names: Vec<String> = manifest
                .members
                .iter()
                .filter(|member| selected_members.contains(&member.id))
                .filter_map(|member| resolve_push_remote(member, &request).ok())
                .collect();
            if push_root_selected {
                if let Ok(remote) = resolve_root_push_remote(backend, &root, &request) {
                    names.push(remote);
                }
                let pinned = super::publication::freeze_root_request(backend, &root, &request)?;
                names.extend(
                    super::publication::root_dependencies(backend, &root, &pinned)?
                        .into_iter()
                        .map(|dependency| dependency.remote),
                );
            }
            backend.validate_transport_remotes(&names)?;
        }
        if request.meta.dry_run.unwrap_or(false) {
            let mut responses = selected_members
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
            if push_root_selected {
                responses.push(push_root(backend, &root, &request, true));
            }

            return Ok(crate::PushResponse {
                response: response_envelope(context, push_aggregate_status(&responses), responses),
            });
        }

        // F7 (Q2): preflight every selected member before pushing any. The dry-run
        // path validates remote/refspec and materialization without mutating; if any
        // member would be Rejected or Failed, reject the whole push so no remote is
        // advanced. Skipped members are intentional policy, not a failure. A remote
        // rejecting an otherwise-valid push is a genuine push-time outcome and is still
        // reported per member in the loop below.
        let mut preflight = selected_members
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
        if push_root_selected {
            preflight.push(push_root(backend, &root, &request, true));
        }
        if preflight.iter().any(|response| {
            matches!(
                response.status,
                crate::MemberStatus::Rejected | crate::MemberStatus::Failed
            )
        }) {
            return Ok(crate::PushResponse {
                response: response_envelope(context, push_aggregate_status(&preflight), preflight),
            });
        }

        // Capture the exact root object before transfers or user event callbacks.
        let root_request = if push_root_selected {
            Some(super::publication::freeze_root_request(
                backend, &root, &request,
            )?)
        } else {
            None
        };

        let mut plans = std::collections::BTreeMap::new();
        for response in &mut preflight {
            if response.status != crate::MemberStatus::Planned {
                continue;
            }
            let path = root.join(&response.member_path);
            let planned_request = if response.member_id == "@root" {
                root_request.as_ref().expect("selected root captured")
            } else {
                &request
            };
            let remote = if response.member_id == "@root" {
                resolve_root_push_remote(backend, &root, planned_request)
            } else {
                let member = manifest
                    .members
                    .iter()
                    .find(|member| member.id == response.member_id)
                    .expect("selected member");
                resolve_push_remote(member, planned_request)
            };
            let captured = remote.and_then(|remote| {
                let head = backend.head(&path)?;
                let refspec = resolve_push_refspec(&head, planned_request)?;
                backend.prepare_push(&path, &remote, &refspec)
            });
            match captured {
                Ok(plan) => {
                    plans.insert(response.member_id.clone(), plan);
                }
                Err(error) => {
                    response.status = crate::MemberStatus::Rejected;
                    response.planned = None;
                    response.error = Some(crate::GwzError::from(
                        &error.with_member(&response.member_id, &response.member_path),
                    ));
                }
            }
        }
        if preflight
            .iter()
            .any(|row| row.status == crate::MemberStatus::Rejected)
        {
            return Ok(crate::PushResponse {
                response: response_envelope(context, crate::AggregateStatus::Rejected, preflight),
            });
        }

        // No member transfer starts until every selected destination and root-lock
        // dependency has passed read authentication. Aggregate failures per target.
        let mut read_preflight = super::publication::ReadPreflight::default();
        for response in &mut preflight {
            if response.status != crate::MemberStatus::Planned {
                continue;
            }
            let plan = plans
                .get(&response.member_id)
                .expect("selected publication captured");
            let path = root.join(&response.member_path);
            let result = backend
                .ls_remote_url(&path, &plan.url, &plan.remote, Some(&path))
                .inspect(|_advertised| {
                    read_preflight.record(&path, &plan.remote, &plan.url);
                })
                .and_then(|_| {
                    if response.member_id == "@root" {
                        super::publication::preflight_dependencies_with_reads(
                            backend,
                            &root,
                            root_request.as_ref().expect("selected root captured"),
                            &mut read_preflight,
                        )
                    } else {
                        Ok(())
                    }
                });
            if let Err(error) = result {
                response.status = crate::MemberStatus::Rejected;
                response.planned = None;
                response.error = Some(crate::GwzError::from(
                    &error.with_member(&response.member_id, &response.member_path),
                ));
            }
        }
        if preflight
            .iter()
            .any(|response| response.status == crate::MemberStatus::Rejected)
        {
            return Ok(crate::PushResponse {
                response: response_envelope(context, crate::AggregateStatus::Rejected, preflight),
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
        let mut responses = par_map_per_host(
            selected_members,
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
            |member_id| plans.get(member_id).and_then(|plan| git_host(&plan.url)),
            |member_id| {
                let member = manifest
                    .members
                    .iter()
                    .find(|member| member.id == member_id)
                    .ok_or_else(|| {
                        ModelError::new(ErrorCode::MemberNotFound, "member not found")
                    })?;
                emitter.member_started(&member.id, &member.path);
                let mut response = preflight
                    .iter()
                    .find(|row| row.member_id == member.id)
                    .expect("selected preflight row")
                    .clone();
                if let Some(plan) = plans.get(&member.id) {
                    finish_prepared_push(
                        backend.push_prepared(&root.join(&member.path), plan),
                        &mut response,
                    );
                }
                emitter.member_finished(&member.id, &member.path);
                Ok(response)
            },
        )
        .into_iter()
        .collect::<ModelResult<Vec<_>>>()?;
        if push_root_selected {
            emitter.member_started("@root", ".");
            let failed: Vec<&str> = responses
                .iter()
                .filter(|response| {
                    matches!(
                        response.status,
                        crate::MemberStatus::Failed | crate::MemberStatus::Rejected
                    )
                })
                .map(|response| response.member_id.as_str())
                .collect();
            let root_response = if failed.is_empty() {
                let published: std::collections::BTreeMap<_, _> = responses
                    .iter()
                    .filter(|response| response.status == crate::MemberStatus::Ok)
                    .filter_map(|response| {
                        plans
                            .get(&response.member_id)
                            .cloned()
                            .map(|plan| (response.member_id.clone(), plan))
                    })
                    .collect();
                match super::publication::checked_root_request(
                    backend,
                    &root,
                    root_request.as_ref().expect("selected root was captured"),
                    &published,
                ) {
                    Ok(_) => {
                        let mut response = preflight
                            .iter()
                            .find(|row| row.member_id == "@root")
                            .expect("root preflight row")
                            .clone();
                        finish_prepared_push(
                            backend
                                .push_prepared(&root, plans.get("@root").expect("root captured")),
                            &mut response,
                        );
                        response
                    }
                    Err(error) => push_root_error(error, crate::MemberStatus::Rejected),
                }
            } else {
                push_root_error(
                    ModelError::new(
                        ErrorCode::RemoteRejected,
                        format!(
                            "root publication was not attempted because member push failed: {}; resolve the member failures and retry",
                            failed.join(", ")
                        ),
                    ),
                    crate::MemberStatus::Rejected,
                )
            };
            responses.push(root_response);
            emitter.member_finished("@root", ".");
        }
        emitter.operation_finished();

        Ok(crate::PushResponse {
            response: response_envelope(context, push_aggregate_status(&responses), responses),
        })
    })();
    result
        .map_err(|error| super::publication::attach_transport_error(backend, error, &error_context))
        .map(|mut response| {
            super::publication::attach_transport(backend, &mut response.response);
            response
        })
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
    if let Err(error) = backend.validate_remote_identity(&member_root, &remote, true) {
        return push_policy_member_error(member, source_kind, request, error);
    }
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
            target_kind: Some(crate::TargetKind::Member),
            lock_match: None,
        lock_difference_reasons: None,
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
            target_kind: Some(crate::TargetKind::Member),
            lock_match: None,
        lock_difference_reasons: None,
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

pub(crate) fn push_root<B>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
    dry_run: bool,
) -> crate::MemberResponse
where
    B: GitBackend,
{
    let is_repo = match backend.is_repository(root) {
        Ok(is_repo) => is_repo,
        Err(error) => return push_root_error(error, crate::MemberStatus::Failed),
    };
    if !is_repo {
        return push_root_error(
            ModelError::new(
                ErrorCode::MemberNotFound,
                "workspace root is not a git repository",
            ),
            crate::MemberStatus::Rejected,
        );
    }
    let remote = match resolve_root_push_remote(backend, root, request) {
        Ok(remote) => remote,
        Err(error) => return push_root_error(error, crate::MemberStatus::Rejected),
    };
    let head = match backend.head(root) {
        Ok(head) => head,
        Err(error) => return push_root_error(error, crate::MemberStatus::Failed),
    };
    let refspec = match resolve_push_refspec(&head, request) {
        Ok(refspec) => refspec,
        Err(error) => return push_root_error(error, crate::MemberStatus::Rejected),
    };
    if let Err(error) = backend.validate_remote_identity(root, &remote, true) {
        return push_root_error(error, crate::MemberStatus::Rejected);
    }
    if dry_run {
        let dependencies = super::publication::freeze_root_request(backend, root, request)
            .and_then(|pinned| super::publication::root_dependencies(backend, root, &pinned));
        let validated = dependencies.and_then(|dependencies| {
            for dependency in dependencies {
                super::publication::validate_dependency_identity(backend, &dependency)?;
            }
            Ok(())
        });
        if let Err(error) = validated {
            return push_root_error(error, crate::MemberStatus::Rejected);
        }
        return crate::MemberResponse {
            member_id: "@root".to_owned(),
            member_path: ".".to_owned(),
            source_kind: crate::SourceKind::Git,
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
            target_kind: Some(crate::TargetKind::Root),
            lock_match: None,
        lock_difference_reasons: None,
        };
    }

    match backend.push(root, &remote, &refspec) {
        Ok(_) => crate::MemberResponse {
            member_id: "@root".to_owned(),
            member_path: ".".to_owned(),
            source_kind: crate::SourceKind::Git,
            status: crate::MemberStatus::Ok,
            error: None,
            planned: None,
            state: None,
            git_status: None,
            target_kind: Some(crate::TargetKind::Root),
            lock_match: None,
        lock_difference_reasons: None,
        },
        Err(error) if error.code == ErrorCode::MissingRemote => {
            push_root_error(error, crate::MemberStatus::Failed)
        }
        Err(error) => push_root_error(
            ModelError::new(ErrorCode::RemoteRejected, error.message),
            crate::MemberStatus::Failed,
        ),
    }
}

pub(crate) fn resolve_root_push_remote<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::PushRequest,
) -> ModelResult<String> {
    if let Some(remote) = request.remote.clone().or_else(|| {
        request
            .meta
            .policy
            .as_ref()
            .and_then(|policy| policy.remote.clone())
    }) {
        return Ok(remote);
    }

    backend
        .remotes(root)?
        .into_iter()
        .find(|remote| remote.push_url.is_some() || remote.url.is_some())
        .map(|remote| remote.name)
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MissingRemote,
                "workspace root has no push remote",
            )
        })
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

pub(crate) fn resolve_push_refspec(
    head: &GitHeadState,
    request: &crate::PushRequest,
) -> ModelResult<String> {
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
    }
}

pub(crate) fn push_root_error(
    error: ModelError,
    status: crate::MemberStatus,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: "@root".to_owned(),
        member_path: ".".to_owned(),
        source_kind: crate::SourceKind::Git,
        status,
        error: Some(crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some("@root".to_owned()),
            member_path: Some(".".to_owned()),
            target_kind: Some(crate::TargetKind::Root),
            detail: None,
            record_context: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        target_kind: Some(crate::TargetKind::Root),
        lock_match: None,
        lock_difference_reasons: None,
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

pub(crate) fn artifact_source_kind_to_protocol(
    source_kind: ArtifactSourceKind,
) -> crate::SourceKind {
    match source_kind {
        ArtifactSourceKind::Git => crate::SourceKind::Git,
        ArtifactSourceKind::Archive => crate::SourceKind::Archive,
        ArtifactSourceKind::Package => crate::SourceKind::Package,
        ArtifactSourceKind::Local => crate::SourceKind::Local,
        ArtifactSourceKind::Generated => crate::SourceKind::Generated,
    }
}

fn finish_prepared_push(
    result: ModelResult<crate::git::GitPushResult>,
    response: &mut crate::MemberResponse,
) {
    response.planned = None;
    match result {
        Ok(_) => response.status = crate::MemberStatus::Ok,
        Err(error) => {
            response.status = crate::MemberStatus::Failed;
            let error = if error.code == ErrorCode::MissingRemote {
                error
            } else {
                ModelError::new(ErrorCode::RemoteRejected, error.message)
            };
            response.error = Some(crate::GwzError::from(
                &error.with_member(&response.member_id, &response.member_path),
            ));
        }
    }
}
