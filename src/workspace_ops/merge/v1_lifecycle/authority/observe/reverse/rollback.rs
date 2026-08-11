use super::super::super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1, MergeOperationRecordV1, ParticipantRollbackKindV1,
    PendingRollbackActionV1, RecoveryOriginStateV1, RollbackCursor, RootMetadataRollbackStepV1,
    rollback_cursor,
};
use crate::workspace_ops::merge::status::PendingActionReconciliation;
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    PreparedReverseEntryView, ReverseEntryKind, ReverseEntryPredecessor, preview_reverse_entry,
    visit_reverse_entry,
};
use crate::workspace_ops::merge::{
    ConflictFileEvidence, MergeParticipantRecord, OperationState, ParticipantState,
    PendingMergeActionKind,
};

pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn observe<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    let fact = match request.kind() {
        ObservationKind::ParticipantAction { member_id }
            if request.lifecycle() == V1LifecycleRequest::Abort =>
        {
            observe_abort_participant(backend, context, current, member_id)?
        }
        ObservationKind::RollbackEntry if request.lifecycle() == V1LifecycleRequest::Abort => {
            observe_entry(backend, context, current)?
        }
        ObservationKind::RollbackCursor => observe_cursor(backend, current)?,
        ObservationKind::Recovery => {
            ExactObservationFact::Completed(CompletedObservation::Recovery(
                super::rolling_back_recovery::verify_recovery_origin(backend, current)?,
            ))
        }
        _ => {
            return Err(recovery_error(
                "rollback lane received a non-rollback observation",
            ));
        }
    };
    BoundExactObservation::issue(current, request, fact)
}

