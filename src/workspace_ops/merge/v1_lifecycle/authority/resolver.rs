use serde::Serialize;

use super::super::super::model::v1::{
    MergeOperationRecordV1, PendingPreservationActionV1, PendingRollbackActionV1,
    PreservationOwnerV1,
};
use super::super::super::{
    MergeParticipantRecord, MergeRecordError, OperationState, ParticipantState, PendingMergeAction,
    PendingMergeActionKind, PublicationStep,
};
use super::super::checked::StoredV1Record;
use super::super::transition::*;
use super::binding::{AuthorityIssuer, BoundValue};
use super::dispatcher::*;
use super::{
    BoundAmbiguityEvidence, BoundAuthority, BoundOwnedResolutionFailureHaltBatch,
    BoundOwnedRetryFailureHaltBatch, BoundPublicationDecision, ParticipantActionPayload,
    ParticipantFailurePayload, PreparedAcceptedWorkspace, PreparedBackupRefIntent,
    PreparedCandidate, PreparedEvidenceIntent, PreparedEvidenceRollback, PreparedFailureHaltBatch,
    PreparedParticipantAction, PreparedParticipantRollback, PreparedPreservationEntry,
    PreparedPublicationIntent, PreparedRefResetIntent, PreparedRollbackEntry,
    PreparedRootMetadataRollback, PreparedStashIntent, PreservationCursorPosition,
    VerifiedBackupRef, VerifiedCandidatePublicationCompletion, VerifiedEvidenceResult,
    VerifiedEvidenceRollbackCompletion, VerifiedEvidenceRollbackStep, VerifiedNoMutationAbort,
    VerifiedParticipantNotStarted, VerifiedParticipantOutcome, VerifiedParticipantRollback,
    VerifiedParticipants, VerifiedPreservationCursorPrefix, VerifiedPublicationAction,
    VerifiedPublicationCompletion, VerifiedRecoveryOrigin, VerifiedRefResetCompletion,
    VerifiedRefResetPhase, VerifiedResults, VerifiedRollbackExhausted,
    VerifiedRootMetadataRollbackCompletion, VerifiedRootMetadataRollbackStep,
    VerifiedStashCompletion, VerifiedStashPhase,
};
use crate::model::{ModelError, ModelResult};

mod execution;
mod observation;

use execution::*;
pub(in crate::workspace_ops::merge::v1_lifecycle) use observation::*;

type B<T> = Box<T>;

