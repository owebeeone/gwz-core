use std::collections::BTreeMap;

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

use super::model::*;

/// The per-participant half of a merge response.
///
/// **M5d charter §4 ("Responses").** One computation serves the open v0
/// record, the open v1 record and the archived projection, so the rows a
/// terminal answer reports have the same shape as the open answer they
/// replace. Before this the archived arm synthesized `repos: []` and default
/// counts, which is why a completed `--no-ff` merge printed
/// `participants: total 0`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace_ops::merge) struct MergeRecordRows {
    pub(in crate::workspace_ops::merge) participant_counts: crate::MergeParticipantCounts,
    pub(in crate::workspace_ops::merge) repos: Vec<crate::MergeRepoSummary>,
    pub(in crate::workspace_ops::merge) preservation: Option<Vec<crate::MergePreservation>>,
    pub(in crate::workspace_ops::merge) publication_step: Option<crate::MergePublicationStep>,
}

/// Project one record body's selected participants into response rows.
///
/// The inputs are the four fields every record version shares, so a v0 body,
/// a v1 body and an archived body of either version all project identically.
pub(in crate::workspace_ops::merge) fn record_rows(
    selected_targets: &[String],
    participants: &BTreeMap<String, MergeParticipantRecord>,
    publication: Option<&PublicationProgress>,
    source_ref: &str,
) -> ModelResult<MergeRecordRows> {
    let mut counts = crate::MergeParticipantCounts {
        total: selected_targets.len() as i64,
        ..Default::default()
    };
    let mut repos = Vec::with_capacity(selected_targets.len());
    let mut preservation = Vec::new();
    for target_id in selected_targets {
        let participant = participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        super::participant_semantics::result::increment_count(&mut counts, participant.state);
        preservation.extend(
            participant
                .preservation
                .iter()
                .map(|value| crate::MergePreservation {
                    target_id: target_id.clone(),
                    path: participant.path.clone(),
                    backup_ref: value.backup_ref.clone(),
                    backup_commit: value.backup_commit.clone(),
                    stash_id: value.stash_id.clone(),
                    stash_object_id: value.stash_object_id.clone(),
                }),
        );
        repos.push(participant.to_protocol(target_id, source_ref));
    }
    if let Some(publication) = publication {
        preservation.extend(publication.root_preservation.iter().map(|value| {
            crate::MergePreservation {
                target_id: "@root".to_owned(),
                path: ".".to_owned(),
                backup_ref: value.backup_ref.clone(),
                backup_commit: value.backup_commit.clone(),
                stash_id: value.stash_id.clone(),
                stash_object_id: value.stash_object_id.clone(),
            }
        }));
    }
    Ok(MergeRecordRows {
        participant_counts: counts,
        repos,
        preservation: (!preservation.is_empty()).then_some(preservation),
        publication_step: publication.map(|value| value.step.into()),
    })
}

/// Stable no-open result for `gwz merge --status`.
pub(crate) fn idle_status_response(
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    Ok(crate::MergeResponse {
        response: crate::operation::response_envelope_for(
            &context_meta(context),
            crate::operation::ActionKind::Merge,
            context.operation_id.clone(),
            crate::AggregateStatus::Noop,
            Vec::new(),
        )?,
        merge_id: None,
        state: crate::MergeOperationState::Idle,
        open: false,
        participant_counts: crate::MergeParticipantCounts::default(),
        repos: Vec::new(),
        operation_drift: Vec::new(),
        preservation: None,
        publication_step: None,
        record: None,
        crash_recovery: None,
    })
}

/// Build a read-only response from an immutable, already validated archive
/// projection. Archived status has no live repository observation: its stable
/// protocol evidence is the terminal outcome and acceptance carried by field
/// 10.
pub(in crate::workspace_ops::merge) fn archived_status_response(
    merge_id: &str,
    archived: &super::model::archive_projection::ArchivedMergeProjection,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    use super::model::archive_projection::ArchivedTerminalOutcome;

    let (state, aggregate) = match archived.terminal_outcome {
        ArchivedTerminalOutcome::Completed => (
            crate::MergeOperationState::Completed,
            crate::AggregateStatus::Ok,
        ),
        ArchivedTerminalOutcome::Aborted => (
            crate::MergeOperationState::Aborted,
            crate::AggregateStatus::Noop,
        ),
    };
    attach_archived_record_projection(
        crate::MergeResponse {
            response: crate::operation::response_envelope_for(
                &context_meta(context),
                crate::operation::ActionKind::Merge,
                context.operation_id.clone(),
                aggregate,
                Vec::new(),
            )?,
            merge_id: Some(merge_id.to_owned()),
            state,
            open: false,
            participant_counts: crate::MergeParticipantCounts::default(),
            repos: Vec::new(),
            operation_drift: Vec::new(),
            preservation: None,
            publication_step: None,
            record: None,
            crash_recovery: None,
        },
        merge_id,
        archived,
    )
}

