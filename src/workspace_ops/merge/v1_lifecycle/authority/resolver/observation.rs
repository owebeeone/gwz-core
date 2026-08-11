use super::*;

pub(in crate::workspace_ops::merge::v1_lifecycle) enum EntryFact {
    None,
    Rollback(B<PreparedRollbackEntry>),
    Preservation(B<PreparedPreservationEntry>),
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ParticipantObservation {
    Prepared(B<PreparedParticipantAction>),
    PreparationFailed(B<PreparedFailureHaltBatch>),
    Outcome(B<VerifiedParticipantOutcome>, EntryFact),
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum PublicationObservation {
    Decision(BoundPublicationDecision),
    MigratedValidationReady,
    MigratedResults(VerifiedResults),
    Candidate(B<PreparedCandidate>),
    EvidenceIntent(PreparedEvidenceIntent),
    EvidenceResult(B<VerifiedEvidenceResult>),
    PublicationIntent(PreparedPublicationIntent),
    CandidatePublished(VerifiedCandidatePublicationCompletion),
    PublicationVerified(VerifiedPublicationCompletion),
    OperationComplete(VerifiedPublicationCompletion),
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum PreservationObservation {
    BackupIntent(B<PreparedBackupRefIntent>),
    BackupDone(B<VerifiedBackupRef>),
    StashIntent(B<PreparedStashIntent>),
    StashPhase(B<VerifiedStashPhase>),
    StashDone(B<VerifiedStashCompletion>),
    ResetIntent(B<PreparedRefResetIntent>),
    ResetPhase(B<VerifiedRefResetPhase>),
    ResetDone(B<VerifiedRefResetCompletion>),
    Exhausted(B<PreparedRollbackEntry>),
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RollbackObservation {
    ParticipantIntent(B<PreparedParticipantRollback>),
    ParticipantDone(B<VerifiedParticipantRollback>),
    EvidenceIntent(B<PreparedEvidenceRollback>),
    EvidenceStep(B<VerifiedEvidenceRollbackStep>),
    EvidenceDone(VerifiedEvidenceRollbackCompletion),
    RootIntent(B<PreparedRootMetadataRollback>),
    RootStep(B<VerifiedRootMetadataRollbackStep>),
    RootDone(VerifiedRootMetadataRollbackCompletion),
    NoMutation(B<VerifiedNoMutationAbort>),
    Exhausted(VerifiedRollbackExhausted),
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum CompletedObservation {
    Participant(ParticipantObservation),
    Participants(VerifiedParticipants),
    Acceptance(B<PreparedAcceptedWorkspace>),
    Publication(PublicationObservation),
    PreservationEntry(B<PreparedPreservationEntry>),
    RollbackEntry(B<PreparedRollbackEntry>),
    Preservation(PreservationObservation),
    Rollback(RollbackObservation),
    Recovery(VerifiedRecoveryOrigin),
    Archive,
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum NotStartedObservation {
    Participant {
        member_id: String,
        action: Box<PendingMergeAction>,
    },
    Publication(VerifiedPublicationAction),
    Preservation {
        action: PendingPreservationActionV1,
        prefix: VerifiedPreservationCursorPrefix,
    },
    Rollback(PendingRollbackActionV1),
    Archive,
}
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ExactObservationFact {
    NotStarted(NotStartedObservation),
    Abandon(B<VerifiedParticipantNotStarted>, EntryFact),
    Completed(CompletedObservation),
    PreservationDurabilityPending {
        completion: PreservationObservation,
        prefix: VerifiedPreservationCursorPrefix,
        action: PendingPreservationActionV1,
    },
    Ambiguous(BoundAmbiguityEvidence),
    PreservationAmbiguous(BoundAmbiguityEvidence, VerifiedPreservationCursorPrefix),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct ExactObservationKey {
    pub(super) request: ObservationKey,
    pub(super) physical: Option<PhysicalActionKind>,
}
pub(in crate::workspace_ops::merge::v1_lifecycle) struct BoundExactObservation {
    pub(super) bound: BoundValue<ExactObservationKey>,
    pub(super) fact: ExactObservationFact,
}

impl BoundExactObservation {
    pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn into_fact(
        self,
    ) -> ExactObservationFact {
        self.fact
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn for_test(
        current: &StoredV1Record,
        request: &BoundObservationRequest,
        fact: ExactObservationFact,
    ) -> ModelResult<Self> {
        Self::issue(current, request, fact)
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn for_test_after_action(
        current: &StoredV1Record,
        request: &BoundObservationRequest,
        fact: ExactObservationFact,
        physical: PhysicalActionKind,
    ) -> ModelResult<Self> {
        Self::issue_with_physical(current, request, fact, Some(physical))
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn issue(
        current: &StoredV1Record,
        request: &BoundObservationRequest,
        fact: ExactObservationFact,
    ) -> ModelResult<Self> {
        let physical = observed_physical(current, request.kind(), &fact);
        Self::issue_with_physical(current, request, fact, physical)
    }

    fn issue_with_physical(
        current: &StoredV1Record,
        request: &BoundObservationRequest,
        fact: ExactObservationFact,
        physical: Option<PhysicalActionKind>,
    ) -> ModelResult<Self> {
        require(request.matches(current, request.0.value.request))?;
        let key = request.0.value.clone();
        let owner = key.owner.clone();
        Ok(Self {
            bound: BoundValue::new(
                current,
                &owner,
                "observe",
                "classified",
                ExactObservationKey {
                    request: key,
                    physical,
                },
            )?,
            fact,
        })
    }

    pub(super) fn matches(
        &self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> bool {
        self.bound.value.request == request.0.value
            && self.bound.matches(
                current,
                &self.bound.value.request.owner,
                "observe",
                "classified",
            )
    }

    pub(super) fn physical(&self) -> Option<&PhysicalActionKind> {
        self.bound.value.physical.as_ref()
    }
}

fn observed_physical(
    current: &StoredV1Record,
    kind: &ObservationKind,
    fact: &ExactObservationFact,
) -> Option<PhysicalActionKind> {
    match fact {
        ExactObservationFact::Completed(CompletedObservation::Publication(
            PublicationObservation::CandidatePublished(_),
        )) => Some(PhysicalActionKind::Publication(
            PublicationPhysicalAction::StageIndex,
        )),
        ExactObservationFact::NotStarted(value) => Some(match value {
            NotStartedObservation::Participant { member_id, action } => {
                PhysicalActionKind::Participant {
                    member_id: member_id.clone(),
                    action: action.clone(),
                }
            }
            NotStartedObservation::Publication(proof) => {
                PhysicalActionKind::Publication(*proof.value())
            }
            NotStartedObservation::Preservation { action, .. } => {
                PhysicalActionKind::Preservation(action.clone())
            }
            NotStartedObservation::Rollback(action) => PhysicalActionKind::Rollback(action.clone()),
            NotStartedObservation::Archive => PhysicalActionKind::Archive,
        }),
        ExactObservationFact::PreservationDurabilityPending { action, .. } => {
            Some(PhysicalActionKind::Preservation(action.clone()))
        }
        ExactObservationFact::Abandon(..)
        | ExactObservationFact::Completed(_)
        | ExactObservationFact::Ambiguous(_)
        | ExactObservationFact::PreservationAmbiguous(..) => persisted_physical(current, kind),
    }
}

fn persisted_physical(
    current: &StoredV1Record,
    kind: &ObservationKind,
) -> Option<PhysicalActionKind> {
    match kind {
        ObservationKind::ParticipantAction { member_id } => current
            .record()
            .participants
            .get(member_id)
            .and_then(|row| row.pending_action.clone())
            .map(|action| PhysicalActionKind::Participant {
                member_id: member_id.clone(),
                action: Box::new(action),
            }),
        ObservationKind::Publication
            if current
                .record()
                .publication
                .as_ref()
                .is_some_and(|progress| {
                    progress.step == PublicationStep::CommittingEvidence
                        && progress.composition_commit.is_none()
                }) =>
        {
            Some(PhysicalActionKind::Publication(
                PublicationPhysicalAction::EvidenceCommit,
            ))
        }
        ObservationKind::PreservationCursor => current
            .record()
            .pending_preservation
            .clone()
            .map(PhysicalActionKind::Preservation),
        ObservationKind::RollbackCursor => current
            .record()
            .pending_rollback
            .clone()
            .map(PhysicalActionKind::Rollback),
        ObservationKind::Archive => Some(PhysicalActionKind::Archive),
        _ => None,
    }
}