pub(in crate::workspace_ops::merge::v1_lifecycle) enum ResolvedV1Action {
    Apply(V1Transition),
    Execute(Box<BoundPhysicalAction>),
    Respond(V1ResponseDisposition),
    Reject(ModelError),
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn resolve_observation(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    observation_request: BoundObservationRequest,
    observation: BoundExactObservation,
    attempt: Option<BoundExecutionAttempt>,
) -> ModelResult<ResolvedV1Action> {
    if !observation_request.matches(current, request) {
        return Err(dispatch_error("observation request binding is stale"));
    }
    if !observation.matches(current, &observation_request) {
        return Err(dispatch_error("exact observation binding is stale"));
    }
    if let Some(value) = attempt.as_ref() {
        let continuing_same_owner = matches!(observation.fact, ExactObservationFact::NotStarted(_));
        if !continuing_same_owner && observation.physical().is_none() {
            return Err(dispatch_error(
                "post-attempt observation has no physical action binding",
            ));
        }
        if !value.matches(
            current,
            &observation_request.0.value,
            (!continuing_same_owner)
                .then(|| observation.physical())
                .flatten(),
        ) {
            return Err(dispatch_error(
                "execution attempt does not match the fresh observation",
            ));
        }
    }
    match observation.fact {
        ExactObservationFact::Completed(fact) => {
            completed(current, request, observation_request.kind(), fact)
        }
        ExactObservationFact::PreservationDurabilityPending {
            completion,
            prefix,
            action,
        } => durability_pending(
            current,
            request,
            observation_request,
            completion,
            prefix,
            action,
            attempt,
        ),
        ExactObservationFact::Ambiguous(proof) => {
            require(
                !matches!(
                    observation_request.kind(),
                    ObservationKind::PreservationCursor
                ) || current.record().pending_preservation.is_none(),
            )?;
            ambiguous(current, proof)
        }
        ExactObservationFact::PreservationAmbiguous(proof, prefix) => {
            require(matches!(
                observation_request.kind(),
                ObservationKind::PreservationCursor
            ))?;
            let action = current
                .record()
                .pending_preservation
                .as_ref()
                .ok_or_else(rejected)?;
            require(preservation_prefix_authorizes(current, action, &prefix))?;
            require(physical_matches(
                current,
                observation_request.kind(),
                &PhysicalActionKind::Preservation(action.clone()),
            ))?;
            if let Some(result) = preservation_attempt_without_pending_goal(attempt.as_ref()) {
                return Ok(result);
            }
            ambiguous(current, proof)
        }
        ExactObservationFact::NotStarted(observed) => {
            not_started(current, request, observation_request, observed, attempt)
        }
        ExactObservationFact::Abandon(proof, entry) => {
            abandon(request, observation_request.kind(), proof, entry, attempt)
        }
    }
}

fn durability_pending(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    observation: BoundObservationRequest,
    completion: PreservationObservation,
    prefix: VerifiedPreservationCursorPrefix,
    action: PendingPreservationActionV1,
    attempt: Option<BoundExecutionAttempt>,
) -> ModelResult<ResolvedV1Action> {
    require(matches!(
        observation.kind(),
        ObservationKind::PreservationCursor
    ))?;
    require(preservation_prefix_authorizes(current, &action, &prefix))?;
    require(physical_matches(
        current,
        observation.kind(),
        &PhysicalActionKind::Preservation(action.clone()),
    ))?;
    match attempt {
        None => not_started(
            current,
            request,
            observation,
            NotStartedObservation::Preservation { action, prefix },
            None,
        ),
        Some(value) if attempt_succeeded(&value) => completed(
            current,
            request,
            observation.kind(),
            CompletedObservation::Preservation(completion),
        ),
        Some(value) => Ok(ResolvedV1Action::Reject(
            attempt_failure(&value).ok_or_else(rejected)?,
        )),
    }
}

fn preservation_attempt_without_pending_goal(
    attempt: Option<&BoundExecutionAttempt>,
) -> Option<ResolvedV1Action> {
    let attempt = attempt?;
    Some(match attempt_failure(attempt) {
        Some(error) => ResolvedV1Action::Reject(error),
        None => ResolvedV1Action::Reject(dispatch_error(
            "preservation success lacks a fresh durability-pending goal",
        )),
    })
}

fn not_started(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    observation: BoundObservationRequest,
    observed: NotStartedObservation,
    attempt: Option<BoundExecutionAttempt>,
) -> ModelResult<ResolvedV1Action> {
    let action = resolve_physical(current, observation.kind(), observed)?;
    if matches!(
        request,
        V1LifecycleRequest::Abort | V1LifecycleRequest::Preserve
    ) && matches!(
        observation.kind(),
        ObservationKind::ParticipantAction { .. }
    ) {
        return reject("abort or preserve requires a bound participant abandonment entry");
    }
    if let Some(value) = attempt
        && value.action() == &action
    {
        return no_progress(current, value);
    }
    let key = observation.0.value;
    let owner = key.owner.clone();
    Ok(ResolvedV1Action::Execute(Box::new(BoundPhysicalAction(
        BoundValue::new(
            current,
            &owner,
            "execute",
            "authorized",
            PhysicalActionKey {
                observation: key,
                action,
            },
        )?,
    ))))
}

fn resolve_physical(
    current: &StoredV1Record,
    kind: &ObservationKind,
    observed: NotStartedObservation,
) -> ModelResult<PhysicalActionKind> {
    let action = match observed {
        NotStartedObservation::Participant { member_id, action } => {
            PhysicalActionKind::Participant { member_id, action }
        }
        NotStartedObservation::Publication(proof) => {
            let action = *proof.value();
            require(proof.matches(
                current,
                "@publication",
                "publication_action",
                publication_action_phase(action),
            ))?;
            PhysicalActionKind::Publication(action)
        }
        NotStartedObservation::Preservation { action, prefix } => {
            require(preservation_prefix_authorizes(current, &action, &prefix))?;
            PhysicalActionKind::Preservation(action)
        }
        NotStartedObservation::Rollback(action) => PhysicalActionKind::Rollback(action),
        NotStartedObservation::Archive => PhysicalActionKind::Archive,
    };
    require(physical_matches(current, kind, &action))?;
    Ok(action)
}

fn publication_action_phase(action: PublicationPhysicalAction) -> &'static str {
    match action {
        PublicationPhysicalAction::EvidenceCommit => "evidence_commit",
        PublicationPhysicalAction::WriteMarker => "write_marker",
        PublicationPhysicalAction::WriteLock => "write_lock",
        PublicationPhysicalAction::WriteBoundary => "write_boundary",
        PublicationPhysicalAction::StageIndex => "stage_index",
    }
}

