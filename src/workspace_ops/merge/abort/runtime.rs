use super::super::{
    MergeOperationRecord, MergeStatusSnapshot,
    publication::{RootEvidenceObservation, observe_root_evidence},
};
use crate::git::{GitBackend, GitCandidateFile, GitHeadState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use std::path::Path;

pub(super) trait AbortRuntime {
    fn snapshot(
        &self,
        root: &Path,
        record: MergeOperationRecord,
    ) -> ModelResult<MergeStatusSnapshot>;
    fn abort_merge(&self, path: &Path, before: &str, merge_head: &str) -> ModelResult<()>;
    fn reset_branch(
        &self,
        path: &Path,
        branch: &str,
        current: &str,
        before: &str,
    ) -> ModelResult<()>;
    fn head(&self, _path: &Path) -> ModelResult<GitHeadState> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence inspection",
        ))
    }
    fn observe_root_evidence(
        &self,
        _root: &Path,
        _record: &MergeOperationRecord,
    ) -> ModelResult<Option<RootEvidenceObservation>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence observation",
        ))
    }
    fn rollback_evidence_commit(
        &self,
        _root: &Path,
        _branch: &str,
        _commit: &str,
        _parent: Option<&str>,
        _files: &[GitCandidateFile],
        _message: &str,
    ) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence rollback",
        ))
    }
    fn stage_paths(&self, _path: &Path, _paths: &[&str]) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence staging",
        ))
    }
    fn root_finalization_is_exact(
        &self,
        _root: &Path,
        _record: &MergeOperationRecord,
    ) -> ModelResult<bool> {
        Ok(false)
    }
}

pub(super) struct GitAbortRuntime<'a, B>(pub(super) &'a B);

impl<B: GitBackend> AbortRuntime for GitAbortRuntime<'_, B> {
    fn snapshot(
        &self,
        root: &Path,
        record: MergeOperationRecord,
    ) -> ModelResult<MergeStatusSnapshot> {
        super::super::status::snapshot_status(self.0, root, record)
    }

    fn abort_merge(&self, path: &Path, before: &str, merge_head: &str) -> ModelResult<()> {
        self.0.abort_merge(path, before, merge_head)
    }

    fn reset_branch(
        &self,
        path: &Path,
        branch: &str,
        current: &str,
        before: &str,
    ) -> ModelResult<()> {
        self.0
            .set_branch_target_checked(path, branch, current, before)
            .map(|_| ())
    }

    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        self.0.head(path)
    }

    fn observe_root_evidence(
        &self,
        root: &Path,
        record: &MergeOperationRecord,
    ) -> ModelResult<Option<RootEvidenceObservation>> {
        observe_root_evidence(self.0, root, record)
    }

    fn rollback_evidence_commit(
        &self,
        root: &Path,
        branch: &str,
        commit: &str,
        parent: Option<&str>,
        files: &[GitCandidateFile],
        message: &str,
    ) -> ModelResult<()> {
        self.0
            .rollback_gwz_paths_commit_checked(root, branch, commit, parent, files, message)
    }

    fn stage_paths(&self, path: &Path, paths: &[&str]) -> ModelResult<()> {
        self.0.stage_paths(path, paths).map(|_| ())
    }

    fn root_finalization_is_exact(
        &self,
        root: &Path,
        record: &MergeOperationRecord,
    ) -> ModelResult<bool> {
        super::super::root::root_finalization_is_exact(self.0, root, record)
    }
}
