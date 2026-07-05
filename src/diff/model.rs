//! Internal, repo-scoped diff model.
//!
//! D1 owns the Git backend primitive: diffing exactly **one** libgit2
//! repository (the workspace root, or a single materialized member) and
//! producing a manifest of changed files with repo-relative paths. Workspace
//! projection — mapping these entries onto the wire [`DiffFileEntry`] with a
//! [`DiffRepoScope`] and workspace-relative paths, root/member ordering, and
//! `gwz.conf`/member-path exclusion — is D2 and lives in the diff planner, not
//! here.
//!
//! These structs are deliberately *not* the generated protocol messages. The
//! wire types (`DiffComparison`, `DiffOptions`, `DiffFileEntry`, …) are
//! workspace-oriented: they carry scopes, snapshot ids, opaque `file_id`s, and
//! `Option`-wrapped nullable fields for CBOR. The backend primitive works below
//! that layer — one repo, repo-relative paths, resolved oids — so a compact
//! internal model keeps the libgit2 code honest and lets D2 own the wire
//! mapping. Where a value maps cleanly onto a generated enum we reuse it
//! ([`RepoDiffStatus`] ⇄ [`DiffStatus`], comparison kind ⇄
//! [`DiffComparisonKind`]) rather than duplicating the vocabulary.

use crate::protocol::generated::{
    DiffAlgorithm, DiffComparisonKind, DiffStatus, DiffWhitespaceMode,
};

/// Which two sides libgit2 compares, with the old/new tree sides already
/// resolved to object ids where the side is a tree. `None` on a tree side means
/// the empty tree (an unborn `HEAD` under `--cached`, or an added/deleted
/// endpoint); a worktree/index side carries no oid because it is not an object.
///
/// This mirrors the wire [`DiffComparisonKind`] but adds the *resolved*
/// endpoints the backend needs to actually run the diff. The revision-token →
/// oid resolution happens per repo (see [`super::resolve_comparison`]).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoDiffComparison {
    pub kind: RepoDiffComparisonKind,
    /// Old-side tree oid (hex). `None` = empty tree. Ignored for the
    /// worktree/index old side of [`RepoDiffComparisonKind::WorktreeVsIndex`].
    pub old_tree: Option<String>,
    /// New-side tree oid (hex) for the tree-vs-tree case. `None` = empty tree.
    /// Ignored when the new side is the index or worktree.
    pub new_tree: Option<String>,
    /// Resolved oid for the request's left endpoint when that endpoint is a Git
    /// object. Worktree/index sides omit it.
    pub left_oid: Option<String>,
    /// Resolved oid for the request's right endpoint when that endpoint is a Git
    /// object. Worktree/index sides omit it.
    pub right_oid: Option<String>,
    /// Resolved merge-base oid when `--merge-base` / `A...B` changed the old
    /// diff side. The diff itself still uses [`Self::old_tree`] as its old side.
    pub merge_base_oid: Option<String>,
}

/// The repo-scoped comparison kind. 1:1 with the wire [`DiffComparisonKind`];
/// kept as its own type so the internal model does not depend on CBOR-facing
/// details and can carry a sensible [`Default`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RepoDiffComparisonKind {
    /// `git diff`: index → worktree.
    #[default]
    WorktreeVsIndex,
    /// `git diff --cached [<commit>]`: tree → index.
    IndexVsTree,
    /// `git diff <commit>`: tree → worktree, blending index data for staged
    /// deletes.
    WorktreeVsTree,
    /// `git diff <a> <b>` / `<a>..<b>` / `<a>...<b>`: tree → tree.
    TreeVsTree,
}

impl RepoDiffComparisonKind {
    /// The matching wire enum, for D2's manifest projection.
    pub fn to_wire(self) -> DiffComparisonKind {
        match self {
            Self::WorktreeVsIndex => DiffComparisonKind::WorktreeVsIndex,
            Self::IndexVsTree => DiffComparisonKind::IndexVsTree,
            Self::WorktreeVsTree => DiffComparisonKind::WorktreeVsTree,
            Self::TreeVsTree => DiffComparisonKind::TreeVsTree,
        }
    }

    pub fn from_wire(kind: DiffComparisonKind) -> Self {
        match kind {
            DiffComparisonKind::WorktreeVsIndex => Self::WorktreeVsIndex,
            DiffComparisonKind::IndexVsTree => Self::IndexVsTree,
            DiffComparisonKind::WorktreeVsTree => Self::WorktreeVsTree,
            DiffComparisonKind::TreeVsTree => Self::TreeVsTree,
        }
    }
}

