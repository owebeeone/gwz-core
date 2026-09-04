mod archive;
mod rewrite;
mod unknown;

use std::path::Path;

use super::checked::{StoredV1Record, V1MutationLease};
use super::transition::PreparedV1Rewrite;
use crate::model::{ErrorCode, ModelError, ModelResult};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CheckedV1Store {
    commit_fault: Option<CommitFault>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommitFault {
    AfterTemporarySync,
    AfterPublish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ArchiveOutcome {
    Published,
    ReconciledDestination,
    ReconciledBothCopies,
}

impl CheckedV1Store {
    pub(super) fn load_open(&self, root: &Path, merge_id: &str) -> ModelResult<StoredV1Record> {
        rewrite::load_open(root, merge_id)
    }

    /// A1's creation owner for the contract-§2 writer floor. See
    /// `rewrite::create_open`. `crash_recovery` is the start's own decision,
    /// threaded to the boundary door (M5d charter §3); `None` on every path
    /// that made no decision, which keeps today's checked publication.
    pub(super) fn create_open(
        &self,
        lease: &V1MutationLease,
        root: &Path,
        record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
        crash_recovery: Option<&crate::checked_artifact::entry::CrashRecoveryDecision>,
    ) -> ModelResult<StoredV1Record> {
        rewrite::create_open(lease, root, record, crash_recovery)
    }

    pub(super) fn reload_unchanged(&self, current: &StoredV1Record) -> ModelResult<StoredV1Record> {
        let reopened = self.load_open(current.location().root(), &current.record().merge_id)?;
        if !current.same_source_as(&reopened) {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "checked v1 source bytes changed across physical execution",
            ));
        }
        Ok(reopened)
    }

    pub(super) fn commit(
        &self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        rewrite: PreparedV1Rewrite,
    ) -> ModelResult<StoredV1Record> {
        rewrite::commit(lease, current, rewrite, self.commit_fault)
    }

    pub(super) fn archive(
        &self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
    ) -> ModelResult<ArchiveOutcome> {
        archive::archive(lease, current)
    }

    #[cfg(test)]
    pub(super) fn failing_after(fault: CommitFault) -> Self {
        Self {
            commit_fault: Some(fault),
        }
    }
}
