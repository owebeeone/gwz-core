use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember};
use crate::git::{
    GitBackend, GitHeadState, GitMergeAnalysis, GitMergeAnalysisKind, GitMergeSimulation,
    GitNativeMergeState, GitStatus,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::{
    CommandDefaultTargets, RootSelectionPolicy, SelectedTarget, assert_workspace_id,
    resolve_targets,
};

use super::{MergeBaseline, MergeParticipantPlan, MergePlan, MergeTargetKind};

pub(crate) fn plan_merge<B: GitBackend>(
    backend: &B,
    root: &Path,
    request: &crate::MergeRequest,
) -> ModelResult<MergePlan> {
    let manifest = artifact::read_manifest(root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(root)?;
    if lock.workspace_id != manifest.workspace.id {
        return Err(ModelError::new(
            ErrorCode::SourceIdentityMismatch,
            "workspace manifest and lock identify different workspaces",
        ));
    }
    let root_head = backend.head(root)?;
    let plan = build_merge_plan(
        &BackendPlanningView(backend),
        root,
        request,
        &manifest,
        &lock,
        MergeBaseline {
            lock_sha256: file_sha256(&root.join(artifact::LOCK_PATH))?,
            manifest_sha256: file_sha256(&root.join(WORKSPACE_MANIFEST))?,
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: root_head.commit.clone(),
            root_branch: root_head.branch.clone(),
            extensions: Default::default(),
        },
    )?;
    if plan
        .participants
        .iter()
        .any(|participant| participant.analysis != Some(crate::MergeAnalysisKind::UpToDate))
        && (root_head.is_detached || root_head.branch.is_none())
    {
        return Err(ModelError::new(
            ErrorCode::BranchDetachedHead,
            "workspace root must be on an attached branch to publish merge evidence",
        )
        .with_member("@root", "."));
    }
    Ok(plan)
}

pub(super) trait PlanningBackend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool>;
    fn status(&self, path: &Path) -> ModelResult<GitStatus>;
    fn head(&self, path: &Path) -> ModelResult<GitHeadState>;
    fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>>;
    fn merge_analysis(
        &self,
        path: &Path,
        branch: &str,
        source: &str,
    ) -> ModelResult<GitMergeAnalysis>;
    fn merge_simulate(
        &self,
        path: &Path,
        target_commit: &str,
        source_commit: &str,
    ) -> ModelResult<GitMergeSimulation>;
    fn read_ref(&self, path: &Path, name: &str) -> ModelResult<Option<String>>;
    fn read_file_at_commit(
        &self,
        path: &Path,
        commit: &str,
        relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>>;
}

struct BackendPlanningView<'a, B>(&'a B);

impl<B: GitBackend> PlanningBackend for BackendPlanningView<'_, B> {
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        self.0.is_repository(path)
    }
    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        self.0.status(path)
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        self.0.head(path)
    }
    fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
        self.0.merge_state(path)
    }
    fn merge_analysis(
        &self,
        path: &Path,
        branch: &str,
        source: &str,
    ) -> ModelResult<GitMergeAnalysis> {
        self.0.merge_analysis(path, branch, source)
    }
    fn merge_simulate(
        &self,
        path: &Path,
        target_commit: &str,
        source_commit: &str,
    ) -> ModelResult<GitMergeSimulation> {
        self.0.merge_simulate(path, target_commit, source_commit)
    }
    fn read_ref(&self, path: &Path, name: &str) -> ModelResult<Option<String>> {
        self.0.read_ref(path, name)
    }
    fn read_file_at_commit(
        &self,
        path: &Path,
        commit: &str,
        relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        self.0.read_file_at_commit(path, commit, relative_path)
    }
}

fn build_merge_plan<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    request: &crate::MergeRequest,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    mut baseline: MergeBaseline,
) -> ModelResult<MergePlan> {
    let targets = resolve_targets(
        manifest,
        request.meta.selection.as_ref(),
        CommandDefaultTargets::Members,
        RootSelectionPolicy::Allow,
    )?;
    let explicitly_selected_root = request.meta.selection.as_ref().is_some_and(|selection| {
        selection
            .member_ids
            .iter()
            .chain(&selection.paths)
            .chain(&selection.targets)
            .any(|target| target == "@root")
    });
    let root_selected = explicitly_selected_root
        && targets
            .iter()
            .any(|target| matches!(target, SelectedTarget::Root));
    let selected: BTreeSet<&str> = targets
        .iter()
        .filter_map(|target| match target {
            SelectedTarget::Member(member) => Some(member.id.as_str()),
            SelectedTarget::Root => None,
        })
        .collect();
    let source = request.source_ref.as_deref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeValidationFailed,
            "source_ref is required for merge start",
        )
    })?;
    let mut participants = manifest
        .members
        .iter()
        .filter(|member| selected.contains(member.id.as_str()))
        .map(|member| preflight_member(backend, root, lock, member, source))
        .collect::<ModelResult<Vec<_>>>()?;
    if root_selected {
        baseline.lock_commit_sha256 = committed_file_sha256(
            backend,
            root,
            baseline.root_head.as_deref(),
            artifact::LOCK_PATH,
        )?;
        baseline.manifest_commit_sha256 = committed_file_sha256(
            backend,
            root,
            baseline.root_head.as_deref(),
            WORKSPACE_MANIFEST,
        )?;
        let root_plan = super::root::preflight_root(backend, root, source, &baseline)?;
        if baseline.root_head.as_deref() != Some(root_plan.before_commit.as_str())
            || baseline.root_branch.as_deref() != Some(root_plan.target_branch.as_str())
        {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "workspace root changed while its merge plan was being frozen",
            )
            .with_member("@root", "."));
        }
        participants.push(root_plan);
    }
    let mode = request.mode.into();
    enforce_mode(mode, &participants)?;
    if request.meta.dry_run == Some(true) {
        complete_predictions(backend, root, &mut participants)?;
    }
    Ok(MergePlan {
        source_ref: source.to_owned(),
        mode,
        baseline,
        participants,
    })
}

