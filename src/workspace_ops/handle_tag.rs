use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

use super::*;

/// Manage git tags across the workspace — the multi-repo `git tag` (GWZTagPlan). Tags are real
/// git refs (`refs/tags/<name>`) fanned out to the selected members + the root, mirroring how
/// `gwz commit` fans out `git commit`. `create`/`list`/`delete` are local; `fetch`/`push` (and
/// `list`/`delete` against a `--remote`) are remote.
pub fn handle_tag<B>(
    backend: &B,
    start: &std::path::Path,
    request: crate::TagRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::TagResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Tag(request.clone()).context(operation_id.into())?;
    let services = crate::operation_context::OperationServices::existing();
    let scoped_backend = backend.with_transport(start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let error_context = context.clone();
    let result: ModelResult<crate::TagResponse> = (|| {
        let dry_run = request.meta.dry_run.unwrap_or(false);
        let (_access, root) = if request.op == crate::TagOp::List {
            (
                None,
                resolve_workspace_root(start, request.meta.workspace.as_ref())?,
            )
        } else {
            let access = acquire_workspace_mutation_guard_in(
                &services,
                start,
                request.meta.workspace.as_ref(),
                OpenMergeCommand::TagMutate,
                dry_run,
            )?;
            let root = access.root().to_path_buf();
            (Some(access), root)
        };
        if let Some(access) = _access.as_ref() {
            assert_conf_unmodified_for(
                backend,
                &root,
                OpenMergeCommand::TagMutate,
                access.writes(),
            )?;
        }
        let manifest = artifact::read_manifest(&root)?;
        assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
        let lock = artifact::read_lock(&root)?;
        let selected = resolve_locked_action_selection(
            &manifest,
            &lock,
            request.meta.selection.as_ref(),
            crate::ActionKind::Tag,
        )?;
        let mut member_roots: Vec<PathBuf> = Vec::new();
        for member_id in &selected {
            if member_id == "@root" {
                member_roots.push(root.clone());
                continue;
            }
            let member = manifest
                .members
                .iter()
                .find(|member| &member.id == member_id)
                .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
            member_roots.push(root.join(&member.path));
        }
        let repos: Vec<PathBuf> = member_roots.clone();
        let push_plans = if request.op == crate::TagOp::Push {
            plan_tag_pushes(backend, &member_roots, request.name.as_deref())?
        } else {
            Vec::new()
        };
        let uses_remote = matches!(request.op, crate::TagOp::Push | crate::TagOp::Fetch)
            || (matches!(request.op, crate::TagOp::List | crate::TagOp::Delete)
                && request.remote.is_some());
        let prepared_pushes = if !dry_run && request.op == crate::TagOp::Push {
            let remote = request.remote.as_deref().unwrap_or("origin");
            push_plans
                .iter()
                .map(|(repo, refspec)| backend.prepare_push(repo, remote, refspec))
                .collect::<ModelResult<Vec<_>>>()?
        } else {
            Vec::new()
        };
        if uses_remote {
            let remote = request.remote.as_deref().unwrap_or("origin");
            let mut names = Vec::new();
            for repo in &repos {
                if backend.is_repository(repo)? {
                    names.push(remote.to_owned());
                }
            }
            for (repo, refspec) in &push_plans {
                if repo == &root {
                    let frozen = crate::PushRequest {
                        meta: request.meta.clone(),
                        remote: Some(remote.into()),
                        refspec: Some(refspec.clone()),
                    };
                    for dependency in
                        super::publication::root_dependencies(backend, &root, &frozen)?
                    {
                        super::publication::validate_dependency_identity(backend, &dependency)?;
                        names.push(dependency.remote);
                    }
                }
            }
            backend.validate_transport_remotes(&names)?;
            for repo in &repos {
                if backend.is_repository(repo)? {
                    backend.validate_remote_identity(
                        repo,
                        remote,
                        matches!(request.op, crate::TagOp::Push | crate::TagOp::Delete),
                    )?;
                }
            }
            if !dry_run {
                for repo in &repos {
                    if backend.is_repository(repo)? {
                        if request.op == crate::TagOp::Push {
                            if let Some(index) =
                                push_plans.iter().position(|(path, _)| path == repo)
                            {
                                let plan = &prepared_pushes[index];
                                backend.ls_remote_url(repo, &plan.url, &plan.remote, Some(repo))?;
                            }
                        } else {
                            super::publication::preflight_remote(
                                backend,
                                repo,
                                remote,
                                request.op == crate::TagOp::Delete,
                            )?;
                        }
                    }
                }
                for (repo, refspec) in &push_plans {
                    if repo == &root {
                        super::publication::preflight_dependencies(
                            backend,
                            &root,
                            &crate::PushRequest {
                                meta: request.meta.clone(),
                                remote: Some(remote.into()),
                                refspec: Some(refspec.clone()),
                            },
                        )?;
                    }
                }
            }
        }

        match request.op {
            crate::TagOp::Create => {
                let git_name = require_name(&request)?;
                // `git tag -s` with no message fails non-interactively; reject it with one clear
                // error instead of an opaque per-repo git failure during the fan-out.
                if request.signed.unwrap_or(false) && request.message.is_none() {
                    return Err(ModelError::new(
                        ErrorCode::InvalidRequest,
                        "a signed tag requires a message (-m)",
                    ));
                }
                // Validation is done; a dry run stops before the fan-out.
                if dry_run {
                    return ok_envelope(context);
                }
                for repo in &repos {
                    // Tag a repo only if it has a commit (skip unborn) and does not already carry the
                    // tag — keeping create idempotent and symmetric with delete/push.
                    if backend.is_repository(repo)?
                        && backend.head(repo)?.commit.is_some()
                        && !backend.tag_list(repo)?.contains(&git_name)
                    {
                        backend
                            .tag_create(
                                repo,
                                &git_name,
                                request.message.as_deref(),
                                request.signed.unwrap_or(false),
                            )
                            .map_err(tag_error)?;
                    }
                }
                ok_envelope(context)
            }
            crate::TagOp::Delete => {
                let git_name = require_name(&request)?;
                // Validation is done; a dry run stops before the fan-out — including the
                // `ls_remote` probe the remote arm would otherwise make.
                if dry_run {
                    return ok_envelope(context);
                }
                match request.remote.as_deref() {
                    // Remote delete: push a delete refspec to each member's remote that has the tag.
                    Some(remote) => {
                        let delete_refspec = format!(":refs/tags/{git_name}");
                        for member_root in &member_roots {
                            if backend.is_repository(member_root)?
                                && remote_has_tag(backend, member_root, remote, &git_name)?
                            {
                                backend
                                    .push(member_root, remote, &delete_refspec)
                                    .map_err(tag_error)?;
                            }
                        }
                        ok_envelope(context)
                    }
                    None => {
                        for repo in &repos {
                            if backend.is_repository(repo)?
                                && backend.tag_list(repo)?.contains(&git_name)
                            {
                                backend.tag_delete(repo, &git_name).map_err(tag_error)?;
                            }
                        }
                        ok_envelope(context)
                    }
                }
            }
            crate::TagOp::List => match request.remote.as_deref() {
                // Remote list: ls-remote each member and keep the tag refs.
                Some(remote) => {
                    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
                    for member_root in &member_roots {
                        if !backend.is_repository(member_root)? {
                            continue;
                        }
                        for advertised in
                            backend.ls_remote(member_root, remote).map_err(tag_error)?
                        {
                            if let Some(name) = remote_tag_name(&advertised.name) {
                                *counts.entry(name).or_insert(0) += 1;
                            }
                        }
                    }
                    Ok(list_response(context, counts))
                }
                // Local list: count every tag across root + members.
                None => {
                    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
                    for repo in &repos {
                        if !backend.is_repository(repo)? {
                            continue;
                        }
                        for full in backend.tag_list(repo)? {
                            *counts.entry(full).or_insert(0) += 1;
                        }
                    }
                    Ok(list_response(context, counts))
                }
            },
            crate::TagOp::Push => {
                let remote = request.remote.as_deref().unwrap_or("origin");
                // A dry run stops before any remote traffic.
                if dry_run {
                    return ok_envelope(context);
                }
                // Capture every tag object before the first transfer. Root checks below
                // inspect that captured object even if a local tag moves meanwhile.
                for ((member_root, refspec), plan) in push_plans.into_iter().zip(prepared_pushes) {
                    if member_root == root {
                        super::publication::checked_root_request(
                            backend,
                            &root,
                            &crate::PushRequest {
                                meta: request.meta.clone(),
                                remote: Some(remote.into()),
                                refspec: Some(refspec),
                            },
                        )?;
                    }
                    backend
                        .push_prepared(&member_root, &plan)
                        .map_err(tag_error)?;
                }
                ok_envelope(context)
            }
            crate::TagOp::Fetch => {
                let remote = request.remote.as_deref().unwrap_or("origin");
                // A dry run stops before any remote traffic.
                if dry_run {
                    return ok_envelope(context);
                }
                for member_root in &member_roots {
                    if backend.is_repository(member_root)? {
                        backend.tag_fetch(member_root, remote).map_err(tag_error)?;
                    }
                }
                ok_envelope(context)
            }
        }
    })();
    result
        .map_err(|error| super::publication::attach_transport_error(backend, error, &error_context))
        .map(|mut response| {
            super::publication::attach_transport(backend, &mut response.response);
            response
        })
}

/// Map an advertised remote ref (`refs/tags/v1`) to its bare tag name (`v1`), skipping peeled
/// `^{}` entries and any non-tag ref (heads, HEAD).
fn remote_tag_name(ref_name: &str) -> Option<String> {
    if ref_name.ends_with("^{}") {
        return None;
    }
    ref_name.strip_prefix("refs/tags/").map(str::to_owned)
}

/// Whether `remote` advertises `refs/tags/<git_name>`.
fn remote_has_tag<B: GitBackend>(
    backend: &B,
    path: &std::path::Path,
    remote: &str,
    git_name: &str,
) -> ModelResult<bool> {
    let target = format!("refs/tags/{git_name}");
    Ok(backend
        .ls_remote(path, remote)
        .map_err(tag_error)?
        .iter()
        .any(|advertised| advertised.name == target))
}

fn list_response(
    context: crate::operation::OperationContext,
    counts: BTreeMap<String, i64>,
) -> crate::TagResponse {
    let tags = counts
        .into_iter()
        .map(|(name, members)| crate::TagInfo { name, members })
        .collect();
    crate::TagResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        tags: Some(tags),
    }
}

fn require_name(request: &crate::TagRequest) -> ModelResult<String> {
    request
        .name
        .clone()
        .ok_or_else(|| ModelError::new(ErrorCode::InvalidRequest, "a tag name is required"))
}

fn ok_envelope(context: crate::operation::OperationContext) -> ModelResult<crate::TagResponse> {
    Ok(crate::TagResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        tags: None,
    })
}

/// Enumerate concrete tag objects without transferring anything. Keep annotated
/// tag objects intact and preserve the resolver's member-before-root ordering.
pub(super) fn plan_tag_pushes<B: GitBackend>(
    backend: &B,
    repos: &[PathBuf],
    name: Option<&str>,
) -> ModelResult<Vec<(PathBuf, String)>> {
    let mut plans = Vec::new();
    for repo in repos {
        if !backend.is_repository(repo)? {
            continue;
        }
        for tag in backend.tag_list(repo)? {
            if name.is_some_and(|name| name != tag) {
                continue;
            }
            let reference = format!("refs/tags/{tag}");
            let object = backend.read_ref(repo, &reference)?.ok_or_else(|| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    "tag disappeared while preparing publication; retry",
                )
            })?;
            plans.push((repo.clone(), format!("{object}:{reference}")));
        }
    }
    Ok(plans)
}
