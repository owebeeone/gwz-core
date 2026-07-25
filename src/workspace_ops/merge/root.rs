mod abort;
mod finalization;
mod planning;
mod reconciliation;

use std::path::Path;

use crate::git::GitBackend;
use crate::model::ModelResult;

use super::MergeOperationRecord;

pub(in crate::workspace_ops::merge) use abort::normalize_evidence_observation;

pub(in crate::workspace_ops::merge) use finalization::{
    candidate_metadata, evidence_parent, root_finalization_is_exact, root_merge_commit,
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
