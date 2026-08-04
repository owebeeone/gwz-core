use super::{MergeBaseline, MergeExecutionMode, MergeTargetKind};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergePlan {
    pub source_ref: String,
    pub mode: MergeExecutionMode,
    pub baseline: MergeBaseline,
    pub participants: Vec<MergeParticipantPlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergeParticipantPlan {
    pub target_id: String,
    pub target_kind: MergeTargetKind,
    pub path: String,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub analysis: Option<crate::MergeAnalysisKind>,
    pub prediction_complete: bool,
    pub predicted_conflict_paths: Vec<String>,
    pub commit_message: String,
}
