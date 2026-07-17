use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

use super::model::*;

impl MergeOperationRecord {
    pub(crate) fn to_response(
        &self,
        context: &OperationContext,
    ) -> ModelResult<crate::MergeResponse> {
        let mut counts = crate::MergeParticipantCounts {
            total: self.selected_targets.len() as i64,
            ..crate::MergeParticipantCounts::default()
        };
        let mut repos = Vec::with_capacity(self.selected_targets.len());
        let mut preservation = Vec::new();
        for target_id in &self.selected_targets {
            let participant = self.participants.get(target_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("merge record is missing participant '{target_id}'"),
                )
            })?;
            increment_count(&mut counts, participant.state);
            preservation.extend(participant.preservation.iter().map(|evidence| {
                crate::MergePreservation {
                    target_id: target_id.clone(),
                    path: participant.path.clone(),
                    backup_ref: evidence.backup_ref.clone(),
                    backup_commit: evidence.backup_commit.clone(),
                    stash_id: evidence.stash_id.clone(),
                    stash_object_id: evidence.stash_object_id.clone(),
                }
            }));
            repos.push(participant.to_protocol(target_id, &self.source_ref));
        }
        Ok(crate::MergeResponse {
            response: crate::operation::response_envelope_for(
                &context_meta(context),
                crate::operation::ActionKind::Merge,
                context.operation_id.clone(),
                aggregate_status(self.state),
                Vec::new(),
            )?,
            merge_id: Some(self.merge_id.clone()),
            state: self.state.into(),
            open: self.state.is_open(),
            participant_counts: counts,
            repos,
            operation_drift: self.operation_drift.iter().map(Into::into).collect(),
            preservation: (!preservation.is_empty()).then_some(preservation),
            publication_step: self.publication.as_ref().map(|value| value.step.into()),
        })
    }
}

fn context_meta(context: &OperationContext) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: context.request_id.clone(),
        schema_version: context.schema_version.clone(),
        attribution: context.attribution.as_ref().map(Into::into),
        ..crate::RequestMeta::default()
    }
}

fn aggregate_status(state: OperationState) -> crate::AggregateStatus {
    match state {
        OperationState::Completed => crate::AggregateStatus::Ok,
        OperationState::Aborted => crate::AggregateStatus::Noop,
        OperationState::AwaitingResolution => crate::AggregateStatus::Conflicted,
        OperationState::Halted | OperationState::RecoveryRequired => crate::AggregateStatus::Failed,
        _ => crate::AggregateStatus::Accepted,
    }
}

fn increment_count(counts: &mut crate::MergeParticipantCounts, state: ParticipantState) {
    match state {
        ParticipantState::Planned => counts.planned += 1,
        ParticipantState::UpToDate => counts.up_to_date += 1,
        ParticipantState::FastForwarded => counts.fast_forwarded += 1,
        ParticipantState::Merged => counts.merged += 1,
        ParticipantState::Conflicted => counts.conflicted += 1,
        ParticipantState::Failed => counts.failed += 1,
        ParticipantState::Unattempted => counts.unattempted += 1,
        ParticipantState::Continued => counts.continued += 1,
        ParticipantState::Aborted => counts.aborted += 1,
        ParticipantState::RolledBack => counts.rolled_back += 1,
    }
}

impl MergeParticipantRecord {
    fn to_protocol(&self, target_id: &str, source_ref: &str) -> crate::MergeRepoSummary {
        crate::MergeRepoSummary {
            target_id: target_id.to_owned(),
            target_kind: self.target_kind.into(),
            path: self.path.clone(),
            source_ref: source_ref.to_owned(),
            source_commit: self.source_commit.clone(),
            target_branch: self.target_branch.clone(),
            before_commit: self.before_commit.clone(),
            resulting_commit: self.resulting_commit.clone(),
            live_commit: None,
            state: self.state.into(),
            predicted: None,
            prediction_complete: None,
            conflict_paths: self.conflict_paths.clone(),
            continue_eligible: None,
            abort_eligible: None,
            drift: self.drift.iter().map(Into::into).collect(),
            error: self.error.as_ref().map(|error| crate::GwzError {
                code: crate::GwzErrorCode::InternalError,
                message: error.message.clone(),
                member_id: Some(target_id.to_owned()),
                member_path: Some(self.path.clone()),
                detail: Some(error.code.clone()),
                target_kind: Some(self.target_kind.into()),
            }),
        }
    }
}

impl From<MergeTargetKind> for crate::TargetKind {
    fn from(value: MergeTargetKind) -> Self {
        match value {
            MergeTargetKind::Member => Self::Member,
            MergeTargetKind::Root => Self::Root,
        }
    }
}