fn observe_entry<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let preview = preview_reverse_entry(
        current,
        V1LifecycleRequest::Abort,
        ReverseEntryPredecessor::ActionFree,
    )?;
    match observe_reverse_publication_handoff(backend, context, current, &preview)? {
        RecordEvidenceOr::RecordEvidence(proof) => Ok(ExactObservationFact::Completed(
            CompletedObservation::Publication(PublicationObservation::EvidenceResult(proof)),
        )),
        RecordEvidenceOr::Ready(handoff) => {
            let preflight =
                preflight_entry_with_handoff(backend, current, &preview, handoff.value())?;
            let entry = prepare_direct_rollback_entry(current, &preview, handoff, preflight)?;
            Ok(ExactObservationFact::Completed(
                CompletedObservation::RollbackEntry(Box::new(entry)),
            ))
        }
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_cursor<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let record = current.record();
    if let Some(action) = record.pending_rollback.as_ref() {
        return observe_pending(backend, current, action);
    }
    match rollback_cursor(record) {
        RollbackCursor::PublicationEvidence => evidence_intent(current),
        RollbackCursor::Participant {
            member_id,
            action,
            terminal_state,
        } => participant_intent(current, member_id, action, terminal_state),
        RollbackCursor::NoMutationParticipant { member_id } => {
            let row = participant(current, member_id)?;
            crate::workspace_ops::merge::abort::verify_v1_no_mutation_participant(
                backend,
                current.location().root(),
                current.record(),
                member_id,
                row,
            )?;
            let proof = no_mutation_abort(current)?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::NoMutation(Box::new(proof)),
            )))
        }
        RollbackCursor::SelectedRootMetadata => {
            let observed = crate::workspace_ops::merge::root::observe_v1_root_metadata_rollback(
                backend,
                current.location().root(),
                record,
                RootMetadataRollbackStepV1::Complete,
            )?;
            if observed == crate::workspace_ops::merge::root::V1RootRollbackObservation::After {
                exhausted(current)
            } else if observed
                == crate::workspace_ops::merge::root::V1RootRollbackObservation::Ambiguous
            {
                Err(recovery_error(
                    "selected-root metadata is ambiguous between rollback actions",
                ))
            } else {
                root_intent(current)
            }
        }
        RollbackCursor::Complete => exhausted(current),
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_abort_participant<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    member_id: &str,
) -> ModelResult<ExactObservationFact> {
    let row = participant(current, member_id)?;
    let pending = row.pending_action.as_ref().ok_or_else(|| {
        member_error(
            member_id,
            row,
            "abort participant has no retained forward owner",
        )
    })?;
    match crate::workspace_ops::merge::status::reconcile_pending_action(
        backend,
        current.location().root(),
        member_id,
        row,
    )? {
        PendingActionReconciliation::NotStarted
            if pending.kind == PendingMergeActionKind::VerifyUpToDate =>
        {
            let next = outcome_row(
                row,
                ParticipantState::UpToDate,
                Some(row.before_commit.clone()),
                Vec::new(),
                Vec::new(),
            );
            outcome_with_entry(backend, context, current, member_id, next)
        }
        PendingActionReconciliation::NotStarted => {
            let proof = VerifiedParticipantNotStarted::issue(
                &AuthorityIssuer::for_observer(current),
                member_id,
                "participant_not_started",
                "verified",
                member_id.into(),
            )?;
            let entry = prepare_entry(
                backend,
                context,
                current,
                ReverseEntryPredecessor::ParticipantNotStarted(&proof),
            )?;
            Ok(ExactObservationFact::Abandon(
                Box::new(proof),
                EntryFact::Rollback(Box::new(entry)),
            ))
        }
        PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
            if pending.kind != PendingMergeActionKind::TrueMerge {
                return ambiguity(current);
            }
            let path = crate::workspace_ops::merge::status::validated_participant_path(
                current.location().root(),
                member_id,
                row,
            )?;
            let snapshot = match backend.merge_conflict_snapshot(
                &path,
                &row.before_commit,
                &row.source_commit,
            ) {
                Ok(value) => value
                    .files
                    .into_iter()
                    .map(|file| ConflictFileEvidence {
                        path: file.path,
                        sha256: file.sha256,
                    })
                    .collect(),
                Err(error) if semantic_drift(&error) => return ambiguity(current),
                Err(error) => return Err(error),
            };
            let next = outcome_row(
                row,
                ParticipantState::Conflicted,
                None,
                conflict_paths,
                snapshot,
            );
            outcome_with_entry(backend, context, current, member_id, next)
        }
        PendingActionReconciliation::Completed { resulting_commit } => {
            let state = match pending.kind {
                PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
            };
            let next = outcome_row(row, state, Some(resulting_commit), Vec::new(), Vec::new());
            outcome_with_entry(backend, context, current, member_id, next)
        }
        PendingActionReconciliation::Ambiguous { .. } => ambiguity(current),
    }
}

fn prepare_entry<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    predecessor: ReverseEntryPredecessor<'_>,
) -> ModelResult<PreparedRollbackEntry> {
    let preview = preview_reverse_entry(current, V1LifecycleRequest::Abort, predecessor)?;
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(backend, context, current, &preview)?
    else {
        return Err(recovery_error(
            "rollback entry must record publication evidence before participant retirement",
        ));
    };
    let preflight = preflight_entry_with_handoff(backend, current, &preview, handoff.value())?;
    prepare_direct_rollback_entry(current, &preview, handoff, preflight)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn preflight_entry_with_handoff<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: &ReverseEntryAuthorityPayload,
) -> ModelResult<VerifiedRollbackEntryPreflight> {
    let issuer = AuthorityIssuer::for_observer(current);
    let permit = ReverseEntryInspectionPermit::issue(&issuer)?;
    let mut visitor = RollbackEntryHandoffVisitor { backend, handoff };
    visit_reverse_entry(permit, current, preview, &mut visitor)
}

struct RollbackEntryHandoffVisitor<'a, B> {
    backend: &'a B,
    handoff: &'a ReverseEntryAuthorityPayload,
}

impl<B> super::super::reverse_entry_visitor_seal::Visitor for RollbackEntryHandoffVisitor<'_, B> {}

