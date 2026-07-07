use std::path::{Path, PathBuf};

use crate::artifact::{self, ManifestArtifact, ManifestMember};
use crate::git::{GitBackend, GitBranch};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OperationContext, OperationRequest, WorkspaceMutatorLock};

use super::*;

pub fn handle_branch<B>(
    backend: &B,
    start: &Path,
    request: crate::BranchRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::BranchResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Branch(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let repos = selected_member_repos(backend, &root, &manifest, &selected)?;

    match request.op {
        crate::BranchOp::List => list_branches(backend, context, &repos),
        crate::BranchOp::Create => create_branch(backend, &root, request, context, &repos),
        crate::BranchOp::Delete => delete_branch(backend, &root, request, context, &repos),
        crate::BranchOp::Merge => merge_branch(backend, &root, request, context, &repos),
    }
}

#[derive(Clone)]
struct BranchRepo {
    member_id: String,
    member_path: String,
    member: ManifestMember,
    path: PathBuf,
}

#[derive(Clone)]
struct CreatePlan {
    repo: BranchRepo,
    start_commit: String,
    existed: bool,
}

#[derive(Clone)]
struct MergePlan {
    repo: BranchRepo,
    current_branch: String,
    head_commit: String,
    source_ref: String,
    source_commit: String,
}

fn selected_member_repos<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    selected: &[String],
) -> ModelResult<Vec<BranchRepo>> {
    let mut repos = Vec::with_capacity(selected.len());
    for member_id in selected {
        let member = manifest_member(manifest, member_id)?;
        let path = root.join(&member.path);
        if !path.exists() || !backend.is_repository(&path)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized"),
            ));
        }
        repos.push(BranchRepo {
            member_id: member.id.clone(),
            member_path: member.path.clone(),
            member: member.clone(),
            path,
        });
    }
    Ok(repos)
}

fn list_branches<B: GitBackend>(
    backend: &B,
    context: OperationContext,
    repos: &[BranchRepo],
) -> ModelResult<crate::BranchResponse> {
    let mut summaries = Vec::new();
    for repo in repos {
        let head = backend.head(&repo.path)?;
        let branches = backend.branch_list(&repo.path)?;
        if branches.is_empty() {
            summaries.push(summary_from_head(
                repo,
                crate::BranchActionResult::Listed,
                None,
                &head,
            ));
            continue;
        }
        for branch in branches {
            summaries.push(summary_from_branch(
                repo,
                crate::BranchActionResult::Listed,
                &branch,
                head.branch.as_deref(),
                head.is_detached,
                head.commit.is_none(),
            ));
        }
    }
    Ok(branch_response(
        context,
        crate::AggregateStatus::Ok,
        summaries,
        Vec::new(),
    ))
}