fn enforce_mode(
    mode: super::MergeExecutionMode,
    participants: &[MergeParticipantPlan],
) -> ModelResult<()> {
    if mode != super::MergeExecutionMode::FfOnly {
        return Ok(());
    }
    if let Some(participant) = participants
        .iter()
        .find(|participant| participant.analysis == Some(crate::MergeAnalysisKind::TrueMerge))
    {
        return Err(ModelError::new(
            ErrorCode::MergeValidationFailed,
            "merge requires a merge commit but --ff-only was requested",
        )
        .with_member(&participant.target_id, &participant.path));
    }
    Ok(())
}

fn complete_predictions<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    participants: &mut [MergeParticipantPlan],
) -> ModelResult<()> {
    for participant in participants
        .iter_mut()
        .filter(|participant| participant.analysis == Some(crate::MergeAnalysisKind::TrueMerge))
    {
        let path = if participant.target_kind == MergeTargetKind::Root {
            root.to_path_buf()
        } else {
            root.join(&participant.path)
        };
        match backend.merge_simulate(
            &path,
            &participant.before_commit,
            &participant.source_commit,
        ) {
            Ok(GitMergeSimulation::Clean) => {
                participant.prediction_complete = true;
                participant.predicted_conflict_paths.clear();
            }
            Ok(GitMergeSimulation::Conflicts(paths)) => {
                participant.prediction_complete = true;
                participant.predicted_conflict_paths = paths;
            }
            Err(error) if error.code == ErrorCode::UnsupportedOperation => {}
            Err(error) => {
                return Err(if participant.target_kind == MergeTargetKind::Root {
                    error.with_member("@root", ".")
                } else {
                    error.with_member(&participant.target_id, &participant.path)
                });
            }
        }
    }
    Ok(())
}

fn preflight_member<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    lock: &LockArtifact,
    member: &ManifestMember,
    source: &str,
) -> ModelResult<MergeParticipantPlan> {
    if member.source_kind != ArtifactSourceKind::Git {
        return Err(member_error(
            ErrorCode::UnsupportedSourceKind,
            member,
            "is not a Git member",
        ));
    }
    let locked = lock
        .members
        .get(&member.id)
        .ok_or_else(|| member_error(ErrorCode::LockNotFound, member, "has no lock record"))?;
    if locked.path != member.path || locked.materialized != Some(true) {
        return Err(member_error(
            ErrorCode::MemberNotFound,
            member,
            "is not materialized at its manifest path",
        ));
    }
    let path = root.join(&member.path);
    if !path.is_dir()
        || !backend
            .is_repository(&path)
            .map_err(|error| member_backend_error(error, member))?
    {
        return Err(member_error(
            ErrorCode::MemberNotFound,
            member,
            "is not a materialized Git repository",
        ));
    }
    let status = backend
        .status(&path)
        .map_err(|error| member_backend_error(error, member))?;
    if status.is_dirty
        || status.staged > 0
        || status.unstaged > 0
        || status.untracked > 0
        || status.unresolved > 0
    {
        return Err(member_error(
            ErrorCode::DirtyMember,
            member,
            "has index or worktree changes",
        ));
    }
    if backend
        .merge_state(&path)
        .map_err(|error| member_backend_error(error, member))?
        .is_some()
    {
        return Err(member_error(
            ErrorCode::MergeValidationFailed,
            member,
            "has a merge in progress",
        ));
    }
    let head = backend
        .head(&path)
        .map_err(|error| member_backend_error(error, member))?;
    if head.is_detached || head.branch.is_none() {
        return Err(member_error(
            ErrorCode::BranchDetachedHead,
            member,
            "HEAD is detached",
        ));
    }
    let branch = head.branch.expect("checked above");
    let before = head
        .commit
        .ok_or_else(|| member_error(ErrorCode::BranchUnbornHead, member, "HEAD is unborn"))?;
    let analysis = backend
        .merge_analysis(&path, &branch, source)
        .map_err(|error| member_backend_error(error, member))?;
    if analysis.target_branch != branch
        || analysis.target_commit != before
        || backend
            .read_ref(&path, &format!("refs/heads/{branch}"))
            .map_err(|error| member_backend_error(error, member))?
            .as_deref()
            != Some(before.as_str())
    {
        return Err(member_error(
            ErrorCode::MergeDrift,
            member,
            "target branch changed during merge preflight",
        ));
    }
    Ok(MergeParticipantPlan {
        target_id: member.id.clone(),
        target_kind: MergeTargetKind::Member,
        path: member.path.clone(),
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

fn file_sha256(path: &Path) -> ModelResult<String> {
    let bytes = fs::read(path).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("failed to hash '{}': {error}", path.display()),
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn committed_file_sha256<B: PlanningBackend>(
    backend: &B,
    root: &Path,
    commit: Option<&str>,
    relative_path: &str,
) -> ModelResult<Option<String>> {
    let Some(commit) = commit else {
        return Ok(None);
    };
    Ok(backend
        .read_file_at_commit(root, commit, relative_path)?
        .map(|bytes| format!("{:x}", Sha256::digest(bytes))))
}

fn member_error(code: ErrorCode, member: &ManifestMember, detail: &str) -> ModelError {
    ModelError::new(code, detail).with_member(&member.id, &member.path)
}

fn member_backend_error(error: ModelError, member: &ManifestMember) -> ModelError {
    error.with_member(&member.id, &member.path)
}

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;
