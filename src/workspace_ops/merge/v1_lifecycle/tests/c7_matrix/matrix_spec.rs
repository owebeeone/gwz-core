use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1 as E, ParticipantRollbackKindV1 as P, PreservationRefResetPhaseV1 as R,
    PreservationStashPhaseV1 as S, RootMetadataRollbackStepV1 as M,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    AttemptClass, FreshFactClass, TransitionClass, V1LifecycleRequest,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RootOwner {
    PublicationRoot,
    SelectedRoot,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum HandoffShape {
    NoCandidate,
    EvidencePending,
    BaselinePre,
    MarkerPre,
    LockPre,
    BoundaryPre,
    MarkerStagedDegenerate,
    BoundaryStaged,
}

impl HandoffShape {
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn has_candidate(self) -> bool {
        !matches!(self, Self::NoCandidate | Self::EvidencePending)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RootPhase {
    BackupRef,
    Stash(S),
    Reset(R),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RowClass {
    Physical,
    CausalParent,
    ProofOnly,
    ActionFree,
}

impl RowClass {
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn transition(
        self,
    ) -> Option<TransitionClass> {
        match self {
            Self::Physical => Some(TransitionClass::Physical),
            Self::CausalParent => Some(TransitionClass::CausalParent),
            Self::ProofOnly | Self::ActionFree => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) struct RootRow {
    pub(in crate::workspace_ops::merge::v1_lifecycle) phase: RootPhase,
    pub(in crate::workspace_ops::merge::v1_lifecycle) class: RowClass,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const REQUESTS: [V1LifecycleRequest; 5] = [
    V1LifecycleRequest::ResumeStart,
    V1LifecycleRequest::Continue,
    V1LifecycleRequest::Abort,
    V1LifecycleRequest::Preserve,
    V1LifecycleRequest::Archive,
];

pub(in crate::workspace_ops::merge::v1_lifecycle) const OWNERS: [RootOwner; 2] =
    [RootOwner::PublicationRoot, RootOwner::SelectedRoot];

pub(in crate::workspace_ops::merge::v1_lifecycle) const HANDOFFS: [HandoffShape; 8] = [
    HandoffShape::NoCandidate,
    HandoffShape::EvidencePending,
    HandoffShape::BaselinePre,
    HandoffShape::MarkerPre,
    HandoffShape::LockPre,
    HandoffShape::BoundaryPre,
    HandoffShape::MarkerStagedDegenerate,
    HandoffShape::BoundaryStaged,
];

const PUBLICATION_ROOT_HANDOFFS: [HandoffShape; 6] = [
    HandoffShape::BaselinePre,
    HandoffShape::MarkerPre,
    HandoffShape::LockPre,
    HandoffShape::BoundaryPre,
    HandoffShape::MarkerStagedDegenerate,
    HandoffShape::BoundaryStaged,
];

const SELECTED_ROOT_HANDOFFS: [HandoffShape; 7] = [
    HandoffShape::NoCandidate,
    HandoffShape::BaselinePre,
    HandoffShape::MarkerPre,
    HandoffShape::LockPre,
    HandoffShape::BoundaryPre,
    HandoffShape::MarkerStagedDegenerate,
    HandoffShape::BoundaryStaged,
];

pub(in crate::workspace_ops::merge::v1_lifecycle) fn legal_handoffs(
    owner: RootOwner,
) -> &'static [HandoffShape] {
    match owner {
        // A publication-root preservation owner exists only after a candidate
        // composition commit exists, so an absent handoff cannot own it.
        RootOwner::PublicationRoot => &PUBLICATION_ROOT_HANDOFFS,
        // EvidencePending requires the complete root evidence base to remain
        // live. A selected root that itself needs preservation cannot satisfy
        // that prerequisite, so only non-root owners use that handoff.
        RootOwner::SelectedRoot => &SELECTED_ROOT_HANDOFFS,
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const ATTEMPTS: [AttemptClass; 5] = [
    AttemptClass::None,
    AttemptClass::MatchingSuccess,
    AttemptClass::MatchingFailed,
    AttemptClass::StaleOrMismatched,
    AttemptClass::ConsumedSecond,
];

pub(in crate::workspace_ops::merge::v1_lifecycle) const PHYSICAL_FACTS: [FreshFactClass; 4] = [
    FreshFactClass::Before,
    FreshFactClass::After,
    FreshFactClass::Ambiguous,
    FreshFactClass::OperationalError,
];

pub(in crate::workspace_ops::merge::v1_lifecycle) const CAUSAL_FACTS: [FreshFactClass; 4] = [
    FreshFactClass::Before,
    FreshFactClass::AfterNeedsDurability,
    FreshFactClass::Ambiguous,
    FreshFactClass::OperationalError,
];

pub(in crate::workspace_ops::merge::v1_lifecycle) const CAUSAL_RESTORE_PARENT_VARIANTS:
    [RootPhase; 2] = [
    RootPhase::Stash(S::RestoreParent),
    RootPhase::Reset(R::RestoreParent),
];

pub(in crate::workspace_ops::merge::v1_lifecycle) fn root_rows(
    handoff: HandoffShape,
) -> Vec<RootRow> {
    if handoff == HandoffShape::NoCandidate {
        return vec![
            physical(RootPhase::BackupRef),
            physical(RootPhase::Stash(S::CreateStash)),
            physical(RootPhase::Stash(S::WriteBundle)),
            action_free(RootPhase::Stash(S::Complete)),
            physical(RootPhase::Reset(R::ResetRef)),
            action_free(RootPhase::Reset(R::Complete)),
        ];
    }
    if handoff == HandoffShape::EvidencePending {
        return vec![
            physical(RootPhase::Stash(S::CreateStash)),
            physical(RootPhase::Stash(S::WriteBundle)),
            action_free(RootPhase::Stash(S::Complete)),
        ];
    }
    let physical_phases = canonical_physical_root_phases(handoff);
    candidate_phase_vocabulary()
        .into_iter()
        .map(|phase| {
            let class = if matches!(
                phase,
                RootPhase::Stash(S::Complete) | RootPhase::Reset(R::Complete)
            ) {
                RowClass::ActionFree
            } else if physical_phases.contains(&phase) {
                if matches!(
                    phase,
                    RootPhase::Stash(S::NormalizeParent)
                        | RootPhase::Stash(S::RestoreParent)
                        | RootPhase::Reset(R::PrepareParent)
                        | RootPhase::Reset(R::RestoreParent)
                ) {
                    RowClass::CausalParent
                } else {
                    RowClass::Physical
                }
            } else {
                RowClass::ProofOnly
            };
            RootRow { phase, class }
        })
        .collect()
}

/// Exact physical/causal rows for the canonical root matrix's clean form.
/// Omitted phases are proof-only because the handoff already equals the
/// phase goal. Parent restore remains separately covered by the required-empty
/// causal variant, whose marker-presence relation is independent of this
/// canonical clean-form fixture.
pub(in crate::workspace_ops::merge::v1_lifecycle) fn canonical_physical_root_phases(
    handoff: HandoffShape,
) -> Vec<RootPhase> {
    use HandoffShape as H;
    let common_stash = [
        RootPhase::BackupRef,
        RootPhase::Stash(S::CreateStash),
        RootPhase::Stash(S::WriteBundle),
    ];
    let common_reset = [RootPhase::Reset(R::ResetRef)];
    let mut phases = common_stash.to_vec();
    let stash_phases: &[RootPhase] = match handoff {
        H::BaselinePre => &[
            RootPhase::Stash(S::NormalizeParent),
            RootPhase::Stash(S::NormalizeMarker),
            RootPhase::Stash(S::NormalizeLock),
            RootPhase::Stash(S::NormalizeIndex),
            RootPhase::Stash(S::RestoreIndex),
            RootPhase::Stash(S::RestoreLock),
            RootPhase::Stash(S::RestoreMarker),
        ],
        H::MarkerPre => &[
            RootPhase::Stash(S::NormalizeLock),
            RootPhase::Stash(S::NormalizeIndex),
            RootPhase::Stash(S::RestoreIndex),
            RootPhase::Stash(S::RestoreLock),
        ],
        H::LockPre | H::BoundaryPre => &[
            RootPhase::Stash(S::NormalizeIndex),
            RootPhase::Stash(S::RestoreIndex),
        ],
        H::MarkerStagedDegenerate | H::BoundaryStaged => &[],
        H::NoCandidate => {
            return vec![
                RootPhase::BackupRef,
                RootPhase::Stash(S::CreateStash),
                RootPhase::Stash(S::WriteBundle),
                RootPhase::Reset(R::ResetRef),
            ];
        }
        H::EvidencePending => {
            return vec![
                RootPhase::Stash(S::CreateStash),
                RootPhase::Stash(S::WriteBundle),
            ];
        }
    };
    phases.splice(1..1, stash_phases.iter().copied());
    match handoff {
        H::BaselinePre => phases.extend([
            RootPhase::Reset(R::PrepareParent),
            RootPhase::Reset(R::PrepareMarker),
            RootPhase::Reset(R::PrepareLock),
            RootPhase::Reset(R::PrepareIndex),
            common_reset[0],
            RootPhase::Reset(R::RestoreIndex),
            RootPhase::Reset(R::RestoreLock),
            RootPhase::Reset(R::RestoreMarker),
        ]),
        H::MarkerPre => phases.extend([
            RootPhase::Reset(R::PrepareLock),
            RootPhase::Reset(R::PrepareIndex),
            common_reset[0],
            RootPhase::Reset(R::RestoreIndex),
            RootPhase::Reset(R::RestoreLock),
        ]),
        H::LockPre | H::BoundaryPre => phases.extend([
            RootPhase::Reset(R::PrepareIndex),
            common_reset[0],
            RootPhase::Reset(R::RestoreIndex),
        ]),
        H::MarkerStagedDegenerate | H::BoundaryStaged => phases.extend(common_reset),
        H::NoCandidate | H::EvidencePending => unreachable!(),
    }
    phases
}

fn candidate_phase_vocabulary() -> [RootPhase; 22] {
    [
        RootPhase::BackupRef,
        RootPhase::Stash(S::NormalizeParent),
        RootPhase::Stash(S::NormalizeMarker),
        RootPhase::Stash(S::NormalizeLock),
        RootPhase::Stash(S::NormalizeIndex),
        RootPhase::Stash(S::CreateStash),
        RootPhase::Stash(S::RestoreIndex),
        RootPhase::Stash(S::RestoreLock),
        RootPhase::Stash(S::RestoreParent),
        RootPhase::Stash(S::RestoreMarker),
        RootPhase::Stash(S::WriteBundle),
        RootPhase::Stash(S::Complete),
        RootPhase::Reset(R::PrepareParent),
        RootPhase::Reset(R::PrepareMarker),
        RootPhase::Reset(R::PrepareLock),
        RootPhase::Reset(R::PrepareIndex),
        RootPhase::Reset(R::ResetRef),
        RootPhase::Reset(R::RestoreIndex),
        RootPhase::Reset(R::RestoreLock),
        RootPhase::Reset(R::RestoreParent),
        RootPhase::Reset(R::RestoreMarker),
        RootPhase::Reset(R::Complete),
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RollbackRow {
    Participant(P),
    RecordNoMutationAbort,
    Evidence(E),
    SelectedRoot(M),
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const ROLLBACK_ROWS: [(RollbackRow, RowClass);
    12] = [
    (
        RollbackRow::Participant(P::AbortConflict),
        RowClass::Physical,
    ),
    (
        RollbackRow::Participant(P::ResetIntegrated),
        RowClass::Physical,
    ),
    (RollbackRow::RecordNoMutationAbort, RowClass::ProofOnly),
    (RollbackRow::Evidence(E::EvidenceCommit), RowClass::Physical),
    (RollbackRow::Evidence(E::Boundary), RowClass::Physical),
    (RollbackRow::Evidence(E::Lock), RowClass::Physical),
    (RollbackRow::Evidence(E::Marker), RowClass::Physical),
    (RollbackRow::Evidence(E::Index), RowClass::Physical),
    (RollbackRow::Evidence(E::Complete), RowClass::ActionFree),
    (RollbackRow::SelectedRoot(M::Manifest), RowClass::Physical),
    (RollbackRow::SelectedRoot(M::Lock), RowClass::Physical),
    (RollbackRow::SelectedRoot(M::Complete), RowClass::ActionFree),
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum NonRootPreservationRow {
    BackupRef,
    CreateStash,
    WriteBundle,
    StashComplete,
    ResetRef,
    ResetComplete,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const NON_ROOT_ROWS: [(
    NonRootPreservationRow,
    RowClass,
); 6] = [
    (NonRootPreservationRow::BackupRef, RowClass::Physical),
    (NonRootPreservationRow::CreateStash, RowClass::Physical),
    (NonRootPreservationRow::WriteBundle, RowClass::Physical),
    (NonRootPreservationRow::StashComplete, RowClass::ActionFree),
    (NonRootPreservationRow::ResetRef, RowClass::Physical),
    (NonRootPreservationRow::ResetComplete, RowClass::ActionFree),
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ActionFreePosition {
    BeginPreservation,
    BeginRollback,
    FinishPreservationOwner,
    FinishRollbackOwner,
    PreservationCursorComplete,
    RollbackCursorComplete,
    RecoveryResume,
    PreservationExhaustion,
    RollbackExhaustion,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const ACTION_FREE_POSITIONS:
    [ActionFreePosition; 9] = [
    ActionFreePosition::BeginPreservation,
    ActionFreePosition::BeginRollback,
    ActionFreePosition::FinishPreservationOwner,
    ActionFreePosition::FinishRollbackOwner,
    ActionFreePosition::PreservationCursorComplete,
    ActionFreePosition::RollbackCursorComplete,
    ActionFreePosition::RecoveryResume,
    ActionFreePosition::PreservationExhaustion,
    ActionFreePosition::RollbackExhaustion,
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum CheckedArtifactRow {
    SourceEqualsGoal,
    AuthorityPublication,
    GoalPublication,
    SourceDetach,
    ManagedPublication,
    SourceCleanup,
    AuthorityCleanup,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) const CHECKED_ARTIFACT_ROWS: [(
    CheckedArtifactRow,
    RowClass,
); 7] = [
    (CheckedArtifactRow::SourceEqualsGoal, RowClass::ProofOnly),
    (CheckedArtifactRow::AuthorityPublication, RowClass::Physical),
    (CheckedArtifactRow::GoalPublication, RowClass::Physical),
    (CheckedArtifactRow::SourceDetach, RowClass::Physical),
    (CheckedArtifactRow::ManagedPublication, RowClass::Physical),
    (CheckedArtifactRow::SourceCleanup, RowClass::Physical),
    (CheckedArtifactRow::AuthorityCleanup, RowClass::Physical),
];

const fn physical(phase: RootPhase) -> RootRow {
    RootRow {
        phase,
        class: RowClass::Physical,
    }
}

const fn action_free(phase: RootPhase) -> RootRow {
    RootRow {
        phase,
        class: RowClass::ActionFree,
    }
}