fn create_branch<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: crate::BranchRequest,
    context: OperationContext,
    repos: &[BranchRepo],
) -> ModelResult<crate::BranchResponse> {
    let branch = require_branch_name(&request)?;
    let start_ref = request.start_ref.as_deref().unwrap_or("HEAD");
    let switch_after_create = request.switch_after_create.unwrap_or(false);
    let plans = create_preflight(backend, repos, &branch, start_ref, switch_after_create)?;

    if request.meta.dry_run.unwrap_or(false) {
        let summaries = plans
            .iter()
            .map(|plan| {
                dry_run_summary(
                    &plan.repo,
                    &branch,
                    plan.start_commit.clone(),
                    if plan.existed {
                        crate::BranchActionResult::Exists
                    } else {
                        crate::BranchActionResult::Created
                    },
                )
            })
            .collect();
        return Ok(branch_response(
            context,
            crate::AggregateStatus::Accepted,
            summaries,
            Vec::new(),
        ));
    }

    let _guard = WorkspaceMutatorLock::try_acquire(root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let mut created_by_this_op: Vec<PathBuf> = Vec::new();
    let mut summaries = Vec::with_capacity(plans.len());
    let mut observed_states = Vec::new();
    for plan in &plans {
        let created = match backend.branch_create(&plan.repo.path, &branch, start_ref) {
            Ok(result) => result.created,
            Err(error) => {
                rollback_created_branches(backend, &created_by_this_op, &branch);
                return Ok(partial_response(context, summaries, &error));
            }
        };
        if created {
            created_by_this_op.push(plan.repo.path.clone());
        }

        if switch_after_create {
            if let Err(error) = backend.switch_branch(&plan.repo.path, &branch) {
                rollback_created_branches(backend, &created_by_this_op, &branch);
                return Ok(partial_response(context, summaries, &error));
            }
            let head = backend.head(&plan.repo.path)?;
            let status = backend.status(&plan.repo.path)?;
            let observed = resolved_member(&plan.repo.member, &head, &status);
            observed_states.push((plan.repo.member_id.clone(), observed.clone()));
            summaries.push(summary_from_head(
                &plan.repo,
                crate::BranchActionResult::Switched,
                Some(branch.clone()),
                &head,
            ));
        } else {
            let head = backend.head(&plan.repo.path)?;
            summaries.push(summary_from_head(
                &plan.repo,
                if created {
                    crate::BranchActionResult::Created
                } else {
                    crate::BranchActionResult::Exists
                },
                Some(branch.clone()),
                &head,
            ));
        }
    }

    if switch_after_create {
        let manifest = artifact::read_manifest(root)?;
        let mut next = read_lock_or_empty(root, &manifest.workspace.id)?;
        for (member_id, observed) in &observed_states {
            next.members.insert(member_id.clone(), observed.clone());
        }
        artifact::write_lock(root, &next)?;
        sync_workspace_boundary(backend, root, &next)?;
    }

    Ok(branch_response(
        context,
        crate::AggregateStatus::Ok,
        summaries,
        Vec::new(),
    ))
}

fn delete_branch<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: crate::BranchRequest,
    context: OperationContext,
    repos: &[BranchRepo],
) -> ModelResult<crate::BranchResponse> {
    let branch = require_branch_name(&request)?;
    delete_preflight(backend, repos, &branch)?;

    if request.meta.dry_run.unwrap_or(false) {
        let summaries = repos
            .iter()
            .map(|repo| {
                dry_run_summary(
                    repo,
                    &branch,
                    String::new(),
                    crate::BranchActionResult::Deleted,
                )
            })
            .collect();
        return Ok(branch_response(
            context,
            crate::AggregateStatus::Accepted,
            summaries,
            Vec::new(),
        ));
    }

    let _guard = WorkspaceMutatorLock::try_acquire(root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let mut summaries = Vec::with_capacity(repos.len());
    for repo in repos {
        if let Err(error) = backend.branch_delete(&repo.path, &branch) {
            return Ok(partial_response(context, summaries, &error));
        }
        let head = backend.head(&repo.path)?;
        summaries.push(summary_from_head(
            repo,
            crate::BranchActionResult::Deleted,
            Some(branch.clone()),
            &head,
        ));
    }
    Ok(branch_response(
        context,
        crate::AggregateStatus::Ok,
        summaries,
        Vec::new(),
    ))
}

fn merge_branch<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: crate::BranchRequest,
    context: OperationContext,
    repos: &[BranchRepo],
) -> ModelResult<crate::BranchResponse> {
    let source_ref = request
        .start_ref
        .clone()
        .ok_or_else(|| invalid("a source ref is required"))?;
    let plans = merge_preflight(backend, repos, &source_ref)?;

    if request.meta.dry_run.unwrap_or(false) {
        let summaries = plans
            .iter()
            .map(|plan| merge_planned_summary(plan, crate::BranchActionResult::Merged))
            .collect();
        return Ok(branch_response(
            context,
            crate::AggregateStatus::Accepted,
            summaries,
            Vec::new(),
        ));
    }

    let _guard = WorkspaceMutatorLock::try_acquire(root)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::UnsupportedOperation,
            "workspace mutator lock is already held",
        )
    })?;

    let mut summaries = Vec::with_capacity(plans.len());
    let mut observed_states = Vec::new();
    let mut saw_conflict = false;
    let mut mutated = false;
    for plan in &plans {
        let result =
            match backend.merge_upstream(&plan.repo.path, &plan.current_branch, &plan.source_ref) {
                Ok(result) => result,
                Err(error) if mutated => return Ok(partial_response(context, summaries, &error)),
                Err(error) => return Err(error),
            };
        mutated = true;

        if result.is_clean() {
            let head = match backend.head(&plan.repo.path) {
                Ok(head) => head,
                Err(error) => return Ok(partial_response(context, summaries, &error)),
            };
            let status = match backend.status(&plan.repo.path) {
                Ok(status) => status,
                Err(error) => return Ok(partial_response(context, summaries, &error)),
            };
            let observed = resolved_member(&plan.repo.member, &head, &status);
            observed_states.push((plan.repo.member_id.clone(), observed));
            summaries.push(merge_result_summary(
                plan,
                crate::BranchActionResult::Merged,
                head.commit.clone(),
                result.commit,
                Vec::new(),
            ));
        } else {
            saw_conflict = true;
            summaries.push(merge_result_summary(
                plan,
                crate::BranchActionResult::Conflicted,
                Some(plan.head_commit.clone()),
                None,
                result.conflicts,
            ));
        }
    }

    if !observed_states.is_empty() {
        let manifest = match artifact::read_manifest(root) {
            Ok(manifest) => manifest,
            Err(error) => return Ok(partial_response(context, summaries, &error)),
        };
        let mut next = match read_lock_or_empty(root, &manifest.workspace.id) {
            Ok(lock) => lock,
            Err(error) => return Ok(partial_response(context, summaries, &error)),
        };
        for (member_id, observed) in &observed_states {
            next.members.insert(member_id.clone(), observed.clone());
        }
        if let Err(error) = artifact::write_lock(root, &next) {
            return Ok(partial_response(context, summaries, &error));
        }
        if let Err(error) = sync_workspace_boundary(backend, root, &next) {
            return Ok(partial_response(context, summaries, &error));
        }
    }

    Ok(branch_response(
        context,
        if saw_conflict {
            crate::AggregateStatus::Conflicted
        } else {
            crate::AggregateStatus::Ok
        },
        summaries,
        Vec::new(),
    ))
}

