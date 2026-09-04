pub(super) mod artifact_facts;
mod finalization;
mod planning;
mod reconciliation;
mod v1_rollback;

use std::path::Path;

use crate::git::GitBackend;
use crate::model::ModelResult;

use super::MergeStatusRecordView;

pub(in crate::workspace_ops::merge) use v1_rollback::{
    V1RootRollbackObservation, execute_v1_root_metadata_rollback,
    observe_v1_root_metadata_rollback, observe_v1_selected_root_baseline,
    selected_root_result_artifacts,
};

pub(in crate::workspace_ops::merge) use finalization::{
    evidence_parent_view, interrupted_evidence_rollback_is_exact_view,
    root_finalization_is_exact_view,
};
pub(super) use planning::preflight_root;
pub(in crate::workspace_ops::merge) use reconciliation::frozen_manifest;

pub(in crate::workspace_ops) fn open_merge_stage_member_paths<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeStatusRecordView<'_>,
) -> ModelResult<Vec<String>> {
    Ok(frozen_manifest(backend, root, record)?
        .members
        .into_iter()
        .filter(|member| member.active)
        .map(|member| member.path)
        .collect())
}
