//! What the caller asks for: the install request, the captured source
//! snapshot and its repositories.

use std::path::PathBuf;

use gwz_copy_contract::{CopyMode, Exclusion};
use gwz_family_model::{
    AllocationId, CloneMode, MemberKind, MemberName, MemberPath, MemberRow, MemberState, ROOT_PATH,
};
use gwz_repo_contract::{ObjectId, RepoKey, RepositoryInfo};

/// One local clone creation. Invocation-local; not a reusable authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest {
    pub name: MemberName,
    /// The registering root of the family (the index holder).
    pub root: PathBuf,
    /// The workspace being cloned (root or a ready clone).
    pub source: PathBuf,
    /// The destination as the host spells it. The store adjudicates whether
    /// it resolves to [`path`](Self::path) against the root
    /// (`StoreError::PathMismatch`); installation never does host-path
    /// arithmetic of its own.
    pub destination: PathBuf,
    /// The destination's root-relative recorded path: the row's `path`.
    pub path: MemberPath,
    /// The source's root-relative path; `None` is the root itself.
    pub source_path: Option<MemberPath>,
    /// The marker value minted for this destination.
    pub allocation: AllocationId,
    pub mode: CloneMode,
    /// `-b <branch>` for clean/bare modes.
    pub branch: Option<String>,
    /// Design §4.1 exclusions, resolved by core (family files, catalog,
    /// merge store, locks, stash bundles, `.git/worktrees`).
    pub exclusions: Vec<Exclusion>,
    /// Native copy-on-write with an ordinary fallback, or forced ordinary
    /// copying. Unused by clean/bare, which construct rather than copy.
    pub copy_mode: CopyMode,
}

impl InstallRequest {
    /// The row this request reserves.
    pub(crate) fn row(&self) -> MemberRow {
        MemberRow {
            path: self.path.as_str().to_owned(),
            kind: match self.mode {
                CloneMode::Bare => MemberKind::Bare,
                CloneMode::Verbatim | CloneMode::Clean => MemberKind::Checkout,
            },
            state: MemberState::Creating,
            allocation_id: self.allocation.clone(),
            source_path: self
                .source_path
                .as_ref()
                .map_or_else(|| ROOT_PATH.to_owned(), |path| path.as_str().to_owned()),
            mode: self.mode,
            last_error: None,
        }
    }

    /// Verbatim copies the source tree; clean and bare construct instead.
    pub(crate) fn copies_the_tree(&self) -> bool {
        self.mode == CloneMode::Verbatim
    }
}

/// Source state captured once before reservation and rechecked before
/// publication. One freeze vector for every member including the root
/// (design §4.2), held in memory for this invocation only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub repositories: Vec<CapturedRepository>,
    /// Digest of the source manifest and lock bytes.
    pub configuration_digest: String,
    /// An open gwz merge at the source, with its diagnostic detail. Verbatim
    /// refuses it (design §4.1); clean and bare do not inherit it.
    pub open_gwz_merge: Option<String>,
}

impl SourceSnapshot {
    pub(crate) fn captured(&self, key: &RepoKey) -> bool {
        self.repositories.iter().any(|repo| &repo.key == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRepository {
    pub key: RepoKey,
    pub info: RepositoryInfo,
    /// The recorded HEAD this destination freezes at; `None` when unborn.
    pub head: Option<ObjectId>,
    /// Branch names present at freeze time, for the `-b` collision check
    /// (design §4.2, refused before the `creating` row).
    pub branches: Vec<String>,
    /// Configured remote names, for the name-versus-remote check (design
    /// §2, §8.1 "origin is reserved / already a git remote").
    pub remotes: Vec<String>,
}