impl From<ParticipantState> for crate::MergeParticipantState {
    fn from(value: ParticipantState) -> Self {
        match value {
            ParticipantState::Planned => Self::Planned,
            ParticipantState::UpToDate => Self::UpToDate,
            ParticipantState::FastForwarded => Self::FastForwarded,
            ParticipantState::Merged => Self::Merged,
            ParticipantState::Conflicted => Self::Conflicted,
            ParticipantState::Failed => Self::Failed,
            ParticipantState::Unattempted => Self::Unattempted,
            ParticipantState::Continued => Self::Continued,
            ParticipantState::Aborted => Self::Aborted,
            ParticipantState::RolledBack => Self::RolledBack,
        }
    }
}

impl From<OperationState> for crate::MergeOperationState {
    fn from(value: OperationState) -> Self {
        match value {
            OperationState::Executing => Self::Executing,
            OperationState::AwaitingResolution => Self::AwaitingResolution,
            OperationState::Halted => Self::Halted,
            OperationState::Finalizing => Self::Finalizing,
            OperationState::Preserving => Self::Preserving,
            OperationState::RollingBack => Self::RollingBack,
            OperationState::Completed => Self::Completed,
            OperationState::Aborted => Self::Aborted,
            OperationState::RecoveryRequired => Self::RecoveryRequired,
        }
    }
}

impl From<PublicationStep> for crate::MergePublicationStep {
    fn from(value: PublicationStep) -> Self {
        match value {
            PublicationStep::NotStarted => Self::NotStarted,
            PublicationStep::ValidatingResults => Self::ValidatingResults,
            PublicationStep::PreparingCandidate => Self::PreparingCandidate,
            PublicationStep::CommittingEvidence => Self::CommittingEvidence,
            PublicationStep::PublishingCandidate => Self::PublishingCandidate,
            PublicationStep::VerifyingPublication => Self::VerifyingPublication,
            PublicationStep::Complete => Self::Complete,
        }
    }
}

impl From<&ParticipantDrift> for crate::MergeParticipantDrift {
    fn from(value: &ParticipantDrift) -> Self {
        let kind = match value.kind {
            ParticipantDriftKind::BranchChanged => crate::MergeParticipantDriftKind::BranchChanged,
            ParticipantDriftKind::HeadAdvanced => crate::MergeParticipantDriftKind::HeadAdvanced,
            ParticipantDriftKind::HeadRewound => crate::MergeParticipantDriftKind::HeadRewound,
            ParticipantDriftKind::TargetRefChanged => {
                crate::MergeParticipantDriftKind::TargetRefChanged
            }
            ParticipantDriftKind::WorktreeModified => {
                crate::MergeParticipantDriftKind::WorktreeModified
            }
            ParticipantDriftKind::IndexModified => crate::MergeParticipantDriftKind::IndexModified,
            ParticipantDriftKind::MergeStateMissing => {
                crate::MergeParticipantDriftKind::MergeStateMissing
            }
            ParticipantDriftKind::MergeHeadChanged => {
                crate::MergeParticipantDriftKind::MergeHeadChanged
            }
            ParticipantDriftKind::NewIntegrationState => {
                crate::MergeParticipantDriftKind::NewIntegrationState
            }
            ParticipantDriftKind::RepositoryMissing => {
                crate::MergeParticipantDriftKind::RepositoryMissing
            }
        };
        Self {
            kind,
            message: value.message.clone(),
            expected_branch: value.expected_branch.clone(),
            live_branch: value.live_branch.clone(),
            expected_head: value.expected_head.clone(),
            live_head: value.live_head.clone(),
            expected_merge_head: value.expected_merge_head.clone(),
            live_merge_head: value.live_merge_head.clone(),
        }
    }
}

impl From<&OperationDrift> for crate::MergeOperationDrift {
    fn from(value: &OperationDrift) -> Self {
        let kind = match value.kind {
            OperationDriftKind::BaselineLockChanged => {
                crate::MergeOperationDriftKind::BaselineLockChanged
            }
            OperationDriftKind::BaselineManifestChanged => {
                crate::MergeOperationDriftKind::BaselineManifestChanged
            }
            OperationDriftKind::RootCandidateMetadataInvalid => {
                crate::MergeOperationDriftKind::RootCandidateMetadataInvalid
            }
            OperationDriftKind::RootCandidateStateChanged => {
                crate::MergeOperationDriftKind::RootCandidateStateChanged
            }
            OperationDriftKind::RecordUnreadable => {
                crate::MergeOperationDriftKind::RecordUnreadable
            }
        };
        Self {
            kind,
            message: value.message.clone(),
        }
    }
}
