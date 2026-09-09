use std::path::Path;

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestMember};
use crate::git::{GitBackend, GitHeadState, GitStatus as BackendGitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, OperationRequest};

use super::*;

pub fn handle_status<B>(
    backend: &B,
    start: &Path,
    request: crate::StatusRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::StatusResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Status(request.clone()).context(operation_id.into())?;
    let workspace_root = crate::workspace_ops::resolve_request_workspace_root(start, &request.meta)?;
    let manifest = artifact::read_manifest(&workspace_root)?;
    if let Some(expected) = request
        .meta
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_id.as_ref())
        && expected != &manifest.workspace.id
    {
        return Err(ModelError::new(
            ErrorCode::WorkspaceNotFound,
            "workspace id does not match manifest",
        ));
    }

    let lock = read_lock_optional(&workspace_root)?;
    let selected = crate::workspace_ops::resolve_action_targets(
        &manifest,
        request.meta.selection.as_ref(),
        crate::ActionKind::Status,
    )?;
    let include_root = selected
        .iter()
        .any(|target| matches!(target, crate::workspace_ops::SelectedTarget::Root));
    let mut reports = Vec::with_capacity(selected.len());
    for member in selected.into_iter().filter_map(|target| match target {
        crate::workspace_ops::SelectedTarget::Root => None,
        crate::workspace_ops::SelectedTarget::Member(member) => Some(member),
    }) {
        reports.push(status_member(
            backend,
            &workspace_root,
            member,
            lock.as_ref(),
        ));
    }
    let members = reports
        .iter()
        .map(|report| report.response.clone())
        .collect::<Vec<_>>();
    let root_report = if include_root {
        root_status(backend, &workspace_root)?
    } else {
        None
    };
    let workspace_git_status = matches!(
        request.mode,
        Some(crate::StatusMode::Combined | crate::StatusMode::Summary)
    )
    .then(|| {
        workspace_git_status(
            root_report.as_ref(),
            &reports,
            request.include_file_changes.unwrap_or(true),
            request.include_branch_summary.unwrap_or(true),
            request
                .path_style
                .unwrap_or(crate::StatusPathStyle::WorkspaceRelative),
        )
    });

    Ok(crate::StatusResponse {
        response: crate::ResponseEnvelope {
            meta: crate::ResponseMeta {
                transport: None,
                request_id: context.request_id,
                schema_version: context.schema_version,
                action: ActionKind::Status.into(),
                aggregate_status: aggregate_status(&members),
                operation_id: Some(context.operation_id),
                message: None,
                attribution: context.attribution.as_ref().map(Into::into),
            },
            members,
            errors: Vec::new(),
        },
        workspace_git_status,
    })
}

