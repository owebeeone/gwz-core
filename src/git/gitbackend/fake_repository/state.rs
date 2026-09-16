//! The fake repository handle and the in-memory state it shares.
//!
//! Handles created through the factory share one state map, the way native
//! handles opening the same path see the same repository; `Default` fixtures
//! keep isolated state for the adapter contract tests.

use super::*;

pub(super) type FileTree = BTreeMap<String, Vec<u8>>;

#[derive(Clone)]
pub(crate) struct FakeGitRepository {
    pub(in crate::git::gitbackend) filesystem: Arc<dyn FileSystem>,
    pub(super) repositories: Arc<Mutex<BTreeMap<PathBuf, RepositoryState>>>,
}
impl Default for FakeGitRepository {
    fn default() -> Self {
        Self::with_filesystem(Arc::new(make_filesystem()))
    }
}
impl FakeGitRepository {
    pub(crate) fn with_filesystem(filesystem: Arc<dyn FileSystem>) -> Self {
        Self {
            filesystem,
            repositories: Default::default(),
        }
    }

    /// Factory-created handles share repository state, just as native handles
    /// opening the same path see the same repository. Direct Default fixtures
    /// retain isolated state for adapter contract tests.
    pub(in crate::git::gitbackend) fn shared() -> Self {
        static REPOSITORIES: OnceLock<Arc<Mutex<BTreeMap<PathBuf, RepositoryState>>>> =
            OnceLock::new();
        Self {
            filesystem: Arc::new(make_filesystem()),
            repositories: Arc::clone(REPOSITORIES.get_or_init(Default::default)),
        }
    }
}
#[derive(Default)]
pub(super) struct RepositoryState {
    pub(super) index: BTreeMap<String, Vec<u8>>,
    pub(super) index_override: Option<Vec<TestIndexEntry>>,
    pub(super) blobs: BTreeMap<String, Vec<u8>>,
    pub(super) metadata: BTreeMap<String, TestCommit>,
    pub(super) config: BTreeMap<String, Vec<String>>,
    pub(super) attached_ref: Option<String>,
    pub(super) sha256: bool,
    pub(super) detached: bool,
    pub(super) commits: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    pub(super) head: Option<String>,
    pub(super) refs: BTreeMap<String, String>,
    pub(super) symbolic_refs: BTreeMap<String, String>,
    pub(super) parents: BTreeMap<String, Option<String>>,
    pub(super) stashes: BTreeMap<String, GitPreservationStashEvidence>,
    pub(super) stash_snapshots: BTreeMap<String, (FileTree, FileTree)>,
    pub(super) repository_state: Option<GitRepositoryState>,
    pub(super) merge_head: Option<String>,
    pub(super) merge_conflict_snapshot: Option<GitMergeConflictSnapshot>,
    pub(super) merge_index: Option<Vec<TestIndexEntry>>,
}
pub(super) fn unsupported<T>(operation: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("fake Git operation not implemented: {operation}"),
    ))
}