/// The terminal answer a v1 start, continue or abort returns once its record
/// is archived.
///
/// **M5d charter §4 ("Responses").** A completed v1 start "returns the
/// per-repo rows, `participant_counts`, `publication_step`, and `preservation`
/// projected from the **archived** record (I2 §7: the archived projection reads
/// only the exact done-record bytes)" — so the rows below come from the decoded
/// done-record and nothing else. `archived_status_response` above stays the
/// read-only archive-history answer for `gwz merge --status <id>` and `--gc`,
/// which report no live participant work and keep their documented empty rows
/// (`gwz-cli/docs/MachineOutput.md:249-251`).
pub(in crate::workspace_ops::merge) fn archived_terminal_response(
    merge_id: &str,
    archived: &super::record_wire::ValidatedArchivedRecord,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let mut response = archived_status_response(merge_id, archived.projection(), context)?;
    let rows = archived.rows().clone();
    response.participant_counts = rows.participant_counts;
    response.repos = rows.repos;
    response.preservation = rows.preservation;
    response.publication_step = rows.publication_step;
    response.operation_drift = archived.operation_drift().to_vec();
    Ok(response)
}

/// Attach immutable archive history to an existing terminal response without
/// discarding command-specific summaries such as post-GC preservation rows.
/// The caller must derive both inputs from the same canonical archive bytes.
pub(in crate::workspace_ops::merge) fn attach_archived_record_projection(
    mut response: crate::MergeResponse,
    merge_id: &str,
    archived: &super::model::archive_projection::ArchivedMergeProjection,
) -> ModelResult<crate::MergeResponse> {
    use super::model::archive_projection::ArchivedTerminalOutcome;

    let expected_state = match archived.terminal_outcome {
        ArchivedTerminalOutcome::Completed => crate::MergeOperationState::Completed,
        ArchivedTerminalOutcome::Aborted => crate::MergeOperationState::Aborted,
    };
    if response.merge_id.as_deref() != Some(merge_id)
        || response.state != expected_state
        || response.open
    {
        return Err(ModelError::new(
            ErrorCode::InternalError,
            "terminal response does not match the validated archived merge record",
        ));
    }
    response.record = Some(super::model::project_archived(archived));
    Ok(response)
}

macro_rules! impl_record_response {
    (@open $record:ident, from_state) => { $record.state.is_open() };
    (@open $record:ident, always) => { true };
    ($record:ty, $method:ident, $visibility:vis, $projector:path, $open:ident) => {
        impl $record {
            $visibility fn $method(
                &self,
                context: &OperationContext,
            ) -> ModelResult<crate::MergeResponse> {
                let rows = record_rows(
                    &self.selected_targets,
                    &self.participants,
                    self.publication.as_ref(),
                    &self.source_ref,
                )?;
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
                    open: impl_record_response!(@open self, $open),
                    participant_counts: rows.participant_counts,
                    repos: rows.repos,
                    operation_drift: self.operation_drift.iter().map(Into::into).collect(),
                    preservation: rows.preservation,
                    publication_step: rows.publication_step,
                    record: Some($projector(self)),
                    crash_recovery: None,
                })
            }
        }
    };
}

impl_record_response!(
    MergeOperationRecord,
    to_response,
    pub(crate),
    project_open_v0,
    from_state
);
impl_record_response!(
    super::model::v1::MergeOperationRecordV1,
    to_v1_response,
    pub(in crate::workspace_ops::merge),
    project_open_v1,
    always
);