pub(crate) fn read_lock_optional(root: &Path) -> ModelResult<Option<LockArtifact>> {
    let path = root.join(artifact::LOCK_PATH);
    if path.exists() {
        artifact::read_lock(root).map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StatusMemberReport {
    pub(crate) response: crate::MemberResponse,
    pub(crate) head: Option<GitHeadState>,
    pub(crate) status: Option<BackendGitStatus>,
}

#[derive(Clone, Debug)]
pub(crate) struct RootStatusReport {
    pub(crate) head: GitHeadState,
    pub(crate) status: BackendGitStatus,
}

pub(crate) fn root_status<B>(
    backend: &B,
    workspace_root: &Path,
) -> ModelResult<Option<RootStatusReport>>
where
    B: GitBackend,
{
    if !backend.is_repository(workspace_root)? {
        return Ok(None);
    }

    Ok(Some(RootStatusReport {
        head: backend.head(workspace_root)?,
        status: backend.status(workspace_root)?,
    }))
}

pub(crate) fn status_member<B>(
    backend: &B,
    workspace_root: &Path,
    member: &ManifestMember,
    lock: Option<&LockArtifact>,
) -> StatusMemberReport
where
    B: GitBackend,
{
    let source_kind = protocol_source_kind(member.source_kind);
    if member.source_kind != ArtifactSourceKind::Git {
        return StatusMemberReport {
            response: member_error(
                member,
                source_kind,
                ModelError::new(
                    ErrorCode::UnsupportedSourceKind,
                    "status supports git members only",
                ),
                crate::MemberStatus::Rejected,
            ),
            head: None,
            status: None,
        };
    }

    let member_root = workspace_root.join(&member.path);
    match backend.is_repository(&member_root) {
        // The member is declared in gwz.conf but its working tree was never
        // cloned (e.g. right after a bare `git clone` of the workspace root).
        // That is an expected, recoverable state, not a git failure.
        Ok(false) => {
            return StatusMemberReport {
                response: member_not_materialized(member, source_kind, lock),
                head: None,
                status: None,
            };
        }
        Err(error) => {
            return StatusMemberReport {
                response: member_error(member, source_kind, error, crate::MemberStatus::Failed),
                head: None,
                status: None,
            };
        }
        Ok(true) => {}
    }
    let head = match backend.head(&member_root) {
        Ok(head) => head,
        Err(error) => {
            return StatusMemberReport {
                response: member_error(member, source_kind, error, crate::MemberStatus::Failed),
                head: None,
                status: None,
            };
        }
    };
    let status = match backend.status(&member_root) {
        Ok(status) => status,
        Err(error) => {
            return StatusMemberReport {
                response: member_error(member, source_kind, error, crate::MemberStatus::Failed),
                head: None,
                status: None,
            };
        }
    };

    let response = crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status: crate::MemberStatus::Ok,
        error: None,
        planned: None,
        state: None,
        git_status: Some(protocol_git_status(member, &head, &status)),
        target_kind: Some(crate::TargetKind::Member),
        lock_match: Some(lock_match(lock, member, &head, &status)),
    };
    StatusMemberReport {
        response,
        head: Some(head),
        status: Some(status),
    }
}

pub(crate) fn workspace_git_status(
    root: Option<&RootStatusReport>,
    reports: &[StatusMemberReport],
    include_file_changes: bool,
    include_branch_summary: bool,
    path_style: crate::StatusPathStyle,
) -> crate::WorkspaceGitStatus {
    let root_clean = root.is_none_or(|report| !report.status.is_dirty);
    let members_clean = reports.iter().all(|report| {
        report.response.status == crate::MemberStatus::Ok
            && report.status.as_ref().is_none_or(|status| !status.is_dirty)
    });
    let clean = root_clean && members_clean;
    let root_file_changes = if include_file_changes {
        root.map(root_file_changes).unwrap_or_default()
    } else {
        Vec::new()
    };
    let file_changes = if include_file_changes {
        reports
            .iter()
            .flat_map(|report| report_file_changes(report, path_style))
            .collect()
    } else {
        Vec::new()
    };
    let branches = if include_branch_summary {
        reports
            .iter()
            .filter_map(report_branch_status)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let (branch_groups, branch_differences) = if include_branch_summary {
        branch_groups_and_differences(&branches)
    } else {
        (Vec::new(), Vec::new())
    };

    crate::WorkspaceGitStatus {
        clean,
        root_status: root.map(protocol_root_git_status),
        root_file_changes,
        file_changes,
        branches,
        branch_groups,
        branch_differences,
    }
}

pub(crate) fn root_file_changes(report: &RootStatusReport) -> Vec<crate::WorkspaceRootFileChange> {
    report
        .status
        .files
        .iter()
        .map(|file| crate::WorkspaceRootFileChange {
            repo_path: file.path.clone(),
            workspace_path: file.path.clone(),
            index_status: file.index_status.clone(),
            worktree_status: file.worktree_status.clone(),
            original_repo_path: file.original_path.clone(),
        })
        .collect()
}

pub(crate) fn report_file_changes(
    report: &StatusMemberReport,
    path_style: crate::StatusPathStyle,
) -> Vec<crate::GitFileChange> {
    let Some(status) = &report.status else {
        return Vec::new();
    };
    status
        .files
        .iter()
        .map(|file| {
            let workspace_path = match path_style {
                crate::StatusPathStyle::WorkspaceRelative => {
                    workspace_path(&report.response.member_path, &file.path)
                }
                crate::StatusPathStyle::MemberRelative => file.path.clone(),
            };
            crate::GitFileChange {
                member_id: report.response.member_id.clone(),
                member_path: report.response.member_path.clone(),
                repo_path: file.path.clone(),
                workspace_path,
                index_status: file.index_status.clone(),
                worktree_status: file.worktree_status.clone(),
                original_repo_path: file.original_path.clone(),
            }
        })
        .collect()
}

pub(crate) fn report_branch_status(
    report: &StatusMemberReport,
) -> Option<crate::GitMemberBranchStatus> {
    let head = report.head.as_ref()?;
    let label = branch_label(head);
    Some(crate::GitMemberBranchStatus {
        member_id: report.response.member_id.clone(),
        member_path: report.response.member_path.clone(),
        label,
        branch: head.branch.clone(),
        detached: head.is_detached,
        unborn: head.commit.is_none() && !head.is_detached,
        head: head.commit.clone(),
        upstream: None,
        ahead: None,
        behind: None,
    })
}

pub(crate) fn protocol_git_status(
    member: &ManifestMember,
    head: &GitHeadState,
    status: &BackendGitStatus,
) -> crate::GitStatus {
    crate::GitStatus {
        member_id: member.id.clone(),
        branch: head.branch.clone(),
        detached: head.is_detached,
        head: head.commit.clone(),
        upstream: None,
        ahead: None,
        behind: None,
        staged: status.staged as i64,
        unstaged: status.unstaged as i64,
        untracked: status.untracked as i64,
        dirty: status.is_dirty,
    }
}

pub(crate) fn protocol_root_git_status(report: &RootStatusReport) -> crate::WorkspaceRootGitStatus {
    crate::WorkspaceRootGitStatus {
        branch: report.head.branch.clone(),
        detached: report.head.is_detached,
        head: report.head.commit.clone(),
        staged: report.status.staged as i64,
        unstaged: report.status.unstaged as i64,
        untracked: report.status.untracked as i64,
        dirty: report.status.is_dirty,
        unborn: report.head.commit.is_none() && !report.head.is_detached,
    }
}

pub(crate) fn aggregate_status(members: &[crate::MemberResponse]) -> crate::AggregateStatus {
    if members
        .iter()
        .any(|member| member.status == crate::MemberStatus::Failed)
    {
        crate::AggregateStatus::Failed
    } else if members
        .iter()
        .any(|member| member.status == crate::MemberStatus::Rejected)
    {
        crate::AggregateStatus::Rejected
    } else if members.iter().any(|member| {
        member
            .git_status
            .as_ref()
            .is_some_and(|status| status.dirty)
    }) {
        // F5/AD3: a dirty member is observable state, not a failure — surface it in the
        // aggregate instead of masquerading as a clean `Ok` (exit code stays 0).
        crate::AggregateStatus::Dirty
    } else {
        crate::AggregateStatus::Ok
    }
}