fn preservation_prefix_authorizes(
    current: &StoredV1Record,
    action: &PendingPreservationActionV1,
    prefix: &VerifiedPreservationCursorPrefix,
) -> bool {
    let (owner, position) = match action {
        PendingPreservationActionV1::BackupRef { owner, .. } => {
            (owner.clone(), PreservationCursorPosition::BackupRef)
        }
        PendingPreservationActionV1::Stash { owner, phase, .. } => {
            (owner.clone(), PreservationCursorPosition::Stash(*phase))
        }
        PendingPreservationActionV1::ResetAttachedRef { owner, phase, .. } => (
            owner.clone(),
            PreservationCursorPosition::ResetAttachedRef(*phase),
        ),
    };
    let owner_id = match &owner {
        PreservationOwnerV1::Participant { member_id } => member_id.as_str(),
        PreservationOwnerV1::PublicationRoot => "@publication-root",
    };
    prefix.value().owner == owner
        && prefix.value().position == position
        && prefix.matches(current, owner_id, "preservation_cursor", "prefix_verified")
}

fn abandon(
    request: V1LifecycleRequest,
    kind: &ObservationKind,
    proof: B<VerifiedParticipantNotStarted>,
    entry: EntryFact,
    attempt: Option<BoundExecutionAttempt>,
) -> ModelResult<ResolvedV1Action> {
    require(attempt.is_none() && matches!(kind, ObservationKind::ParticipantAction { .. }))?;
    match (request, entry) {
        (V1LifecycleRequest::Abort, EntryFact::Rollback(value)) => part(
            ParticipantTransition::AbandonNotStartedAndBeginRollback(proof, value),
        ),
        (V1LifecycleRequest::Preserve, EntryFact::Preservation(value)) => {
            part(ParticipantTransition::AbandonNotStartedAndBeginPreservation(proof, value))
        }
        _ => reject("abandonment entry does not match the bound request"),
    }
}

fn completed(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    kind: &ObservationKind,
    fact: CompletedObservation,
) -> ModelResult<ResolvedV1Action> {
    use CompletedObservation as C;
    use ObservationKind as K;
    match (kind, fact) {
        (K::ParticipantPreparation { member_id }, C::Participant(value)) => {
            prepared(current, member_id, value)
        }
        (K::ParticipantAction { member_id }, C::Participant(value)) => {
            outcome(current, request, member_id, value)
        }
        (K::ParticipantsComplete, C::Participants(value)) => {
            op(OperationTransition::EnterFinalizing(value))
        }
        (K::Acceptance, C::Acceptance(value)) => tr(V1Transition::Acceptance(B::new(
            AcceptanceTransition::Freeze(value),
        ))),
        (K::Publication, C::Publication(value)) => publication(value),
        (K::PreservationEntry, C::Publication(PublicationObservation::EvidenceResult(value)))
        | (K::RollbackEntry, C::Publication(PublicationObservation::EvidenceResult(value))) => {
            pubn(PublicationTransition::RecordEvidence(value))
        }
        (K::PreservationEntry, C::PreservationEntry(value))
            if request == V1LifecycleRequest::Preserve =>
        {
            op(OperationTransition::BeginPreservation(value))
        }
        (K::RollbackEntry, C::RollbackEntry(value)) if request == V1LifecycleRequest::Abort => {
            op(OperationTransition::BeginRollback(value))
        }
        (K::PreservationCursor, C::Preservation(value)) => preservation(request, value),
        (K::RollbackCursor, C::Rollback(value)) => rollback(value),
        (K::Recovery, C::Recovery(value)) => tr(V1Transition::Recovery(B::new(
            RecoveryTransition::Resume(value),
        ))),
        (K::Archive, C::Archive) if request == V1LifecycleRequest::Archive => Ok(
            ResolvedV1Action::Respond(V1ResponseDisposition::ArchiveReady),
        ),
        _ => reject("completed fact does not match the bound request and observation"),
    }
}

