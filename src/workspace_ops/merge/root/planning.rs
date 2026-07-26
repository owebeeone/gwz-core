use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact;
use crate::git::{GitMergeAnalysisKind, GitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::plan::PlanningBackend;
use super::super::{MergeBaseline, MergeParticipantPlan, MergeTargetKind};

pub(in crate::workspace_ops::merge) fn preflight_root<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    source: &str,
    baseline: &MergeBaseline,
) -> ModelResult<MergeParticipantPlan> {
    if !root.is_dir() || !backend.is_repository(root).map_err(root_backend_error)? {
        return Err(root_error(
            ErrorCode::MemberNotFound,
            "is not an ordinary Git repository",
        ));
    }
    let head = backend.head(root).map_err(root_backend_error)?;
    if head.is_detached || head.branch.is_none() {
        return Err(root_error(
            ErrorCode::BranchDetachedHead,
            "HEAD is detached",
        ));
    }
    let branch = head.branch.expect("checked above");
    let before = head
        .commit
        .ok_or_else(|| root_error(ErrorCode::BranchUnbornHead, "HEAD is unborn"))?;
    let status = backend.status(root).map_err(root_backend_error)?;
    if dirty(&status) {
        return Err(root_error(
            ErrorCode::DirtyMember,
            "has index or worktree changes",
        ));
    }
    if backend
        .merge_state(root)
        .map_err(root_backend_error)?
        .is_some()
    {
        return Err(root_error(
            ErrorCode::MergeValidationFailed,
            "has a merge in progress",
        ));
    }
    verify_baseline_file(
        backend,
        root,
        &before,
        artifact::LOCK_PATH,
        baseline.lock_commit_sha256.as_deref(),
    )?;
    verify_baseline_file(
        backend,
        root,
        &before,
        WORKSPACE_MANIFEST,
        baseline.manifest_commit_sha256.as_deref(),
    )?;
    let analysis = backend
        .merge_analysis(root, &branch, source)
        .map_err(root_backend_error)?;
    if analysis.target_branch != branch
        || analysis.target_commit != before
        || backend
            .read_ref(root, &format!("refs/heads/{branch}"))
            .map_err(root_backend_error)?
            .as_deref()
            != Some(before.as_str())
    {
        return Err(root_error(
            ErrorCode::MergeDrift,
            "target branch changed during merge preflight",
        ));
    }
    Ok(MergeParticipantPlan {
        target_id: "@root".to_owned(),
        target_kind: MergeTargetKind::Root,
        path: ".".to_owned(),
        target_branch: branch.clone(),
        before_commit: before,
        source_commit: analysis.source_commit,
        analysis: Some(match analysis.kind {
            GitMergeAnalysisKind::UpToDate => crate::MergeAnalysisKind::UpToDate,
            GitMergeAnalysisKind::FastForward => crate::MergeAnalysisKind::FastForward,
            GitMergeAnalysisKind::TrueMerge => crate::MergeAnalysisKind::TrueMerge,
        }),
        prediction_complete: analysis.prediction_complete,
        predicted_conflict_paths: Vec::new(),
        commit_message: format!("Merge {source} into {branch}"),
    })
}

fn verify_baseline_file<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    before: &str,
    relative_path: &str,
    expected_sha256: Option<&str>,
) -> ModelResult<()> {
    let bytes = backend
        .read_file_at_commit(root, before, relative_path)
        .map_err(root_backend_error)?
        .ok_or_else(|| {
            root_error(
                ErrorCode::MergeValidationFailed,
                &format!(
                    "before commit does not contain required workspace artifact '{relative_path}'"
                ),
            )
        })?;
    if expected_sha256.is_some_and(|expected| format!("{:x}", Sha256::digest(&bytes)) != expected) {
        return Err(root_error(
            ErrorCode::MergeDrift,
            &format!(
                "workspace artifact '{relative_path}' is not committed at the root before commit"
            ),
        ));
    }
    Ok(())
}

fn dirty(status: &GitStatus) -> bool {
    status.is_dirty
        || status.staged > 0
        || status.unstaged > 0
        || status.untracked > 0
        || status.unresolved > 0
}

fn root_error(code: ErrorCode, detail: &str) -> ModelError {
    ModelError::new(code, detail).with_member("@root", ".")
}

fn root_backend_error(error: ModelError) -> ModelError {
    error.with_member("@root", ".")
}
