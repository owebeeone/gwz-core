use super::*;

struct FailBeforeRoot;

impl ExecutionBackend for FailBeforeRoot {
    fn inspect(&self, path: &Path, _: &str, source: &str) -> ModelResult<Inspection> {
        assert_ne!(path, Path::new("workspace"));
        Ok((
            GitStatus::clean(),
            GitHeadState {
                branch: Some("main".to_owned()),
                commit: Some("member-before".to_owned()),
                is_detached: false,
            },
            GitMergeAnalysis {
                target_branch: "main".to_owned(),
                target_commit: "member-before".to_owned(),
                source_commit: source.to_owned(),
                kind: GitMergeAnalysisKind::FastForward,
                commit_identity_required: false,
                prediction_complete: true,
            },
        ))
    }

    fn prepare_merge(
        &self,
        _: &Path,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        Ok(GitPreparedMerge::FastForward)
    }

    fn merge(
        &self,
        _: &Path,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "injected member failure",
        ))
    }
}

#[test]
fn unexpected_member_failure_leaves_the_root_unattempted() {
    let plans = vec![
        MergeParticipantPlan {
            target_id: "mem_app".to_owned(),
            target_kind: super::super::MergeTargetKind::Member,
            path: "app".to_owned(),
            target_branch: "main".to_owned(),
            before_commit: "member-before".to_owned(),
            source_commit: "member-source".to_owned(),
            analysis: Some(crate::MergeAnalysisKind::FastForward),
            prediction_complete: true,
            commit_message: "merge".to_owned(),
        },
        MergeParticipantPlan {
            target_id: "@root".to_owned(),
            target_kind: super::super::MergeTargetKind::Root,
            path: ".".to_owned(),
            target_branch: "main".to_owned(),
            before_commit: "root-before".to_owned(),
            source_commit: "root-source".to_owned(),
            analysis: Some(crate::MergeAnalysisKind::FastForward),
            prediction_complete: true,
            commit_message: "merge".to_owned(),
        },
    ];

    let run = execute_plan(&FailBeforeRoot, Path::new("workspace"), &plans, None);

    assert_eq!(run.rows[0].state, PState::Failed);
    assert_eq!(run.rows[1].state, PState::Unattempted);
    assert_eq!(run.rows[1].plan.target_id, "@root");
}
