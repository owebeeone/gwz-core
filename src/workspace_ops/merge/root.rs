mod abort;
#[cfg(test)]
pub(super) mod artifact_facts;
mod finalization;
mod planning;
mod reconciliation;

use std::path::Path;

use crate::git::GitBackend;
use crate::model::ModelResult;

use super::MergeOperationRecord;

#[cfg(test)]
pub(in crate::workspace_ops::merge) use abort::{
    V1RootRollbackObservation, execute_v1_root_metadata_rollback,
    observe_v1_root_metadata_rollback, observe_v1_selected_root_baseline,
    selected_root_result_artifacts,
};
pub(in crate::workspace_ops::merge) use abort::{
    interrupted_evidence_rollback_is_exact, interrupted_evidence_rollback_is_exact_view,
    normalize_evidence_observation,
};

pub(in crate::workspace_ops::merge) use finalization::{
    candidate_metadata, evidence_parent, evidence_parent_view, root_finalization_is_exact,
    root_finalization_is_exact_view, root_merge_commit,
};
pub(super) use planning::preflight_root;
pub(in crate::workspace_ops::merge) use reconciliation::frozen_manifest;

pub(crate) fn open_merge_stage_member_paths<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Vec<String>> {
    Ok(frozen_manifest(backend, root, record)?
        .members
        .into_iter()
        .filter(|member| member.active)
        .map(|member| member.path)
        .collect())
}