impl<B: GitBackend> SealedReverseEntryVisitor for RollbackEntryHandoffVisitor<'_, B> {
    type SealedAuthority = VerifiedRollbackEntryPreflight;

    fn inspect(
        &mut self,
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        request: V1LifecycleRequest,
        kind: ReverseEntryKind,
        anticipated_model_sha256: [u8; 32],
    ) -> ModelResult<Self::SealedAuthority> {
        let expected = ReverseEntryAuthorityPayload {
            request,
            kind,
            anticipated_model_sha256,
            publication: self.handoff.publication,
        };
        if self.handoff != &expected {
            return Err(recovery_error(
                "rollback handoff does not match the preview",
            ));
        }
        crate::workspace_ops::merge::abort::preflight_v1_rollback(
            self.backend,
            current.location().root(),
            anticipated,
        )?;
        crate::workspace_ops::merge::abort::preflight_v1_evidence(
            self.backend,
            current.location().root(),
            anticipated,
        )?;
        VerifiedRollbackEntryPreflight::issue(
            &AuthorityIssuer::for_observer(current),
            "@operation",
            "rollback_entry_preflight",
            "verified",
            expected,
        )
    }
}

fn observe_pending<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    action: &PendingRollbackActionV1,
) -> ModelResult<ExactObservationFact> {
    match action {
        PendingRollbackActionV1::Participant {
            member_id,
            action,
            terminal_state,
        } => observe_participant(backend, current, member_id, *action, *terminal_state),
        PendingRollbackActionV1::PublicationEvidence { next_step } => {
            observe_evidence(backend, current, *next_step)
        }
        PendingRollbackActionV1::SelectedRootMetadata { next_step } => {
            observe_root(backend, current, *next_step)
        }
    }
}

fn participant_intent(
    current: &StoredV1Record,
    member_id: &str,
    action: ParticipantRollbackKindV1,
    terminal_state: ParticipantState,
) -> ModelResult<ExactObservationFact> {
    let pending = PendingRollbackActionV1::Participant {
        member_id: member_id.into(),
        action,
        terminal_state,
    };
    let proof = PreparedParticipantRollback::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "begin_participant_rollback",
        "prepared",
        pending,
    )?;
    Ok(completed(CompletedObservation::Rollback(
        RollbackObservation::ParticipantIntent(Box::new(proof)),
    )))
}

fn observe_participant<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    member_id: &str,
    action: ParticipantRollbackKindV1,
    terminal_state: ParticipantState,
) -> ModelResult<ExactObservationFact> {
    let row = participant(current, member_id)?;
    match crate::workspace_ops::merge::abort::observe_v1_participant_rollback(
        backend,
        current.location().root(),
        current.record(),
        member_id,
        row,
        action,
    )? {
        crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::Before => {
            Ok(ExactObservationFact::NotStarted(
                NotStartedObservation::Rollback(current.record().pending_rollback.clone().unwrap()),
            ))
        }
        crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::After => {
            let mut next = row.clone();
            next.state = terminal_state;
            next.pending_action = None;
            let proof = VerifiedParticipantRollback::issue(
                &AuthorityIssuer::for_observer(current),
                member_id,
                "finish_participant_rollback",
                "completed",
                ParticipantActionPayload {
                    member_id: member_id.into(),
                    row: next,
                },
            )?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::ParticipantDone(Box::new(proof)),
            )))
        }
        crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::Ambiguous => {
            ambiguity(current)
        }
    }
}

fn evidence_intent(current: &StoredV1Record) -> ModelResult<ExactObservationFact> {
    let pending = PendingRollbackActionV1::PublicationEvidence {
        next_step: EvidenceRollbackStepV1::EvidenceCommit,
    };
    let proof = PreparedEvidenceRollback::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "begin_evidence_rollback",
        "prepared",
        pending,
    )?;
    Ok(completed(CompletedObservation::Rollback(
        RollbackObservation::EvidenceIntent(Box::new(proof)),
    )))
}