fn create_preflight<B: GitBackend>(
    backend: &B,
    repos: &[BranchRepo],
    branch: &str,
    start_ref: &str,
    switch_after_create: bool,
) -> ModelResult<Vec<CreatePlan>> {
    let mut plans = Vec::with_capacity(repos.len());
    let branch_ref = format!("refs/heads/{branch}");
    for repo in repos {
        let status = backend.status(&repo.path)?;
        if switch_after_create && status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                format!("member '{}' has uncommitted changes", repo.member_id),
            ));
        }
        let start_commit = backend.read_ref(&repo.path, start_ref)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "start ref '{start_ref}' not found for member '{}'",
                    repo.member_id
                ),
            )
        })?;
        let existing = backend.read_ref(&repo.path, &branch_ref)?;
        if let Some(existing_commit) = existing {
            if existing_commit != start_commit {
                return Err(ModelError::new(
                    ErrorCode::DivergedMember,
                    format!(
                        "branch '{branch}' for member '{}' is at {existing_commit}, not {start_commit}",
                        repo.member_id
                    ),
                ));
            }
            plans.push(CreatePlan {
                repo: repo.clone(),
                start_commit,
                existed: true,
            });
        } else {
            plans.push(CreatePlan {
                repo: repo.clone(),
                start_commit,
                existed: false,
            });
        }
    }
    Ok(plans)
}

fn delete_preflight<B: GitBackend>(
    backend: &B,
    repos: &[BranchRepo],
    branch: &str,
) -> ModelResult<()> {
    for repo in repos {
        let branches = backend.branch_list(&repo.path)?;
        let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "branch '{branch}' not found for member '{}'",
                    repo.member_id
                ),
            ));
        };
        if target.is_current {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "cannot delete current branch '{branch}' for member '{}'",
                    repo.member_id
                ),
            ));
        }
    }
    Ok(())
}

fn merge_preflight<B: GitBackend>(
    backend: &B,
    repos: &[BranchRepo],
    source_ref: &str,
) -> ModelResult<Vec<MergePlan>> {
    let mut plans = Vec::with_capacity(repos.len());
    for repo in repos {
        let status = backend.status(&repo.path)?;
        if status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                format!("member '{}' has uncommitted changes", repo.member_id),
            ));
        }
        reject_in_progress_integration(repo)?;
        let head = backend.head(&repo.path)?;
        if head.is_detached {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!("member '{}' HEAD is detached", repo.member_id),
            ));
        }
        let current_branch = head.branch.clone().ok_or_else(|| {
            ModelError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "member '{}' HEAD is not attached to a branch",
                    repo.member_id
                ),
            )
        })?;
        let head_commit = head.commit.clone().ok_or_else(|| {
            ModelError::new(
                ErrorCode::InvalidRequest,
                format!("member '{}' HEAD is unborn", repo.member_id),
            )
        })?;
        let source_commit = backend.read_ref(&repo.path, source_ref)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "source ref '{source_ref}' not found for member '{}'",
                    repo.member_id
                ),
            )
        })?;
        plans.push(MergePlan {
            repo: repo.clone(),
            current_branch,
            head_commit,
            source_ref: source_ref.to_owned(),
            source_commit,
        });
    }
    Ok(plans)
}

fn reject_in_progress_integration(repo: &BranchRepo) -> ModelResult<()> {
    let git_dir = repo.path.join(".git");
    if git_dir.join("MERGE_HEAD").exists()
        || git_dir.join("rebase-merge").exists()
        || git_dir.join("rebase-apply").exists()
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "member '{}' has an in-progress merge or rebase",
                repo.member_id
            ),
        ));
    }
    Ok(())
}

fn rollback_created_branches<B: GitBackend>(backend: &B, repos: &[PathBuf], branch: &str) {
    for repo in repos.iter().rev() {
        let _ = backend.branch_delete(repo, branch);
    }
}

