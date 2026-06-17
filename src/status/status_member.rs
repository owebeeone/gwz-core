use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember};
use crate::git::{GitBackend, GitHeadState, GitStatus as BackendGitStatus};
use crate::model::{ErrorCode, MemberId, ModelError, ModelResult};
use crate::operation::{ActionKind, OperationRequest};
use crate::workspace::{MemberPath, discover_workspace_root};


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
    let workspace_root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
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
    let selected = resolve_selection(&manifest, request.meta.selection.as_ref())?;
    let mut reports = Vec::with_capacity(selected.len());
    for member in selected {
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
    let root_report = root_status(backend, &workspace_root)?;
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

pub(crate) fn resolve_workspace_root(
    start: &Path,
    workspace: Option<&crate::WorkspaceRef>,
) -> ModelResult<PathBuf> {
    if let Some(root) = workspace.and_then(|workspace| workspace.root.as_ref()) {
        Ok(PathBuf::from(root))
    } else {
        discover_workspace_root(start)
    }
}

pub(crate) fn read_lock_optional(root: &Path) -> ModelResult<Option<LockArtifact>> {
    let path = root.join(artifact::LOCK_PATH);
    if path.exists() {
        artifact::read_lock(root).map(Some)
    } else {
        Ok(None)
    }
}

pub(crate) fn resolve_selection<'a>(
    manifest: &'a ManifestArtifact,
    selection: Option<&crate::Selection>,
) -> ModelResult<Vec<&'a ManifestMember>> {
    match selection {
        None => Ok(manifest
            .members
            .iter()
            .filter(|member| member.active)
            .collect()),
        Some(selection) => resolve_explicit_selection(manifest, selection),
    }
}

pub(crate) fn resolve_explicit_selection<'a>(
    manifest: &'a ManifestArtifact,
    selection: &crate::Selection,
) -> ModelResult<Vec<&'a ManifestMember>> {
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
            .collect());
    }
    if !has_filters {
        return Err(invalid(
            "selection must include all=true, member_ids, or paths",
        ));
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for member_id in &selection.member_ids {
        MemberId::parse_str(member_id)?;
        let member = find_member_by_id(manifest, member_id)?;
        push_selected(member, &mut seen, &mut selected)?;
    }
    for path in &selection.paths {
        MemberPath::parse(path)?;
        let member = find_member_by_path(manifest, path)?;
        push_selected(member, &mut seen, &mut selected)?;
    }
    Ok(selected)
}

pub(crate) fn find_member_by_id<'a>(
    manifest: &'a ManifestArtifact,
    member_id: &str,
) -> ModelResult<&'a ManifestMember> {
    let mut matches = manifest
        .members
        .iter()
        .filter(|member| member.id == member_id);
    let member = matches
        .next()
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member id not found"))?;
    if matches.next().is_some() {
        return Err(invalid("member id selection is ambiguous"));
    }
    require_active(member)?;
    Ok(member)
}

pub(crate) fn find_member_by_path<'a>(
    manifest: &'a ManifestArtifact,
    path: &str,
) -> ModelResult<&'a ManifestMember> {
    let mut matches = manifest.members.iter().filter(|member| member.path == path);
    let member = matches
        .next()
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member path not found"))?;
    if matches.next().is_some() {
        return Err(invalid("member path selection is ambiguous"));
    }
    require_active(member)?;
    Ok(member)
}

pub(crate) fn push_selected<'a>(
    member: &'a ManifestMember,
    seen: &mut BTreeSet<&'a str>,
    selected: &mut Vec<&'a ManifestMember>,
) -> ModelResult<()> {
    if !seen.insert(member.id.as_str()) {
        return Err(invalid("selection resolves the same member more than once"));
    }
    selected.push(member);
    Ok(())
}

pub(crate) fn require_active(member: &ManifestMember) -> ModelResult<()> {
    if member.active {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::MemberInactive,
            "selected member is inactive",
        ))
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

pub(crate) fn root_status<B>(backend: &B, workspace_root: &Path) -> ModelResult<Option<RootStatusReport>>
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

pub(crate) fn report_branch_status(report: &StatusMemberReport) -> Option<crate::GitMemberBranchStatus> {
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
    } else {
        crate::AggregateStatus::Ok
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}

