use serde::{Deserialize, Serialize};

use crate::model::{ErrorCode, ModelError, ModelResult};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeExecutionMode {
    #[default]
    Normal,
    FfOnly,
    NoFf,
}

impl MergeExecutionMode {
    pub(super) fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
}

impl From<Option<crate::MergeMode>> for MergeExecutionMode {
    fn from(value: Option<crate::MergeMode>) -> Self {
        match value.unwrap_or_default() {
            crate::MergeMode::Normal => Self::Normal,
            crate::MergeMode::FfOnly => Self::FfOnly,
            crate::MergeMode::NoFf => Self::NoFf,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeTargetKind {
    Member,
    Root,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantState {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Executing,
    AwaitingResolution,
    Halted,
    Finalizing,
    Preserving,
    RollingBack,
    Completed,
    Aborted,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationStep {
    NotStarted,
    ValidatingResults,
    PreparingCandidate,
    CommittingEvidence,
    PublishingCandidate,
    VerifyingPublication,
    Complete,
}

impl OperationState {
    pub(crate) fn is_open(self) -> bool {
        !matches!(self, Self::Completed | Self::Aborted)
    }

    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        let legal = self == next
            || matches!(
                (self, next),
                (
                    Self::Executing,
                    Self::AwaitingResolution
                        | Self::Halted
                        | Self::Finalizing
                        | Self::Preserving
                        | Self::RecoveryRequired
                ) | (
                    Self::AwaitingResolution,
                    Self::Executing
                        | Self::Finalizing
                        | Self::Preserving
                        | Self::RollingBack
                        | Self::RecoveryRequired
                ) | (
                    Self::Halted,
                    Self::Executing | Self::Preserving | Self::RollingBack | Self::RecoveryRequired
                ) | (
                    Self::Finalizing,
                    Self::Completed | Self::Preserving | Self::RollingBack | Self::RecoveryRequired
                ) | (Self::Preserving, Self::RollingBack | Self::RecoveryRequired)
                    | (Self::RollingBack, Self::Aborted | Self::RecoveryRequired)
                    | (
                        Self::RecoveryRequired,
                        Self::Executing | Self::RollingBack | Self::Preserving
                    )
            );
        legal
            .then_some(next)
            .ok_or_else(|| transition_error("operation", self, next))
    }
}

impl ParticipantState {
    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        let attempted = matches!(
            next,
            Self::UpToDate | Self::FastForwarded | Self::Merged | Self::Conflicted | Self::Failed
        );
        let legal = self == next
            || matches!(self, Self::Planned | Self::Unattempted | Self::Failed) && attempted
            || matches!((self, next), (Self::Planned, Self::Unattempted))
            || matches!(
                (self, next),
                (
                    Self::Planned
                        | Self::Unattempted
                        | Self::Failed
                        | Self::UpToDate
                        | Self::Conflicted,
                    Self::Aborted
                ) | (Self::Conflicted, Self::Continued)
                    | (
                        Self::FastForwarded | Self::Merged | Self::Continued,
                        Self::RolledBack
                    )
            );
        legal
            .then_some(next)
            .ok_or_else(|| transition_error("participant", self, next))
    }
}

impl PublicationStep {
    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        (next >= self)
            .then_some(next)
            .ok_or_else(|| transition_error("publication", self, next))
    }
}

fn transition_error<T: std::fmt::Debug>(kind: &str, from: T, to: T) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("illegal merge {kind} transition: {from:?} -> {to:?}"),
    )
}
