use super::*;
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, ParticipantRollbackKindV1};
use crate::workspace_ops::merge::preserve::{
    V1BundleObservation, v1_bundle_observation, v1_preservation_image, v1_preservation_owners,
};
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    PreparedReverseEntryView, ReverseEntryKind, ReverseEntryPredecessor, preview_reverse_entry,
    visit_reverse_entry,
};
use crate::workspace_ops::merge::{OperationState, ParticipantState};

pub(super) fn observe_entry<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let preview = preview_reverse_entry(
        current,
        V1LifecycleRequest::Preserve,
        ReverseEntryPredecessor::ActionFree,
    )?;
    match observe_reverse_publication_handoff(backend, context, current, &preview)? {
        RecordEvidenceOr::RecordEvidence(proof) => Ok(completed(
            CompletedObservation::Publication(PublicationObservation::EvidenceResult(proof)),
        )),
        RecordEvidenceOr::Ready(handoff) => {
            let preflight = preflight_entry(backend, current, &preview, handoff.value())?;
            let entry = prepare_preservation_entry(current, &preview, handoff, preflight)?;
            Ok(completed(CompletedObservation::PreservationEntry(
                Box::new(entry),
            )))
        }
    }
}

pub(super) fn preflight_entry<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    handoff: &ReverseEntryAuthorityPayload,
) -> ModelResult<VerifiedPreservationEntryPreflight> {
    let issuer = AuthorityIssuer::for_observer(current);
    let permit = ReverseEntryInspectionPermit::issue(&issuer)?;
    let mut visitor = PreservationEntryVisitor { backend, handoff };
    visit_reverse_entry(permit, current, preview, &mut visitor)
}

struct PreservationEntryVisitor<'a, B> {
    backend: &'a B,
    handoff: &'a ReverseEntryAuthorityPayload,
}

impl<B> super::super::super::reverse_entry_visitor_seal::Visitor
    for PreservationEntryVisitor<'_, B>
{
}

impl<B: MergeAuthorityBackend> SealedReverseEntryVisitor for PreservationEntryVisitor<'_, B> {
    type SealedAuthority = VerifiedPreservationEntryPreflight;

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
        if self.handoff != &expected || kind != ReverseEntryKind::Preservation {
            return Err(preservation_error(
                "preservation handoff does not match its preview",
            ));
        }

        if !anticipated.operation_drift.is_empty() {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "operation drift prevents coordinated preservation entry",
            ));
        }
        crate::workspace_ops::merge::v1_rollback::preflight_v1_evidence(
            self.backend,
            current.location().root(),
            anticipated,
        )?;

        let mut preserving = anticipated.clone();
        preserving.preservation_publication_handoff = Some(model_handoff(self.handoff.publication));
        let plans = v1_preservation_owners(self.backend, current.location().root(), &preserving)?;
        preflight_non_preservation_participants(self.backend, current, anticipated, &plans)?;
        for plan in &plans {
            // Reading every image is part of the global preflight. It proves the
            // selected owner set and each immutable anchor are observable before
            // any preservation entry record can be durably committed.
            let _ = v1_preservation_image(self.backend, &preserving, plan, &plan.protected_commit)?;
            let bundle =
                v1_bundle_observation(current.location().root(), &preserving, &plans, &plan.owner)
                    .map_err(|error| attach_member(error, &plan.target_id, &plan.relative_path))?;
            if bundle != V1BundleObservation::Before {
                return Err(owner_error(
                    plan,
                    "preservation bundle path is not an exact empty entry prefix",
                ));
            }
        }

        VerifiedPreservationEntryPreflight::issue(
            &AuthorityIssuer::for_observer(current),
            "@operation",
            "preservation_entry_preflight",
            "verified",
            expected,
        )
    }
}

