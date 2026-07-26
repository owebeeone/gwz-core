use super::super::{MergeOperationRecord, MergeParticipantPlan, MergeStore};
use super::prepared::{Row, execute_prepared, prepare_one};
use super::record::{apply_row, set_pending_action};
use crate::MergeParticipantState as PState;
use crate::git::{
    GitBackend, GitHeadState, GitIntegrateResult, GitMergeAnalysis, GitPreparedMerge, GitStatus,
};
use crate::model::{ModelError, ModelResult};
use crate::operation::EventEmitter;
use std::path::Path;

pub(super) fn execute_durable<B: ExecutionBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    participants: &[MergeParticipantPlan],
    attribution: Option<&crate::model::OperationAttribution>,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for (index, participant) in participants.iter().enumerate() {
        emitter.member_started(&participant.target_id, &participant.path);
        let prepared = match prepare_one(backend, root, participant, attribution) {
            Ok(prepared) => prepared,
            Err(error) => {
                persist_start_failure(store, root, record, participant, &error, emitter)?;
                mark_later_unattempted(store, root, record, &participants[index + 1..], emitter)?;
                break;
            }
        };
        set_pending_action(record, participant, &prepared)?;
        super::super::persist_merge_record(store, root, record, emitter)?;
        match execute_prepared(backend, root, participant, &prepared) {
            Ok(row) => {
                let conflict_snapshot = if row.state == PState::Conflicted {
                    backend
                        .merge_conflict_snapshot(
                            &root.join(&participant.path),
                            &participant.before_commit,
                            &participant.source_commit,
                        )?
                        .files
                        .into_iter()
                        .map(|file| super::super::ConflictFileEvidence {
                            path: file.path,
                            sha256: file.sha256,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                apply_row(record, participant, &row, None, conflict_snapshot)?;
                record
                    .participants
                    .get_mut(&participant.target_id)
                    .expect("participant was validated before execution")
                    .pending_action = None;
                super::super::persist_merge_record(store, root, record, emitter)?;
                super::super::emit_merge_member_finished(emitter, record, &participant.target_id)?;
            }
            Err(error) => {
                persist_start_failure(store, root, record, participant, &error, emitter)?;
                mark_later_unattempted(store, root, record, &participants[index + 1..], emitter)?;
                break;
            }
        }
    }
    Ok(())
}

fn persist_start_failure<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    participant: &MergeParticipantPlan,
    error: &ModelError,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let contextual = error
        .clone()
        .with_member(&participant.target_id, &participant.path);
    apply_row(
        record,
        participant,
        &Row::new(participant, PState::Failed),
        Some(&contextual),
        Vec::new(),
    )?;
    super::super::persist_merge_record(store, root, record, emitter)?;
    super::super::emit_merge_member_finished(emitter, record, &participant.target_id)
}

fn mark_later_unattempted<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    later: &[MergeParticipantPlan],
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    for participant in later {
        apply_row(
            record,
            participant,
            &Row::new(participant, PState::Unattempted),
            None,
            Vec::new(),
        )?;
        super::super::persist_merge_record(store, root, record, emitter)?;
        super::super::emit_merge_member_finished(emitter, record, &participant.target_id)?;
    }
    Ok(())
}

pub(super) type Inspection = (GitStatus, GitHeadState, GitMergeAnalysis);
pub(super) trait ExecutionBackend {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection>;
    fn prepare_merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge>;
    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult>;
    fn merge_conflict_snapshot(
        &self,
        _path: &Path,
        _expected_before: &str,
        _expected_merge_head: &str,
    ) -> ModelResult<crate::git::GitMergeConflictSnapshot> {
        Ok(crate::git::GitMergeConflictSnapshot { files: Vec::new() })
    }
}
impl<B: GitBackend> ExecutionBackend for B {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
        Ok((
            self.status(path)?,
            self.head(path)?,
            self.merge_analysis(path, branch, source)?,
        ))
    }
    fn prepare_merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        GitBackend::prepare_merge_upstream_checked(
            self,
            path,
            branch,
            expected_before,
            source,
            attribution,
        )
    }
    fn merge_conflict_snapshot(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
    ) -> ModelResult<crate::git::GitMergeConflictSnapshot> {
        GitBackend::merge_conflict_snapshot(self, path, expected_before, expected_merge_head)
    }
    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        GitBackend::execute_prepared_merge_upstream_checked(
            self,
            path,
            branch,
            expected_before,
            source,
            message,
            prepared,
        )
    }
}
