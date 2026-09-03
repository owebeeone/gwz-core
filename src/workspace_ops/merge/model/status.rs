use std::collections::BTreeMap;

use super::{OperationDrift, ParticipantDrift, ParticipantDriftKind, PendingMergeActionKind};

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct RetryEligibility {
    pub eligible: bool,
    pub blockers: Vec<ParticipantDriftKind>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct RollbackEligibility {
    pub eligible: bool,
    pub blockers: Vec<ParticipantDriftKind>,
}

/// Read-only status projection of a durable action that has not yet been
/// paired with a durable participant outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingActionObservation {
    pub kind: PendingMergeActionKind,
    pub state: PendingActionObservationState,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingActionObservationState {
    NotStarted,
    ExpectedConflict,
    CompletedExactly,
    Ambiguous,
}

/// One read-only live observation for a recorded participant. Status computes
/// this without modifying the durable operation record; continue and abort
/// consume the same drift and eligibility classification after the M1 gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergeParticipantObservation {
    pub live_commit: Option<String>,
    pub conflict_paths: Vec<String>,
    pub drift: Vec<ParticipantDrift>,
    pub continue_eligibility: RetryEligibility,
    pub abort_eligibility: RollbackEligibility,
    pub pending_action: Option<PendingActionObservation>,
}