fn preflight_non_preservation_participants<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    record: &MergeOperationRecordV1,
    plans: &[V1PreservationOwnerPlan],
) -> ModelResult<()> {
    for member_id in &record.selected_targets {
        if plans.iter().any(|plan| plan.target_id == *member_id) {
            continue;
        }
        let row = record.participants.get(member_id).ok_or_else(|| {
            preservation_error(format!(
                "selected preservation participant '{member_id}' is missing"
            ))
        })?;
        match row.state {
            ParticipantState::Conflicted => {
                let observed = crate::workspace_ops::merge::v1_rollback::observe_v1_participant_rollback(
                    backend,
                    current.location().root(),
                    record,
                    member_id,
                    row,
                    ParticipantRollbackKindV1::AbortConflict,
                )
                .map_err(|error| attach_member(error, member_id, &row.path))?;
                if observed
                    == crate::workspace_ops::merge::v1_rollback::V1ParticipantRollbackObservation::Ambiguous
                {
                    return Err(ModelError::new(
                        ErrorCode::MergeRecoveryRequired,
                        "conflicted participant has no exact preservable rollback form",
                    )
                    .with_member(member_id, &row.path));
                }
            }
            ParticipantState::Planned
            | ParticipantState::Failed
            | ParticipantState::Unattempted => {
                crate::workspace_ops::merge::v1_rollback::verify_v1_no_mutation_participant(
                    backend,
                    current.location().root(),
                    record,
                    member_id,
                    row,
                )
                .map_err(|error| attach_member(error, member_id, &row.path))?;
            }
            ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Continued => {
                return Err(ModelError::new(
                    ErrorCode::PreservationEvidenceMismatch,
                    "successful participant is missing from the preservation owner set",
                )
                .with_member(member_id, &row.path));
            }
            ParticipantState::Aborted | ParticipantState::RolledBack => {
                return Err(ModelError::new(
                    ErrorCode::PreservationEvidenceMismatch,
                    "rollback-terminal participant cannot enter preservation",
                )
                .with_member(member_id, &row.path));
            }
        }
    }
    Ok(())
}

fn attach_member(mut error: ModelError, member_id: &str, member_path: &str) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(member_id.into());
        error.member_path = Some(member_path.into());
    }
    error
}

pub(super) fn observe_preserve_participant<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
    member_id: &str,
) -> ModelResult<ExactObservationFact> {
    let fact = observe_forward(backend, context, current, request)?.into_fact();
    match fact {
        ExactObservationFact::NotStarted(NotStartedObservation::Participant {
            member_id: observed,
            ..
        }) if observed == member_id => {
            let proof = VerifiedParticipantNotStarted::issue(
                &AuthorityIssuer::for_observer(current),
                member_id,
                "participant_not_started",
                "verified",
                member_id.into(),
            )?;
            let entry = prepare_participant_entry(
                backend,
                context,
                current,
                ReverseEntryPredecessor::ParticipantNotStarted(&proof),
            )?;
            Ok(ExactObservationFact::Abandon(
                Box::new(proof),
                EntryFact::Preservation(Box::new(entry)),
            ))
        }
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::Outcome(proof, EntryFact::None),
        )) if proof.value().member_id == member_id
            && current.record().state == OperationState::Halted =>
        {
            let entry = prepare_participant_entry(
                backend,
                context,
                current,
                ReverseEntryPredecessor::ParticipantOutcome(&proof),
            )?;
            Ok(completed(CompletedObservation::Participant(
                ParticipantObservation::Outcome(proof, EntryFact::Preservation(Box::new(entry))),
            )))
        }
        other => Ok(other),
    }
}

fn prepare_participant_entry<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    predecessor: ReverseEntryPredecessor<'_>,
) -> ModelResult<PreparedPreservationEntry> {
    let preview = preview_reverse_entry(current, V1LifecycleRequest::Preserve, predecessor)?;
    let RecordEvidenceOr::Ready(handoff) =
        observe_reverse_publication_handoff(backend, context, current, &preview)?
    else {
        return Err(preservation_error(
            "participant preservation entry must first record publication evidence",
        ));
    };
    let preflight = preflight_entry(backend, current, &preview, handoff.value())?;
    prepare_preservation_entry(current, &preview, handoff, preflight)
}