fn observe_evidence<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    step: EvidenceRollbackStepV1,
) -> ModelResult<ExactObservationFact> {
    use crate::workspace_ops::merge::abort::V1EvidenceRollbackObservation as O;
    match crate::workspace_ops::merge::abort::observe_v1_evidence_rollback(
        backend,
        current.location().root(),
        current.record(),
        step,
    )? {
        O::Before => Ok(ExactObservationFact::NotStarted(
            NotStartedObservation::Rollback(current.record().pending_rollback.clone().unwrap()),
        )),
        O::After if step == EvidenceRollbackStepV1::Complete => {
            let proof = VerifiedEvidenceRollbackCompletion::issue(
                &AuthorityIssuer::for_observer(current),
                "@publication",
                "finish_evidence_rollback",
                "complete",
                (),
            )?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::EvidenceDone(proof),
            )))
        }
        O::After => {
            let next = next_evidence(step).unwrap();
            let proof = VerifiedEvidenceRollbackStep::issue(
                &AuthorityIssuer::for_observer(current),
                "@publication",
                "advance_evidence_rollback",
                evidence_phase(step),
                PendingRollbackActionV1::PublicationEvidence { next_step: next },
            )?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::EvidenceStep(Box::new(proof)),
            )))
        }
        O::Ambiguous => ambiguity(current),
    }
}

fn root_intent(current: &StoredV1Record) -> ModelResult<ExactObservationFact> {
    let pending = PendingRollbackActionV1::SelectedRootMetadata {
        next_step: RootMetadataRollbackStepV1::Manifest,
    };
    let proof = PreparedRootMetadataRollback::issue(
        &AuthorityIssuer::for_observer(current),
        "@root",
        "begin_root_metadata_rollback",
        "prepared",
        pending,
    )?;
    Ok(completed(CompletedObservation::Rollback(
        RollbackObservation::RootIntent(Box::new(proof)),
    )))
}

fn observe_root<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    step: RootMetadataRollbackStepV1,
) -> ModelResult<ExactObservationFact> {
    use crate::workspace_ops::merge::root::V1RootRollbackObservation as O;
    match crate::workspace_ops::merge::root::observe_v1_root_metadata_rollback(
        backend,
        current.location().root(),
        current.record(),
        step,
    )? {
        O::Before => Ok(ExactObservationFact::NotStarted(
            NotStartedObservation::Rollback(current.record().pending_rollback.clone().unwrap()),
        )),
        O::After if step == RootMetadataRollbackStepV1::Complete => {
            let proof = VerifiedRootMetadataRollbackCompletion::issue(
                &AuthorityIssuer::for_observer(current),
                "@root",
                "finish_root_metadata_rollback",
                "complete",
                (),
            )?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::RootDone(proof),
            )))
        }
        O::After => {
            let next = next_root(step).unwrap();
            let proof = VerifiedRootMetadataRollbackStep::issue(
                &AuthorityIssuer::for_observer(current),
                "@root",
                "advance_root_metadata_rollback",
                root_phase(step),
                PendingRollbackActionV1::SelectedRootMetadata { next_step: next },
            )?;
            Ok(completed(CompletedObservation::Rollback(
                RollbackObservation::RootStep(Box::new(proof)),
            )))
        }
        O::Ambiguous => ambiguity(current),
    }
}

fn exhausted(current: &StoredV1Record) -> ModelResult<ExactObservationFact> {
    let proof = rollback_exhausted(current)?;
    Ok(completed(CompletedObservation::Rollback(
        RollbackObservation::Exhausted(proof),
    )))
}

fn outcome_with_entry<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    member_id: &str,
    row: MergeParticipantRecord,
) -> ModelResult<ExactObservationFact> {
    let proof = VerifiedParticipantOutcome::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: member_id.into(),
            row,
        },
    )?;
    let entry = if current.record().state == OperationState::Halted {
        EntryFact::Rollback(Box::new(prepare_entry(
            backend,
            context,
            current,
            ReverseEntryPredecessor::ParticipantOutcome(&proof),
        )?))
    } else {
        EntryFact::None
    };
    Ok(completed(CompletedObservation::Participant(
        ParticipantObservation::Outcome(Box::new(proof), entry),
    )))
}