/// The knobs that change which bytes/deltas libgit2 emits, mapped from the wire
/// [`DiffOptions`](crate::protocol::generated::DiffOptions) but reduced to the
/// subset the backend primitive honors in D1. Presentation-only fields
/// (prefixes, line_prefix, null_terminated, output_format) live on the wire
/// message and are applied by the D3/D4 renderer, not the manifest primitive.
///
/// `find_copies` is intentionally absent: v0 rejects `find_copies=true` as an
/// `unsupported_operation` *before* reaching the backend (see
/// [`super::reject_unsupported_options`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepoDiffOptions {
    /// Repo-relative pathspecs (member prefix already stripped by D2). Empty =
    /// whole repo.
    pub pathspecs: Vec<String>,
    /// Context lines around each hunk (libgit2 default 3 when `None`).
    pub context_lines: Option<u32>,
    pub interhunk_lines: Option<u32>,
    pub algorithm: RepoDiffAlgorithm,
    pub whitespace: RepoDiffWhitespace,
    /// Run rename detection (`Diff::find_similar`).
    pub find_renames: bool,
    /// Rename similarity threshold 0..=100. `None` = libgit2 default (50).
    pub rename_threshold: Option<u16>,
    /// Cap on rename-detection candidate walk. `None` = libgit2 default.
    pub rename_limit: Option<usize>,
    /// Force files to be treated as text (`git diff --text`).
    pub force_text: bool,
    /// Include type-change deltas (blob ⇄ symlink, etc.). Git shows these by
    /// default; libgit2 needs the flag set.
    pub include_typechange: bool,
    /// Reverse the diff sides.
    pub reverse: bool,
}

impl RepoDiffOptions {
    /// Options that produce a plain `git diff` for the whole repo.
    pub fn full_repo() -> Self {
        Self {
            include_typechange: true,
            ..Self::default()
        }
    }
}

/// Diff algorithm, 1:1 with the wire [`DiffAlgorithm`]. Histogram is absent:
/// the `git2 0.21` wrapper has no setter and the protocol rejects it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RepoDiffAlgorithm {
    #[default]
    Default,
    Myers,
    Minimal,
    Patience,
}

impl RepoDiffAlgorithm {
    pub fn from_wire(algorithm: DiffAlgorithm) -> Self {
        match algorithm {
            DiffAlgorithm::Default => Self::Default,
            DiffAlgorithm::Myers => Self::Myers,
            DiffAlgorithm::Minimal => Self::Minimal,
            DiffAlgorithm::Patience => Self::Patience,
        }
    }
}

/// Whitespace handling, 1:1 with the wire [`DiffWhitespaceMode`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RepoDiffWhitespace {
    #[default]
    Default,
    IgnoreAll,
    IgnoreChange,
    IgnoreEol,
    IgnoreBlankLines,
}

impl RepoDiffWhitespace {
    pub fn from_wire(mode: DiffWhitespaceMode) -> Self {
        match mode {
            DiffWhitespaceMode::Default => Self::Default,
            DiffWhitespaceMode::IgnoreAll => Self::IgnoreAll,
            DiffWhitespaceMode::IgnoreChange => Self::IgnoreChange,
            DiffWhitespaceMode::IgnoreEol => Self::IgnoreEol,
            DiffWhitespaceMode::IgnoreBlankLines => Self::IgnoreBlankLines,
        }
    }
}

/// Per-file change classification. Mirrors the wire [`DiffStatus`] minus
/// `Copied` (v0 rejects copy detection) and with `Unmerged` folded onto
/// libgit2's `Conflicted` delta.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoDiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
    Unmerged,
}

impl RepoDiffStatus {
    /// The matching wire enum, for D2's manifest projection.
    pub fn to_wire(self) -> DiffStatus {
        match self {
            Self::Added => DiffStatus::Added,
            Self::Modified => DiffStatus::Modified,
            Self::Deleted => DiffStatus::Deleted,
            Self::Renamed => DiffStatus::Renamed,
            Self::TypeChanged => DiffStatus::TypeChanged,
            Self::Unmerged => DiffStatus::Unmerged,
        }
    }

    /// Git `--name-status` letter (`A`/`M`/`D`/`R`/`T`/`U`), for parity tests
    /// and machine-mode rendering without going through the patch bytes.
    pub fn status_char(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::TypeChanged => 'T',
            Self::Unmerged => 'U',
        }
    }
}

/// One changed file in a single repository, with repo-relative paths.
///
/// `old_path`/`new_path` follow libgit2: for add the old path is absent, for
/// delete the new path is absent, for rename both are present and differ, and
/// for modify/type-change both are present and equal. Modes are the raw octal
/// Git file modes (e.g. `0o100644`, `0o100755`, `0o120000`), `None` when the
/// side is absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoDiffEntry {
    pub status: RepoDiffStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    /// Rename similarity 0..=100; `None` for non-rename entries.
    pub similarity: Option<u16>,
    /// Added lines. `None` when the delta is binary (no textual line stats).
    pub insertions: Option<usize>,
    /// Removed lines. `None` when the delta is binary.
    pub deletions: Option<usize>,
    pub is_binary: bool,
}

impl RepoDiffEntry {
    /// The path Git would key `--name-status` output on: the new side, falling
    /// back to the old side for a deletion.
    pub fn primary_path(&self) -> Option<&str> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }
}

/// The diff of one repository: its changed-file entries in libgit2 order, plus
/// the repo-level rollup. `has_differences` is the exit-code / `--quiet` signal
/// for this repo before workspace aggregation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoDiffManifest {
    pub entries: Vec<RepoDiffEntry>,
    /// Total added lines across all textual entries.
    pub insertions: usize,
    /// Total removed lines across all textual entries.
    pub deletions: usize,
}

impl RepoDiffManifest {
    pub fn has_differences(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn files_changed(&self) -> usize {
        self.entries.len()
    }
}
