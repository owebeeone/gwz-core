#[cfg(test)]
use super::super::MergeTargetKind;
use super::super::{MergeParticipantRecord, ParticipantState};

/// The durable result ownership implied by one participant state.
///
/// This is deliberately independent of transition legality, which remains
/// owned by `model.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum ParticipantResultClass {
    None,
    SuccessfulUnchanged,
    Integrated,
    Conflict,
}

impl ParticipantResultClass {
    fn is_successful(self) -> bool {
        matches!(self, Self::SuccessfulUnchanged | Self::Integrated)
    }

    fn is_integrated(self) -> bool {
        self == Self::Integrated
    }

    #[allow(
        dead_code,
        reason = "M5d lint sweep: reached only from this crate's own `cfg(test)` suites, so the non-test lib build sees it as dead; held rather than deleted."
    )]
    fn is_conflict(self) -> bool {
        self == Self::Conflict
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParticipantCount {
    Planned,
    UpToDate,
    FastForwarded,
    Merged,
    Conflicted,
    Failed,
    Unattempted,
    Continued,
    Aborted,
    RolledBack,
}

pub(in crate::workspace_ops::merge) fn wire_state(
    state: ParticipantState,
) -> crate::MergeParticipantState {
    match state {
        ParticipantState::Planned => crate::MergeParticipantState::Planned,
        ParticipantState::UpToDate => crate::MergeParticipantState::UpToDate,
        ParticipantState::FastForwarded => crate::MergeParticipantState::FastForwarded,
        ParticipantState::Merged => crate::MergeParticipantState::Merged,
        ParticipantState::Conflicted => crate::MergeParticipantState::Conflicted,
        ParticipantState::Failed => crate::MergeParticipantState::Failed,
        ParticipantState::Unattempted => crate::MergeParticipantState::Unattempted,
        ParticipantState::Continued => crate::MergeParticipantState::Continued,
        ParticipantState::Aborted => crate::MergeParticipantState::Aborted,
        ParticipantState::RolledBack => crate::MergeParticipantState::RolledBack,
    }
}

pub(in crate::workspace_ops::merge) fn increment_count(
    counts: &mut crate::MergeParticipantCounts,
    state: ParticipantState,
) {
    match count_projection(state) {
        ParticipantCount::Planned => counts.planned += 1,
        ParticipantCount::UpToDate => counts.up_to_date += 1,
        ParticipantCount::FastForwarded => counts.fast_forwarded += 1,
        ParticipantCount::Merged => counts.merged += 1,
        ParticipantCount::Conflicted => counts.conflicted += 1,
        ParticipantCount::Failed => counts.failed += 1,
        ParticipantCount::Unattempted => counts.unattempted += 1,
        ParticipantCount::Continued => counts.continued += 1,
        ParticipantCount::Aborted => counts.aborted += 1,
        ParticipantCount::RolledBack => counts.rolled_back += 1,
    }
}

pub(in crate::workspace_ops::merge) fn result_class(
    state: ParticipantState,
) -> ParticipantResultClass {
    match state {
        ParticipantState::Planned => ParticipantResultClass::None,
        ParticipantState::UpToDate => ParticipantResultClass::SuccessfulUnchanged,
        ParticipantState::FastForwarded => ParticipantResultClass::Integrated,
        ParticipantState::Merged => ParticipantResultClass::Integrated,
        ParticipantState::Conflicted => ParticipantResultClass::Conflict,
        ParticipantState::Failed => ParticipantResultClass::None,
        ParticipantState::Unattempted => ParticipantResultClass::None,
        ParticipantState::Continued => ParticipantResultClass::Integrated,
        ParticipantState::Aborted => ParticipantResultClass::None,
        ParticipantState::RolledBack => ParticipantResultClass::None,
    }
}

pub(in crate::workspace_ops::merge) fn is_successful_result(state: ParticipantState) -> bool {
    result_class(state).is_successful()
}

pub(in crate::workspace_ops::merge) fn is_integrated_result(state: ParticipantState) -> bool {
    result_class(state).is_integrated()
}

#[allow(
    dead_code,
    reason = "M5d lint sweep: reached only from this crate's own `cfg(test)` suites, so the non-test lib build sees it as dead; held rather than deleted."
)]
pub(in crate::workspace_ops::merge) fn is_conflicted_result(state: ParticipantState) -> bool {
    result_class(state).is_conflict()
}

pub(in crate::workspace_ops::merge) fn has_changed_result(
    participant: &MergeParticipantRecord,
) -> bool {
    is_integrated_result(participant.state)
        && participant.resulting_commit.as_deref() != Some(participant.before_commit.as_str())
}

fn count_projection(state: ParticipantState) -> ParticipantCount {
    match state {
        ParticipantState::Planned => ParticipantCount::Planned,
        ParticipantState::UpToDate => ParticipantCount::UpToDate,
        ParticipantState::FastForwarded => ParticipantCount::FastForwarded,
        ParticipantState::Merged => ParticipantCount::Merged,
        ParticipantState::Conflicted => ParticipantCount::Conflicted,
        ParticipantState::Failed => ParticipantCount::Failed,
        ParticipantState::Unattempted => ParticipantCount::Unattempted,
        ParticipantState::Continued => ParticipantCount::Continued,
        ParticipantState::Aborted => ParticipantCount::Aborted,
        ParticipantState::RolledBack => ParticipantCount::RolledBack,
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
