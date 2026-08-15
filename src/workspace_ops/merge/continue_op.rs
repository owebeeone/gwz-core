use std::path::Path;

use crate::git::{
    GitBackend, GitIntegrateResult, GitMergeAnalysisKind, GitPreparedCommit, GitPreparedMerge,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext, WorkspaceMutatorLock};

use super::{
    ConflictFileEvidence, MergeExecutionMode, MergeOperationRecord, MergeParticipantRecord,
    MergeRecordError, MergeStore, OperationState, ParticipantState, PendingMergeActionKind,
    PendingMergeExpectedResult,
};

mod coordinator;
mod execution;
mod reconciliation;
#[cfg(test)]
mod tests;

pub(crate) use coordinator::handle_continue;

use execution::*;
use reconciliation::*;

#[derive(Clone, Copy)]
enum ContinueActionKind {
    Resolve,
    Retry(GitMergeAnalysisKind),
}

struct ContinueAction {
    target_id: String,
    path: String,
    kind: ContinueActionKind,
    prepared: ContinuePrepared,
    durable: bool,
}

enum ContinuePrepared {
    Merge(GitPreparedMerge),
    Resolution(GitPreparedCommit),
}

enum ActionFailure {
    Ordinary(ModelError),
    RecoveryRequired(ModelError),
}