fn require_branch_name(request: &crate::BranchRequest) -> ModelResult<String> {
    request
        .name
        .clone()
        .ok_or_else(|| invalid("a branch name is required"))
}

fn manifest_member<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))
}

fn summary_from_branch(
    repo: &BranchRepo,
    result: crate::BranchActionResult,
    branch: &GitBranch,
    current_branch: Option<&str>,
    detached: bool,
    unborn: bool,
) -> crate::BranchRepoSummary {
    crate::BranchRepoSummary {
        member_id: repo.member_id.clone(),
        member_path: repo.member_path.clone(),
        source_kind: crate::SourceKind::Git,
        result,
        branch: Some(branch.name.clone()),
        current_branch: current_branch.map(str::to_owned),
        detached,
        unborn,
        head: Some(branch.commit.clone()),
        upstream: None,
        ahead: None,
        behind: None,
        source_ref: None,
        target_branch: None,
        resulting_commit: None,
        conflict_paths: Vec::new(),
    }
}

fn summary_from_head(
    repo: &BranchRepo,
    result: crate::BranchActionResult,
    branch: Option<String>,
    head: &crate::git::GitHeadState,
) -> crate::BranchRepoSummary {
    crate::BranchRepoSummary {
        member_id: repo.member_id.clone(),
        member_path: repo.member_path.clone(),
        source_kind: crate::SourceKind::Git,
        result,
        branch,
        current_branch: head.branch.clone(),
        detached: head.is_detached,
        unborn: head.commit.is_none(),
        head: head.commit.clone(),
        upstream: None,
        ahead: None,
        behind: None,
        source_ref: None,
        target_branch: None,
        resulting_commit: None,
        conflict_paths: Vec::new(),
    }
}

fn dry_run_summary(
    repo: &BranchRepo,
    branch: &str,
    head: String,
    result: crate::BranchActionResult,
) -> crate::BranchRepoSummary {
    crate::BranchRepoSummary {
        member_id: repo.member_id.clone(),
        member_path: repo.member_path.clone(),
        source_kind: crate::SourceKind::Git,
        result,
        branch: Some(branch.to_owned()),
        current_branch: None,
        detached: false,
        unborn: false,
        head: if head.is_empty() { None } else { Some(head) },
        upstream: None,
        ahead: None,
        behind: None,
        source_ref: None,
        target_branch: None,
        resulting_commit: None,
        conflict_paths: Vec::new(),
    }
}

fn merge_planned_summary(
    plan: &MergePlan,
    result: crate::BranchActionResult,
) -> crate::BranchRepoSummary {
    crate::BranchRepoSummary {
        member_id: plan.repo.member_id.clone(),
        member_path: plan.repo.member_path.clone(),
        source_kind: crate::SourceKind::Git,
        result,
        branch: Some(plan.current_branch.clone()),
        current_branch: Some(plan.current_branch.clone()),
        detached: false,
        unborn: false,
        head: Some(plan.head_commit.clone()),
        upstream: None,
        ahead: None,
        behind: None,
        source_ref: Some(plan.source_ref.clone()),
        target_branch: Some(plan.current_branch.clone()),
        resulting_commit: Some(plan.source_commit.clone()),
        conflict_paths: Vec::new(),
    }
}

fn merge_result_summary(
    plan: &MergePlan,
    result: crate::BranchActionResult,
    head: Option<String>,
    resulting_commit: Option<String>,
    conflict_paths: Vec<String>,
) -> crate::BranchRepoSummary {
    crate::BranchRepoSummary {
        member_id: plan.repo.member_id.clone(),
        member_path: plan.repo.member_path.clone(),
        source_kind: crate::SourceKind::Git,
        result,
        branch: Some(plan.current_branch.clone()),
        current_branch: Some(plan.current_branch.clone()),
        detached: false,
        unborn: false,
        head,
        upstream: None,
        ahead: None,
        behind: None,
        source_ref: Some(plan.source_ref.clone()),
        target_branch: Some(plan.current_branch.clone()),
        resulting_commit,
        conflict_paths,
    }
}

fn partial_response(
    context: OperationContext,
    summaries: Vec<crate::BranchRepoSummary>,
    error: &ModelError,
) -> crate::BranchResponse {
    branch_response(
        context,
        crate::AggregateStatus::Partial,
        summaries,
        vec![error.into()],
    )
}

fn branch_response(
    context: OperationContext,
    aggregate_status: crate::AggregateStatus,
    repos: Vec<crate::BranchRepoSummary>,
    errors: Vec<crate::GwzError>,
) -> crate::BranchResponse {
    let mut response = response_envelope(context, aggregate_status, Vec::new());
    response.errors = errors;
    crate::BranchResponse {
        response,
        repos: Some(repos),
    }
}
