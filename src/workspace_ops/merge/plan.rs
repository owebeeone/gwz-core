use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::{self, ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember};
use crate::git::{
    GitBackend, GitHeadState, GitMergeAnalysis, GitMergeAnalysisKind, GitNativeMergeState,
    GitStatus,
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
    build_merge_plan(
        &BackendPlanningView(backend),
        root,
        request,
        &manifest,
        &lock,
        MergeBaseline {
            lock_sha256: file_sha256(&root.join(artifact::LOCK_PATH))?,
            manifest_sha256: file_sha256(&root.join(WORKSPACE_MANIFEST))?,
            root_head: None,
            extensions: Default::default(),
        },
    )
}

trait PlanningBackend {
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
    fn read_ref(&self, path: &Path, name: &str) -> ModelResult<Option<String>>;
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
    fn read_ref(&self, path: &Path, name: &str) -> ModelResult<Option<String>> {
        self.0.read_ref(path, name)
    }
}

fn build_merge_plan<P: PlanningBackend>(
    backend: &P,
    root: &Path,
    request: &crate::MergeRequest,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    baseline: MergeBaseline,
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
    if explicitly_selected_root
        && targets
            .iter()
            .any(|target| matches!(target, SelectedTarget::Root))
    {
        return Err(ModelError::new(
            ErrorCode::RootMergeNotYetSupported,
            "explicit @root merge participation is not yet available",
        ));
    }
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
    let participants = manifest
        .members
        .iter()
        .filter(|member| selected.contains(member.id.as_str()))
        .map(|member| preflight_member(backend, root, lock, member, source))
        .collect::<ModelResult<Vec<_>>>()?;
    Ok(MergePlan {
        source_ref: source.to_owned(),
        baseline,
        participants,
    })
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

fn member_error(code: ErrorCode, member: &ManifestMember, detail: &str) -> ModelError {
    ModelError::new(code, detail).with_member(&member.id, &member.path)
}

fn member_backend_error(error: ModelError, member: &ManifestMember) -> ModelError {
    error.with_member(&member.id, &member.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_ops::tests::TempDir;

    #[derive(Default)]
    struct FakeBackend {
        calls: std::cell::RefCell<Vec<String>>,
        dirty: Option<&'static str>,
        drift: Option<&'static str>,
        failure: Option<(FailurePoint, &'static str)>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FailurePoint {
        IsRepository,
        Status,
        Head,
        MergeState,
        MergeAnalysis,
        ReadRef,
    }

    impl FakeBackend {
        fn fail(&self, point: FailurePoint, path: &Path) -> ModelResult<()> {
            if self.failure == Some((point, key(path))) {
                let (code, message) = match point {
                    FailurePoint::MergeState => (
                        ErrorCode::InvalidRequest,
                        "repository has an integration operation in progress: RebaseMerge",
                    ),
                    FailurePoint::MergeAnalysis => {
                        (ErrorCode::GitCommandFailed, "revspec 'feature/x' not found")
                    }
                    _ => (ErrorCode::IoError, "backend probe failed"),
                };
                return Err(ModelError::new(code, message));
            }
            Ok(())
        }
    }

    impl PlanningBackend for FakeBackend {
        fn is_repository(&self, path: &Path) -> ModelResult<bool> {
            self.fail(FailurePoint::IsRepository, path)?;
            Ok(true)
        }
        fn status(&self, path: &Path) -> ModelResult<GitStatus> {
            self.fail(FailurePoint::Status, path)?;
            Ok(GitStatus {
                untracked: usize::from(self.dirty == Some(key(path))),
                ..GitStatus::clean()
            })
        }
        fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
            self.fail(FailurePoint::Head, path)?;
            Ok(GitHeadState {
                branch: Some("main".into()),
                commit: Some(format!("before-{}", key(path))),
                is_detached: false,
            })
        }
        fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
            self.fail(FailurePoint::MergeState, path)?;
            if self.dirty == Some("integration") {
                return Err(ModelError::new(
                    ErrorCode::InvalidRequest,
                    "rebase in progress",
                ));
            }
            Ok(None)
        }
        fn merge_analysis(&self, path: &Path, _: &str, _: &str) -> ModelResult<GitMergeAnalysis> {
            self.fail(FailurePoint::MergeAnalysis, path)?;
            self.calls.borrow_mut().push(key(path).to_owned());
            Ok(GitMergeAnalysis {
                target_branch: "main".into(),
                target_commit: format!("before-{}", key(path)),
                source_commit: format!("source-{}", key(path)),
                kind: GitMergeAnalysisKind::TrueMerge,
                commit_identity_required: true,
                prediction_complete: false,
            })
        }
        fn read_ref(&self, path: &Path, _: &str) -> ModelResult<Option<String>> {
            self.fail(FailurePoint::ReadRef, path)?;
            Ok(Some(if self.drift == Some(key(path)) {
                "moved".into()
            } else {
                format!("before-{}", key(path))
            }))
        }
    }

    fn key(path: &Path) -> &str {
        path.file_name().unwrap().to_str().unwrap()
    }

    fn fixture() -> (TempDir, ManifestArtifact, LockArtifact) {
        let root = TempDir::new("merge-plan");
        for path in ["z", "a"] {
            fs::create_dir(root.path().join(path)).unwrap();
        }
        let manifest = ManifestArtifact::from_yaml("schema: gwz.workspace/v0\nworkspace:\n  id: ws_test\nmembers:\n- id: mem_z\n  path: z\n  type: git\n  source_id: src_z\n  active: true\n  remotes: []\n- id: mem_old\n  path: old\n  type: git\n  source_id: src_old\n  active: false\n  remotes: []\n- id: mem_a\n  path: a\n  type: git\n  source_id: src_a\n  active: true\n  remotes: []\n").unwrap();
        let lock = LockArtifact::from_yaml("schema: gwz.lock/v0\nworkspace_id: ws_test\nmanifest_schema: gwz.workspace/v0\nmembers:\n  mem_z:\n    path: z\n    source_id: src_z\n    source_kind: git\n    commit: before-z\n    branch: main\n    materialized: true\n  mem_a:\n    path: a\n    source_id: src_a\n    source_kind: git\n    commit: before-a\n    branch: main\n    materialized: true\n").unwrap();
        (root, manifest, lock)
    }

    fn request(selection: Option<crate::Selection>, dry_run: bool) -> crate::MergeRequest {
        crate::MergeRequest {
            meta: crate::RequestMeta {
                request_id: "req".into(),
                schema_version: "gwz.v0".into(),
                selection,
                dry_run: dry_run.then_some(true),
                ..Default::default()
            },
            op: crate::MergeOp::Start,
            source_ref: Some("feature/x".into()),
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
        }
    }

    fn build(
        backend: &FakeBackend,
        fixture: &(TempDir, ManifestArtifact, LockArtifact),
        request: &crate::MergeRequest,
    ) -> ModelResult<MergePlan> {
        build_merge_plan(
            backend,
            fixture.0.path(),
            request,
            &fixture.1,
            &fixture.2,
            MergeBaseline {
                lock_sha256: "lock".into(),
                manifest_sha256: "manifest".into(),
                root_head: None,
                extensions: Default::default(),
            },
        )
    }

    fn ids(plan: &MergePlan) -> Vec<&str> {
        plan.participants
            .iter()
            .map(|participant| participant.target_id.as_str())
            .collect()
    }

    #[test]
    fn selection_freezes_active_members_in_manifest_order() {
        let fixture = fixture();
        let backend = FakeBackend::default();
        let plan = build(&backend, &fixture, &request(None, false)).unwrap();
        assert_eq!(ids(&plan), ["mem_z", "mem_a"]);
        assert_eq!(plan.participants[0].before_commit, "before-z");
        assert_eq!(plan.participants[0].source_commit, "source-z");
        let reversed = crate::Selection {
            targets: vec!["mem_a".into(), "mem_z".into()],
            ..Default::default()
        };
        assert_eq!(
            ids(&build(&backend, &fixture, &request(Some(reversed), false)).unwrap()),
            ["mem_z", "mem_a"]
        );
    }

    #[test]
    fn only_explicit_root_returns_the_m2c_phase_error() {
        let root = crate::Selection {
            targets: vec!["@root".into()],
            ..Default::default()
        };
        assert_eq!(
            build(&Default::default(), &fixture(), &request(Some(root), false))
                .unwrap_err()
                .code,
            ErrorCode::RootMergeNotYetSupported
        );
        for selection in [
            crate::Selection {
                all: Some(true),
                ..Default::default()
            },
            crate::Selection {
                targets: vec!["@all".into()],
                ..Default::default()
            },
            crate::Selection {
                all: Some(true),
                targets: vec!["@root".into()],
                exclude_targets: vec!["@root".into()],
                ..Default::default()
            },
        ] {
            assert_eq!(
                ids(&build(
                    &Default::default(),
                    &fixture(),
                    &request(Some(selection), false)
                )
                .unwrap()),
                ["mem_z", "mem_a"]
            );
        }
    }

    #[test]
    fn dry_run_is_advisory_and_full_preflight_precedes_any_execution() {
        let fixture = fixture();
        let mut backend = FakeBackend::default();
        let normal = build(&backend, &fixture, &request(None, false)).unwrap();
        backend.calls.borrow_mut().clear();
        assert_eq!(
            build(&backend, &fixture, &request(None, true)).unwrap(),
            normal
        );
        backend.calls.borrow_mut().clear();
        backend.dirty = Some("a");
        assert_eq!(
            build(&backend, &fixture, &request(None, false))
                .unwrap_err()
                .code,
            ErrorCode::DirtyMember
        );
        assert_eq!(*backend.calls.borrow(), ["z"]);
    }

    #[test]
    fn preflight_propagates_integration_errors_and_rejects_target_drift() {
        let integration = FakeBackend {
            dirty: Some("integration"),
            ..Default::default()
        };
        assert_eq!(
            build(&integration, &fixture(), &request(None, false))
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let backend = FakeBackend {
            drift: Some("a"),
            ..Default::default()
        };
        assert_eq!(
            build(&backend, &fixture(), &request(None, false))
                .unwrap_err()
                .code,
            ErrorCode::MergeDrift
        );
    }

    #[test]
    fn second_member_missing_source_has_member_context_and_preserves_backend_code() {
        let backend = FakeBackend {
            failure: Some((FailurePoint::MergeAnalysis, "a")),
            ..Default::default()
        };

        let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

        assert_eq!(error.code, ErrorCode::GitCommandFailed);
        assert_eq!(error.member_id.as_deref(), Some("mem_a"));
        assert_eq!(error.member_path.as_deref(), Some("a"));
        assert!(error.message.starts_with("member 'mem_a' at 'a':"));
        let wire = crate::GwzError::from(&error);
        assert_eq!(wire.member_id.as_deref(), Some("mem_a"));
        assert_eq!(wire.member_path.as_deref(), Some("a"));
        assert_eq!(wire.target_kind, Some(crate::TargetKind::Member));
    }

    #[test]
    fn second_member_foreign_integration_state_has_member_context() {
        let backend = FakeBackend {
            failure: Some((FailurePoint::MergeState, "a")),
            ..Default::default()
        };

        let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(error.member_id.as_deref(), Some("mem_a"));
        assert_eq!(error.member_path.as_deref(), Some("a"));
        assert!(error.message.contains("RebaseMerge"));
        assert!(error.message.starts_with("member 'mem_a' at 'a':"));
    }

    #[test]
    fn every_fallible_backend_preflight_probe_adds_member_context() {
        for point in [
            FailurePoint::IsRepository,
            FailurePoint::Status,
            FailurePoint::Head,
            FailurePoint::MergeState,
            FailurePoint::MergeAnalysis,
            FailurePoint::ReadRef,
        ] {
            let backend = FakeBackend {
                failure: Some((point, "a")),
                ..Default::default()
            };

            let error = build(&backend, &fixture(), &request(None, false)).unwrap_err();

            assert_eq!(error.member_id.as_deref(), Some("mem_a"), "{point:?}");
            assert_eq!(error.member_path.as_deref(), Some("a"), "{point:?}");
            assert!(
                error.message.starts_with("member 'mem_a' at 'a':"),
                "{point:?}: {error}"
            );
        }
    }
}
