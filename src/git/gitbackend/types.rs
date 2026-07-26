use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCreateResult {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCloneResult {
    pub path: PathBuf,
    pub head: GitHeadState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFetchResult {
    pub remote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteRef {
    /// Full ref name as advertised by the remote (e.g. `refs/heads/main`, `HEAD`).
    pub name: String,
    /// Object id the ref points at, as a hex string.
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUpdateResult {
    pub updated: bool,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBranch {
    pub name: String,
    pub commit: String,
    pub is_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBranchCreateResult {
    pub branch: GitBranch,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitStashPushOptions {
    pub include_untracked: bool,
    pub include_ignored: bool,
    /// Preserve staged index entries in the worktree after pushing, matching
    /// `git stash push --keep-index`.
    pub preserve_index: bool,
}

impl GitStashPushOptions {
    pub fn tracked_only() -> Self {
        Self::default()
    }

    pub fn include_untracked() -> Self {
        Self {
            include_untracked: true,
            ..Self::default()
        }
    }

    pub fn include_ignored() -> Self {
        Self {
            include_untracked: true,
            include_ignored: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitStashRestoreOptions {
    /// Default restore attempts to reinstate the index (`git stash apply --index`).
    pub preserve_index: bool,
}

impl Default for GitStashRestoreOptions {
    fn default() -> Self {
        Self {
            preserve_index: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStashPushResult {
    pub object_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStashEntry {
    pub index: usize,
    pub object_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStashTarget {
    /// Exact native stash object id. This may target any stash, including non-GWZ stashes.
    pub object_id: Option<String>,
    /// GWZ message prefix fallback, e.g. `gwz:stash_123:`. Prefix fallback is
    /// intentionally restricted to `gwz:` messages so non-GWZ stashes are never
    /// mutated by fuzzy identity after native indices move.
    pub gwz_message_prefix: Option<String>,
}

impl GitStashTarget {
    pub fn object_id(object_id: impl Into<String>) -> Self {
        Self {
            object_id: Some(object_id.into()),
            gwz_message_prefix: None,
        }
    }

    pub fn gwz_message_prefix(prefix: impl Into<String>) -> Self {
        Self {
            object_id: None,
            gwz_message_prefix: Some(prefix.into()),
        }
    }
}

/// Outcome of a merge/rebase integration. A conflict is reported, not errored:
/// `conflicts` names the paths and `commit` is `None`, with the worktree left
/// mid-integration for the developer to resolve — exactly as porcelain git leaves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitIntegrateResult {
    /// New HEAD commit when the integration completed cleanly; `None` on conflict.
    pub commit: Option<String>,
    /// Conflicted paths; empty iff the integration completed cleanly.
    pub conflicts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitMergeAnalysisKind {
    UpToDate,
    FastForward,
    TrueMerge,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitMergeAnalysis {
    pub target_branch: String,
    pub target_commit: String,
    pub source_commit: String,
    pub kind: GitMergeAnalysisKind,
    pub commit_identity_required: bool,
    /// False for a true merge that has not run the optional simulation seam.
    pub prediction_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitMergeSimulation {
    Clean,
    Conflicts(Vec<String>),
}

/// A fully resolved Git signature frozen before a durable commit intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPreparedSignature {
    pub name: String,
    pub email: String,
    pub time_seconds: i64,
    pub timezone_offset_minutes: i32,
}

/// Exact content and identities for an unattached merge commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPreparedCommit {
    pub tree_oid: String,
    pub author: GitPreparedSignature,
    pub committer: GitPreparedSignature,
}

/// Result class frozen before a checked merge action is made durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitPreparedMerge {
    Unchanged,
    FastForward,
    ExpectedConflict,
    Commit(GitPreparedCommit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitNativeMergeState {
    pub merge_head: String,
    pub conflict_paths: Vec<String>,
    pub unresolved_entries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitConflictFileSnapshot {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitMergeConflictSnapshot {
    pub files: Vec<GitConflictFileSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRepositoryState {
    Clean,
    Merge,
    Revert,
    RevertSequence,
    CherryPick,
    CherryPickSequence,
    Bisect,
    Rebase,
    RebaseInteractive,
    RebaseMerge,
    ApplyMailbox,
    ApplyMailboxOrRebase,
}

impl GitRepositoryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Merge => "merge",
            Self::Revert => "revert",
            Self::RevertSequence => "revert_sequence",
            Self::CherryPick => "cherry_pick",
            Self::CherryPickSequence => "cherry_pick_sequence",
            Self::Bisect => "bisect",
            Self::Rebase => "rebase",
            Self::RebaseInteractive => "rebase_interactive",
            Self::RebaseMerge => "rebase_merge",
            Self::ApplyMailbox => "apply_mailbox",
            Self::ApplyMailboxOrRebase => "apply_mailbox_or_rebase",
        }
    }
}

pub(crate) fn map_repository_state(state: git2::RepositoryState) -> GitRepositoryState {
    match state {
        git2::RepositoryState::Clean => GitRepositoryState::Clean,
        git2::RepositoryState::Merge => GitRepositoryState::Merge,
        git2::RepositoryState::Revert => GitRepositoryState::Revert,
        git2::RepositoryState::RevertSequence => GitRepositoryState::RevertSequence,
        git2::RepositoryState::CherryPick => GitRepositoryState::CherryPick,
        git2::RepositoryState::CherryPickSequence => GitRepositoryState::CherryPickSequence,
        git2::RepositoryState::Bisect => GitRepositoryState::Bisect,
        git2::RepositoryState::Rebase => GitRepositoryState::Rebase,
        git2::RepositoryState::RebaseInteractive => GitRepositoryState::RebaseInteractive,
        git2::RepositoryState::RebaseMerge => GitRepositoryState::RebaseMerge,
        git2::RepositoryState::ApplyMailbox => GitRepositoryState::ApplyMailbox,
        git2::RepositoryState::ApplyMailboxOrRebase => GitRepositoryState::ApplyMailboxOrRebase,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBackupRefResult {
    pub name: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCandidateFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCandidateHash {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitScopedCommitResult {
    pub commit: String,
    pub tree: String,
    pub candidate_hashes: Vec<GitCandidateHash>,
}

impl GitIntegrateResult {
    pub(crate) fn clean(commit: String) -> Self {
        Self {
            commit: Some(commit),
            conflicts: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteResult {
    pub remote: GitRemote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPushResult {
    pub remote: String,
    pub refspec: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStageResult {
    /// Top-level *file* pathspecs confirmed present in the index by the self-verify
    /// pass. Directory pathspecs are staged but not counted here.
    pub staged: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCommitResult {
    /// The new commit oid created by this commit.
    pub commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitTagResult {
    /// The tag name created.
    pub name: String,
    /// The commit oid the tag points at (peeled through annotated tags).
    pub commit: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStatus {
    pub is_dirty: bool,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub ignored: usize,
    pub unresolved: usize,
    pub files: Vec<GitFileStatus>,
}

impl GitStatus {
    pub fn clean() -> Self {
        Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitStatusOptions {
    pub include_ignored: bool,
}

impl GitStatusOptions {
    pub fn include_ignored() -> Self {
        Self {
            include_ignored: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHeadState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub is_detached: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemote {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}