impl MergeStatusSnapshot {
    pub(crate) fn to_response(
        &self,
        context: &OperationContext,
    ) -> ModelResult<crate::MergeResponse> {
        let mut response = self.record.to_response(context)?;
        // A snapshot is built from a record discovered in `.gwz/merge`, so it
        // remains open to the workspace gate even when its terminal lifecycle
        // state means only archive completion remains. The archived response
        // returned after close continues to project `open = false`.
        response.open = true;
        for repo in &mut response.repos {
            let observation = self.participants.get(&repo.target_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::InternalError,
                    format!(
                        "merge status snapshot is missing participant '{}'",
                        repo.target_id
                    ),
                )
            })?;
            repo.live_commit.clone_from(&observation.live_commit);
            repo.conflict_paths.clone_from(&observation.conflict_paths);
            repo.continue_eligible = Some(observation.continue_eligibility.eligible);
            repo.abort_eligible = Some(observation.abort_eligibility.eligible);
            repo.drift = observation.drift.iter().map(Into::into).collect();
            repo.pending_action = observation.pending_action.as_ref().map(Into::into);
        }
        response.operation_drift = self.operation_drift.iter().map(Into::into).collect();
        Ok(response)
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

impl MergeParticipantRecord {
    pub(crate) fn to_protocol(&self, target_id: &str, source_ref: &str) -> crate::MergeRepoSummary {
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
            state: super::participant_semantics::result::wire_state(self.state),
            predicted: None,
            prediction_complete: None,
            conflict_paths: self.conflict_paths.clone(),
            continue_eligible: None,
            abort_eligible: None,
            drift: self.drift.iter().map(Into::into).collect(),
            error: self.error.as_ref().map(|error| crate::GwzError {
                code: error.code.into(),
                message: error.message.clone(),
                member_id: Some(target_id.to_owned()),
                member_path: Some(self.path.clone()),
                detail: error.detail.clone(),
                target_kind: Some(self.target_kind.into()),
                record_context: None,
            }),
            pending_action: None,
        }
    }
}

impl From<&PendingActionObservation> for crate::MergePendingActionSummary {
    fn from(value: &PendingActionObservation) -> Self {
        let kind = match value.kind {
            PendingMergeActionKind::VerifyUpToDate => crate::MergePendingActionKind::VerifyUpToDate,
            PendingMergeActionKind::FastForward => crate::MergePendingActionKind::FastForward,
            PendingMergeActionKind::TrueMerge => crate::MergePendingActionKind::TrueMerge,
            PendingMergeActionKind::ResolveConflict => {
                crate::MergePendingActionKind::ResolveConflict
            }
        };
        let state = match value.state {
            PendingActionObservationState::NotStarted => crate::MergePendingActionState::NotStarted,
            PendingActionObservationState::ExpectedConflict => {
                crate::MergePendingActionState::ExpectedConflict
            }
            PendingActionObservationState::CompletedExactly => {
                crate::MergePendingActionState::CompletedExactly
            }
            PendingActionObservationState::Ambiguous => crate::MergePendingActionState::Ambiguous,
        };
        Self {
            kind,
            state,
            message: value.message.clone(),
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
            ParticipantDriftKind::HeadDiverged => crate::MergeParticipantDriftKind::HeadDiverged,
            ParticipantDriftKind::ObjectMissing => crate::MergeParticipantDriftKind::ObjectMissing,
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
            ParticipantDriftKind::ForeignIntegrationState => {
                crate::MergeParticipantDriftKind::ForeignIntegrationState
            }
            ParticipantDriftKind::PendingActionAmbiguous => {
                crate::MergeParticipantDriftKind::PendingActionAmbiguous
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{ActionKind, OperationContext};
    use std::collections::BTreeMap;

    #[test]
    fn terminal_record_is_open_only_while_discovered_in_open_storage() {
        let record: MergeOperationRecord = serde_yaml::from_str(
            r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_start, state: aborted, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#,
        )
        .unwrap();
        let context = OperationContext {
            operation_id: "op_status".to_owned(),
            request_id: "req".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            action: ActionKind::Merge,
            dry_run: false,
            attribution: None,
        };

        assert!(!record.to_response(&context).unwrap().open);
        let snapshot = MergeStatusSnapshot {
            record,
            participants: BTreeMap::new(),
            operation_drift: Vec::new(),
        };
        assert!(snapshot.to_response(&context).unwrap().open);
    }
}