fn prepared(
    current: &StoredV1Record,
    member_id: &str,
    value: ParticipantObservation,
) -> ModelResult<ResolvedV1Action> {
    match value {
        ParticipantObservation::Prepared(value)
            if current.record().state == OperationState::AwaitingResolution
                && value.value().member_id == member_id
                && value.value().row.state == ParticipantState::Conflicted =>
        {
            op(OperationTransition::BeginExecution)
        }
        ParticipantObservation::Prepared(value) if value.value().member_id == member_id => {
            part(ParticipantTransition::Prepare(value))
        }
        ParticipantObservation::PreparationFailed(value)
            if value.value().member_id == member_id =>
        {
            part(ParticipantTransition::RecordPreparationFailureAndHalt(
                value,
            ))
        }
        _ => reject("participant preparation fact does not match its member"),
    }
}

fn outcome(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    member_id: &str,
    value: ParticipantObservation,
) -> ModelResult<ResolvedV1Action> {
    let ParticipantObservation::Outcome(proof, entry) = value else {
        return reject("participant action requires an outcome fact");
    };
    require(proof.value().member_id == member_id)?;
    match (current.record().state, request, entry) {
        (
            OperationState::Halted,
            V1LifecycleRequest::ResumeStart | V1LifecycleRequest::Continue,
            EntryFact::None,
        ) if !outcome_retains_halt(current.record(), proof.value()) => {
            part(ParticipantTransition::RecordHaltedOutcomeAndResumeExecution(proof))
        }
        (OperationState::Halted, V1LifecycleRequest::Abort, EntryFact::Rollback(value)) => part(
            ParticipantTransition::RecordHaltedOutcomeAndBeginRollback(proof, value),
        ),
        (OperationState::Halted, V1LifecycleRequest::Preserve, EntryFact::Preservation(value)) => {
            part(ParticipantTransition::RecordHaltedOutcomeAndBeginPreservation(proof, value))
        }
        (_, _, EntryFact::None) => part(ParticipantTransition::RecordOutcome(proof)),
        _ => reject("participant outcome entry does not match the bound request"),
    }
}

fn publication(value: PublicationObservation) -> ModelResult<ResolvedV1Action> {
    use PublicationObservation as P;
    let value = match value {
        P::Decision(value) if *value.value() => PublicationTransition::ClassifyRequired(value),
        P::Decision(value) => PublicationTransition::ClassifyNone(value),
        P::MigratedValidationReady => PublicationTransition::BeginMigratedValidation,
        P::MigratedResults(value) if *value.value() => {
            PublicationTransition::ClassifyMigratedRequired(value)
        }
        P::MigratedResults(value) => PublicationTransition::ClassifyMigratedNone(value),
        P::Candidate(value) => PublicationTransition::RecordCandidate(value),
        P::EvidenceIntent(value) => PublicationTransition::BeginEvidence(value),
        P::EvidenceResult(value) => PublicationTransition::RecordEvidence(value),
        P::PublicationIntent(value) => PublicationTransition::BeginCandidatePublication(value),
        P::CandidatePublished(value) => PublicationTransition::RecordCandidatePublished(value),
        P::PublicationVerified(value) => PublicationTransition::RecordPublicationVerified(value),
        P::OperationComplete(value) => return op(OperationTransition::CompleteOperation(value)),
    };
    pubn(value)
}

fn outcome_retains_halt(
    record: &MergeOperationRecordV1,
    outcome: &ParticipantActionPayload,
) -> bool {
    record.participants.iter().any(|(member_id, row)| {
        let row = if member_id == &outcome.member_id {
            &outcome.row
        } else {
            row
        };
        row.state == ParticipantState::Failed
            || row.state == ParticipantState::Conflicted && row.error.is_some()
    })
}

