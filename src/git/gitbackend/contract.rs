use super::*;

mod managed_form;
mod observation;
mod root_preservation;

pub use managed_form::*;
pub use observation::*;
pub use root_preservation::*;

/// Whether a prepared merge may publish a fast-forward or must create a
/// two-parent merge commit when the source is strictly ahead.
///
/// Declared here, beside the trait's own rejection arm: the no-ff wire suite
/// pins the exact set of files that spell this variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitPreparedMergeMode {
    AllowFastForward,
    ForceMergeCommit,
}

#[macro_use]
mod methods_transport;
#[macro_use]
mod methods_repository;
#[macro_use]
mod methods_fetch;
#[macro_use]
mod methods_merge;
#[macro_use]
mod methods_preservation;
#[macro_use]
mod methods_worktree;
#[macro_use]
mod methods_inspection;
#[macro_use]
mod methods_commit_tag;

/// All Git operations used by GWZ. Implementations must preserve the documented
/// observation, mutation and failure semantics; unsupported operations fail closed.
/// The test-only FakeGitRepository implements this same interface.
pub trait GitRepository {
    #[cfg(test)]
    fn test_init_repo(&self, repo: &Path, spec: &TestRepoSpec) -> ModelResult<()>;
    #[cfg(test)]
    fn test_create_commit(&self, repo: &Path, spec: &TestCommitSpec) -> ModelResult<String>;
    #[cfg(test)]
    fn test_read_commit(&self, repo: &Path, oid: &str) -> ModelResult<TestCommit>;
    #[cfg(test)]
    fn test_set_ref(
        &self,
        repo: &Path,
        name: &str,
        target: Option<&TestRefTarget>,
    ) -> ModelResult<()>;
    #[cfg(test)]
    fn test_set_head(&self, repo: &Path, state: &TestHead) -> ModelResult<()>;
    #[cfg(test)]
    fn test_replace_index(&self, repo: &Path, entries: &[TestIndexEntry]) -> ModelResult<()>;
    #[cfg(test)]
    fn test_read_index(&self, repo: &Path) -> ModelResult<Vec<TestIndexEntry>>;
    #[cfg(test)]
    fn test_set_config(&self, repo: &Path, key: &str, values: &[String]) -> ModelResult<()>;
    #[cfg(test)]
    fn test_read_config(&self, repo: &Path, key: &str) -> ModelResult<Vec<String>>;
    #[cfg(test)]
    fn test_force_checkout(&self, _repo: &Path, _commit: &str) -> ModelResult<()> {
        unsupported_backend("test_force_checkout")
    }
    #[cfg(test)]
    fn test_reset_mixed(&self, _repo: &Path, _commit: &str) -> ModelResult<()> {
        unsupported_backend("test_reset_mixed")
    }
    #[cfg(test)]
    fn test_set_repository_state(
        &self,
        _repo: &Path,
        _state: GitRepositoryState,
        _merge_head: Option<&str>,
    ) -> ModelResult<()> {
        unsupported_backend("test_set_repository_state")
    }
    #[cfg(test)]
    fn test_seed_merge_conflict(
        &self,
        _repo: &Path,
        _before: &str,
        _source: &str,
    ) -> ModelResult<GitMergeConflictSnapshot> {
        unsupported_backend("test_seed_merge_conflict")
    }
    #[cfg(test)]
    fn test_create_commit_from_parent(
        &self,
        _repo: &Path,
        _parent: &str,
        _message: &str,
        _edits: &[TestCommitFileEdit],
    ) -> ModelResult<String> {
        unsupported_backend("test_create_commit_from_parent")
    }

    /// Read preservation stashes for exactly this merge, without mutation.
    /// Keep duplicate matches visible so callers can reject ambiguity.
    fn preservation_stashes(
        &self,
        _path: &Path,
        _merge_id: &str,
    ) -> ModelResult<Vec<GitPreservationStashEvidence>> {
        unsupported_backend("preservation_stashes")
    }

    repository_contract_transport!();
    repository_contract_repository!();
    repository_contract_fetch!();
    repository_contract_merge!();
    repository_contract_preservation!();
    repository_contract_worktree!();
    repository_contract_inspection!();
    repository_contract_commit_tag!();
}

fn unsupported_backend<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("{method} is not implemented by this GitBackend"),
    ))
}

/// Compatibility name for existing callers; this is the same trait.
pub use GitRepository as GitBackend;
