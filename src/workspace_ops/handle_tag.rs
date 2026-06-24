use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationRequest;

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
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;

    // Local ops (create/list/delete) span the workspace root (D2) + selected members; remote
    // ops (push/fetch, and list/delete against a `--remote`) span the members only, since the
    // workspace root is local-only.
    let mut member_roots: Vec<PathBuf> = Vec::new();
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        member_roots.push(root.join(&member.path));
    }
    let mut repos: Vec<PathBuf> = vec![root.clone()];
    repos.extend(member_roots.iter().cloned());

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
        crate::TagOp::Delete => match request.remote.as_deref() {
            // Remote delete: push a delete refspec to each member's remote that has the tag.
            Some(remote) => {
                let git_name = require_name(&request)?;
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
                let git_name = require_name(&request)?;
                for repo in &repos {
                    if backend.is_repository(repo)? && backend.tag_list(repo)?.contains(&git_name) {
                        backend.tag_delete(repo, &git_name).map_err(tag_error)?;
                    }
                }
                ok_envelope(context)
            }
        },
        crate::TagOp::List => match request.remote.as_deref() {
            // Remote list: ls-remote each member and keep the tag refs.
            Some(remote) => {
                let mut counts: BTreeMap<String, i64> = BTreeMap::new();
                for member_root in &member_roots {
                    if !backend.is_repository(member_root)? {
                        continue;
                    }
                    for advertised in backend.ls_remote(member_root, remote).map_err(tag_error)? {
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
            for member_root in &member_roots {
                if !backend.is_repository(member_root)? {
                    continue;
                }
                // libgit2 does NOT expand a glob refspec on push, so resolve concrete tags first:
                // a named tag pushes itself (when present); no name pushes every tag — each via
                // its own concrete refspec.
                let to_push: Vec<String> = match &request.name {
                    Some(name) => {
                        if backend.tag_list(member_root)?.contains(name) {
                            vec![name.clone()]
                        } else {
                            Vec::new()
                        }
                    }
                    None => backend.tag_list(member_root)?,
                };
                for git_name in to_push {
                    let refspec = format!("refs/tags/{git_name}:refs/tags/{git_name}");
                    backend
                        .push(member_root, remote, &refspec)
                        .map_err(tag_error)?;
                }
            }
            ok_envelope(context)
        }
        crate::TagOp::Fetch => {
            let remote = request.remote.as_deref().unwrap_or("origin");
            for member_root in &member_roots {
                if backend.is_repository(member_root)? {
                    backend.tag_fetch(member_root, remote).map_err(tag_error)?;
                }
            }
            ok_envelope(context)
        }
    }
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