fn preservation(
    _request: V1LifecycleRequest,
    value: PreservationObservation,
) -> ModelResult<ResolvedV1Action> {
    use PreservationObservation as P;
    let value = match value {
        P::BackupIntent(value) => PreservationTransition::BeginBackupRef(value),
        P::BackupDone(value) => PreservationTransition::FinishBackupRef(value),
        P::StashIntent(value) => PreservationTransition::BeginStash(value),
        P::StashPhase(value) => PreservationTransition::AdvanceStash(value),
        P::StashDone(value) => PreservationTransition::FinishStash(value),
        P::ResetIntent(value) => PreservationTransition::BeginResetAttachedRef(value),
        P::ResetPhase(value) => PreservationTransition::AdvanceResetAttachedRef(value),
        P::ResetDone(value) => PreservationTransition::FinishResetAttachedRef(value),
        P::Exhausted(value) => return op(OperationTransition::BeginRollback(value)),
    };
    pres(value)
}

fn rollback(value: RollbackObservation) -> ModelResult<ResolvedV1Action> {
    use RollbackObservation as R;
    let value = match value {
        R::ParticipantIntent(value) => RollbackTransition::BeginParticipant(value),
        R::ParticipantDone(value) => RollbackTransition::FinishParticipant(value),
        R::EvidenceIntent(value) => RollbackTransition::BeginEvidence(value),
        R::EvidenceStep(value) => RollbackTransition::AdvanceEvidence(value),
        R::EvidenceDone(value) => RollbackTransition::FinishEvidence(value),
        R::RootIntent(value) => RollbackTransition::BeginSelectedRoot(value),
        R::RootStep(value) => RollbackTransition::AdvanceSelectedRoot(value),
        R::RootDone(value) => RollbackTransition::FinishSelectedRoot(value),
        R::NoMutation(value) => return part(ParticipantTransition::RecordNoMutationAbort(value)),
        R::Exhausted(value) => return op(OperationTransition::AbortOperation(value)),
    };
    roll(value)
}

fn ambiguous(
    current: &StoredV1Record,
    proof: BoundAmbiguityEvidence,
) -> ModelResult<ResolvedV1Action> {
    require(proof.matches(current, "@operation", "enter_recovery", "ambiguous"))?;
    if current.record().state == OperationState::Executing && has_halt_cause(current.record()) {
        return op(OperationTransition::Halt);
    }
    let owned = match current.record().state {
        OperationState::Preserving => current.record().pending_preservation.is_some(),
        OperationState::RollingBack => current.record().pending_rollback.is_some(),
        OperationState::Executing
        | OperationState::AwaitingResolution
        | OperationState::Halted
        | OperationState::Finalizing => true,
        _ => false,
    };
    if !owned {
        return reject("ambiguity has no representable recovery origin and persisted owner");
    }
    tr(V1Transition::Recovery(B::new(RecoveryTransition::Enter(
        proof,
    ))))
}

fn tr(value: V1Transition) -> ModelResult<ResolvedV1Action> {
    Ok(ResolvedV1Action::Apply(value))
}
fn op(value: OperationTransition) -> ModelResult<ResolvedV1Action> {
    tr(V1Transition::Operation(B::new(value)))
}
fn part(value: ParticipantTransition) -> ModelResult<ResolvedV1Action> {
    tr(V1Transition::Participant(B::new(value)))
}
fn pubn(value: PublicationTransition) -> ModelResult<ResolvedV1Action> {
    tr(V1Transition::Publication(B::new(value)))
}
fn pres(value: PreservationTransition) -> ModelResult<ResolvedV1Action> {
    tr(V1Transition::Preservation(B::new(value)))
}
fn roll(value: RollbackTransition) -> ModelResult<ResolvedV1Action> {
    tr(V1Transition::Rollback(B::new(value)))
}
fn reject(detail: &str) -> ModelResult<ResolvedV1Action> {
    Ok(ResolvedV1Action::Reject(dispatch_error(detail)))
}
fn require(condition: bool) -> ModelResult<()> {
    condition.then_some(()).ok_or_else(rejected)
}
fn rejected() -> ModelError {
    dispatch_error("observation authority does not match the checked record or request")
}