fn outcome_row(
    prior: &MergeParticipantRecord,
    state: ParticipantState,
    resulting_commit: Option<String>,
    conflict_paths: Vec<String>,
    conflict_snapshot: Vec<ConflictFileEvidence>,
) -> MergeParticipantRecord {
    let mut row = prior.clone();
    row.state = state;
    row.resulting_commit = resulting_commit;
    row.expected_merge_head =
        (state == ParticipantState::Conflicted).then(|| prior.source_commit.clone());
    row.conflict_paths = conflict_paths;
    row.conflict_snapshot = conflict_snapshot;
    row.error = None;
    row.pending_action = None;
    row
}

fn participant<'a>(
    current: &'a StoredV1Record,
    member_id: &str,
) -> ModelResult<&'a MergeParticipantRecord> {
    current.record().participants.get(member_id).ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("rollback participant '{member_id}' is missing"),
        )
    })
}

fn ambiguity(current: &StoredV1Record) -> ModelResult<ExactObservationFact> {
    let proof = BoundAmbiguityEvidence::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "enter_recovery",
        "ambiguous",
        RecoveryOriginStateV1::RollingBack,
    )?;
    Ok(ExactObservationFact::Ambiguous(proof))
}

fn completed(value: CompletedObservation) -> ExactObservationFact {
    ExactObservationFact::Completed(value)
}

fn next_evidence(step: EvidenceRollbackStepV1) -> Option<EvidenceRollbackStepV1> {
    Some(match step {
        EvidenceRollbackStepV1::EvidenceCommit => EvidenceRollbackStepV1::Boundary,
        EvidenceRollbackStepV1::Boundary => EvidenceRollbackStepV1::Lock,
        EvidenceRollbackStepV1::Lock => EvidenceRollbackStepV1::Marker,
        EvidenceRollbackStepV1::Marker => EvidenceRollbackStepV1::Index,
        EvidenceRollbackStepV1::Index => EvidenceRollbackStepV1::Complete,
        EvidenceRollbackStepV1::Complete => return None,
    })
}

fn next_root(step: RootMetadataRollbackStepV1) -> Option<RootMetadataRollbackStepV1> {
    Some(match step {
        RootMetadataRollbackStepV1::Manifest => RootMetadataRollbackStepV1::Lock,
        RootMetadataRollbackStepV1::Lock => RootMetadataRollbackStepV1::Complete,
        RootMetadataRollbackStepV1::Complete => return None,
    })
}

fn evidence_phase(step: EvidenceRollbackStepV1) -> &'static str {
    match step {
        EvidenceRollbackStepV1::EvidenceCommit => "evidence_commit",
        EvidenceRollbackStepV1::Boundary => "boundary",
        EvidenceRollbackStepV1::Lock => "lock",
        EvidenceRollbackStepV1::Marker => "marker",
        EvidenceRollbackStepV1::Index => "index",
        EvidenceRollbackStepV1::Complete => "complete",
    }
}

fn root_phase(step: RootMetadataRollbackStepV1) -> &'static str {
    match step {
        RootMetadataRollbackStepV1::Manifest => "manifest",
        RootMetadataRollbackStepV1::Lock => "lock",
        RootMetadataRollbackStepV1::Complete => "complete",
    }
}

fn semantic_drift(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::DirtyMember
            | ErrorCode::MergeDrift
            | ErrorCode::MergeRecoveryRequired
            | ErrorCode::AcceptanceInputDrift
    )
}

fn member_error(member_id: &str, row: &MergeParticipantRecord, detail: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail).with_member(member_id, &row.path)
}

fn recovery_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail.into())
}
