//! Repository observation contract for the GWZ local clone family.
//!
//! This crate owns the plain values that describe a Git repository's layout,
//! its unsaved work and its protected history, plus two read-only ports:
//! [`RepoInspector`] (layout / work / history inventory of one repository
//! path) and [`ObjectReader`] (bounded object-graph reads for history
//! verification). It contains no Git implementation: `gwz-repo-inspect`
//! implements both ports over the local filesystem and Git object store;
//! `gwz-work-detector`, `gwz-history-check`, `gwz-local-import`,
//! `gwz-workspace-install`, `gwz-repo-factory` and `gwz-local-disposal`
//! consume the values and ports.
//!
//! Contract (gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §3,
//! `GwzLocalCloneImplementationArchitecture.md` §4/§5):
//!
//! - [`ObjectId`] carries its object format; nothing assumes 40 hex chars.
//! - Path names are bytes ([`BytePath`]); they need not be UTF-8.
//! - Observations distinguish known from unknown ([`Observation`]), and a
//!   known work observation distinguishes conflicts, ignored data and
//!   status-suppression flags with their physical state.
//! - Reads are bounded by [`ReadLimits`] and never fetch, rewrite an index,
//!   clear a flag or run maintenance. A read that cannot stay within its
//!   bounds fails typed; it never guesses.
//!
//! No `git2` type, OS handle, core model error or protocol type crosses this
//! boundary. Adapters translate at the edge.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(any(test, feature = "contract-tests"))]
pub mod contract_tests;

/// A repository-relative path as Git stores it: raw bytes, not necessarily
/// UTF-8, with `/` separators.
pub type BytePath = Vec<u8>;

/// The hash function of a repository's object store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

impl ObjectFormat {
    /// Digest length in bytes.
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
}

/// A typed object id: the digest bytes plus the format that produced them.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    format: ObjectFormat,
    bytes: Vec<u8>,
}

impl ObjectId {
    pub fn from_bytes(format: ObjectFormat, bytes: &[u8]) -> Result<Self, ObjectIdError> {
        if bytes.len() != format.digest_len() {
            return Err(ObjectIdError::Length {
                format,
                expected: format.digest_len(),
                actual: bytes.len(),
            });
        }
        Ok(Self {
            format,
            bytes: bytes.to_vec(),
        })
    }

    pub fn parse_hex(format: ObjectFormat, hex: &str) -> Result<Self, ObjectIdError> {
        if hex.len() != format.digest_len() * 2 {
            return Err(ObjectIdError::Length {
                format,
                expected: format.digest_len(),
                actual: hex.len() / 2,
            });
        }
        let mut bytes = Vec::with_capacity(format.digest_len());
        for pair in hex.as_bytes().chunks(2) {
            let text = std::str::from_utf8(pair).map_err(|_| ObjectIdError::Hex {
                detail: "non-ascii hex".to_owned(),
            })?;
            let value = u8::from_str_radix(text, 16).map_err(|error| ObjectIdError::Hex {
                detail: error.to_string(),
            })?;
            bytes.push(value);
        }
        Ok(Self { format, bytes })
    }

    pub fn format(&self) -> ObjectFormat {
        self.format
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to_hex(&self) -> String {
        self.bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}:{}", self.format, self.to_hex())
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectIdError {
    Length {
        format: ObjectFormat,
        expected: usize,
        actual: usize,
    },
    Hex {
        detail: String,
    },
}

impl fmt::Display for ObjectIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length {
                format,
                expected,
                actual,
            } => write!(
                f,
                "object id length {actual} does not match {format:?} ({expected} bytes)"
            ),
            Self::Hex { detail } => write!(f, "invalid hex object id: {detail}"),
        }
    }
}

impl std::error::Error for ObjectIdError {}

/// Identity used to pair repositories across family members: the workspace
/// root, or a member by its manifest id. Paths are never the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RepoKey {
    Root,
    Member { id: String },
}

impl fmt::Display for RepoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("@root"),
            Self::Member { id } => f.write_str(id),
        }
    }
}

/// What `HEAD` points at.
///
/// `branch` is the **full** reference name (`refs/heads/main`), never the
/// short one, matching [`RootSource::Ref`]'s `name` (lane I proposal I-1,
/// pinned in LCM1.0c follow-up 3; `contract_tests::observations_are_repeatable`
/// asserts the `refs/` prefix on every inspector it is run against).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadState {
    Attached { branch: String, target: ObjectId },
    Detached { target: ObjectId },
    Unborn { branch: String },
}

/// An admitted repository layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInfo {
    /// The path that was inspected (worktree root, or the Git directory of
    /// a bare repository), **resolved** (`std::fs::canonicalize`) like
    /// `git_dir` and `common_dir`, so a boundary comparison and an equality
    /// comparison mean the same thing on a host whose temporary directory
    /// is itself a symlink (lane I proposal I-1, pinned in LCM1.0c follow-up
    /// 3; the conformance suite asserts all three are absolute).
    pub path: PathBuf,
    /// Resolved, as `path` is.
    pub git_dir: PathBuf,
    /// Resolved, as `path` is.
    pub common_dir: PathBuf,
    pub bare: bool,
    pub object_format: ObjectFormat,
    pub head: HeadState,
}

/// One reason a layout is unsupported for local clone (design §4.0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutHazard {
    /// `.git` is a file (gitfile / linked worktree).
    GitFile { path: PathBuf },
    /// `commondir` resolves outside the repository.
    ExternalCommonDir { path: PathBuf },
    /// `objects/info/alternates` or `http-alternates` is present.
    Alternates { path: PathBuf },
    /// Git metadata or the object store resolves through a symlink outside
    /// the copied boundary.
    EscapingMetadataLink { path: PathBuf, target: PathBuf },
    /// A configuration value would still name a path outside the destination
    /// after copy (`core.worktree`, effective `core.hooksPath`, `include.path`,
    /// `includeIf`, `url.*.insteadOf`).
    EscapingConfig { key: String, value: String },
    /// An effective configuration value could not be resolved.
    UnresolvableConfig { key: String, detail: String },
    /// Partial-clone / promisor objects that are not available locally.
    PartialClone { detail: String },
    /// An invocation-level environment override that would redirect Git
    /// metadata (`GIT_DIR`, `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`,
    /// `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_WORK_TREE`).
    EnvironmentOverride { variable: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    NotARepository {
        path: PathBuf,
    },
    /// Every hazard found, aggregated; the caller refuses before reservation.
    Unsupported {
        path: PathBuf,
        hazards: Vec<LayoutHazard>,
    },
    /// The repository could not be opened or read far enough to classify.
    /// Reports only the failure: hazards observed before the failing read
    /// (an environment override, a gitfile) are **not** carried, because a
    /// `ReadFailed` is not a layout verdict -- the caller refuses either
    /// way, and the next successful inspection reports every hazard (lane I
    /// proposal I-3, LCM1.0c follow-up 3: recorded, not aggregated).
    ReadFailed {
        path: PathBuf,
        detail: String,
    },
    /// This inspector does not inspect layouts.
    Unimplemented {
        operation: &'static str,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository { path } => write!(f, "{} is not a repository", path.display()),
            Self::Unsupported { path, hazards } => {
                write!(
                    f,
                    "{} has an unsupported layout: {hazards:?}",
                    path.display()
                )
            }
            Self::ReadFailed { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// A value the observer could establish, or the reasons it could not.
/// `Unknown` is never treated as clean, preserved or admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation<T> {
    Known(T),
    Unknown(Vec<UnknownReason>),
}

impl<T> Observation<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown(_) => None,
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    pub fn unimplemented(operation: &'static str) -> Self {
        Self::Unknown(vec![UnknownReason::unimplemented(operation)])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownReason {
    pub kind: UnknownKind,
    /// The entry the reason concerns, when there is one.
    pub path: Option<BytePath>,
    pub detail: String,
}

impl UnknownReason {
    pub fn new(kind: UnknownKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            detail: detail.into(),
        }
    }

    pub fn unimplemented(operation: &'static str) -> Self {
        Self::new(
            UnknownKind::Unimplemented,
            format!("{operation} is not implemented"),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownKind {
    /// An entry, ref, reflog or object could not be read.
    Unreadable,
    /// An index flag the observer does not interpret.
    UnsupportedIndexFlag,
    /// A repository or nested-repository layout the observer does not
    /// interpret.
    UnsupportedLayout,
    /// GWZ evidence (merge/stash records) in an unsupported or malformed
    /// format.
    UnsupportedEvidence,
    /// A configured resource limit was reached before the observation
    /// completed.
    LimitExceeded,
    /// The observer does not implement this observation.
    Unimplemented,
    /// The caller's cancellation port stopped the observation before it
    /// completed (lane H proposal H1, LCM1.0c follow-up 2). Distinct from
    /// [`LimitExceeded`](Self::LimitExceeded): nothing was exhausted, the
    /// caller asked to stop, and repeating the observation may complete.
    Cancelled,
}

/// On-disk unsaved work in one repository, as observed (never as reported
/// by a status command alone).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkObservation {
    pub entries: Vec<WorkEntry>,
    /// Tracked paths whose status is suppressed by an index flag, with the
    /// physically observed state of each.
    pub suppressed: Vec<SuppressedEntry>,
    /// Tracked paths validly absent from the worktree (sparse checkout).
    pub sparse_absent: Vec<BytePath>,
    /// An unfinished native Git operation.
    pub native_operation: Option<NativeOperation>,
    /// Native stash entries present.
    pub stash_entries: u64,
    /// Entries the observer could not establish, each with its path (lane W
    /// proposal W1, LCM1.0c follow-up 2): the observation as a whole is
    /// known -- every other entry is accurately reported -- but these paths
    /// are not. A consumer classifies each as an unknown reason, never as
    /// clean; an observer whose whole inventory failed returns
    /// [`Observation::Unknown`] instead. Empty means every entry was
    /// established.
    pub unknown: Vec<UnknownReason>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkEntry {
    pub path: BytePath,
    pub kind: WorkKind,
    /// Whether the content is binary, when known.
    pub binary: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkKind {
    Staged,
    Unstaged,
    Untracked,
    /// Ignored user data. Ignored does not mean disposable.
    Ignored,
    /// Conflict stages present in the index.
    Conflict,
    ModeChange,
    LinkChange,
    Renamed,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppressedEntry {
    pub path: BytePath,
    pub flag: SuppressionFlag,
    pub physical: PhysicalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressionFlag {
    AssumeUnchanged,
    SkipWorktree,
    Other,
}

/// The physically observed state of a suppressed tracked path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalState {
    MatchesIndex,
    Differs,
    Absent,
    /// The bytes could not be observed; the path is unknown, never clean.
    Unobservable,
}

/// An unfinished native Git operation, as libgit2's `RepositoryState`
/// classifies the repository's on-disk state. The mapping is part of the
/// contract (lane T proposal T-3, pinned in LCM1.0c follow-up 3;
/// `gwz-repo-inspect` implements it):
///
/// | libgit2 state | on disk | variant |
/// |---|---|---|
/// | `Merge` | `MERGE_HEAD` | `Merge` |
/// | `Revert`, `RevertSequence` | `REVERT_HEAD`, `sequencer/` | `Revert` |
/// | `CherryPick`, `CherryPickSequence` | `CHERRY_PICK_HEAD`, `sequencer/` | `CherryPick` |
/// | `Bisect` | `BISECT_LOG` | `Bisect` |
/// | `Rebase`, `RebaseInteractive`, `RebaseMerge` | `rebase-apply/rebasing`, `rebase-merge/interactive`, `rebase-merge/` | `Rebase` |
/// | `ApplyMailbox`, `ApplyMailboxOrRebase` | `rebase-apply/applying`, `rebase-apply/` with neither marker | `ApplyMailbox` |
/// | `Clean` | -- | `None` in [`WorkObservation::native_operation`] |
///
/// `Other` is reserved for a state the inspector cannot classify; libgit2
/// reports none today. Every variant is an open operation to the work
/// classifier (the `open-merge` waiver), so the distinction is diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
    Other,
}

/// Every root whose complete object graph must be preserved elsewhere
/// before a repository may be deleted (design §5.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtectedRoots {
    pub roots: Vec<ProtectedRoot>,
    /// Roots the inventory could not establish, each naming itself (lane I
    /// proposal I-2, LCM1.0c follow-up 3), mirroring
    /// [`WorkObservation::unknown`]: one unreadable ref is reported here by
    /// name and reason while every readable root is still listed in `roots`,
    /// instead of turning the whole inventory into [`Observation::Unknown`].
    /// An inventory with a non-empty `unknown` is **incomplete**: a consumer
    /// that verifies or deletes on the strength of `roots` must treat it
    /// exactly as it treats `Observation::Unknown` (design §5.1: unknown
    /// evidence refuses) -- `gwz-history-check` and `gwz-local-disposal`
    /// carry that obligation, and an inspector switches to per-root reasons
    /// only once they do (checkpoint §12). An observer whose whole inventory
    /// failed (the reference store unreadable) still returns
    /// `Observation::Unknown`. Empty means every root was established.
    pub unknown: Vec<UnknownReason>,
}

impl ProtectedRoots {
    /// Every root was established: nothing is unknown.
    pub fn is_complete(&self) -> bool {
        self.unknown.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedRoot {
    pub source: RootSource,
    pub oid: ObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootSource {
    /// A ref by full name (`refs/heads/main`, `refs/tags/v0` for a
    /// **lightweight** tag, `refs/gwz/local-imports/<id>`), holding the id
    /// the ref points at directly. A `refs/tags/` ref whose direct object is
    /// a tag object is not reported here but as
    /// [`AnnotatedTag`](Self::AnnotatedTag).
    Ref {
        name: String,
    },
    /// `HEAD`, attached or detached.
    Head,
    /// A retained reflog entry of `reference`.
    Reflog {
        reference: String,
        index: u64,
    },
    /// A native stash entry, newest first.
    Stash {
        index: u64,
    },
    /// An annotated tag: a `refs/tags/<name>` ref whose direct object is a
    /// tag object, reported **once**, under this source and at the tag
    /// object's own id -- never also as [`Ref`](Self::Ref) -- because design
    /// §5.1 protects the tag object separately from its target and the target
    /// is an edge of that object, so one root covers both (lane T proposal
    /// T-1 and lane I's finding, ruled in LCM1.0c follow-up 3;
    /// `contract_tests::GraphFixture::tagged` and
    /// `reports_exactly_the_fixture_roots` pin it). `name` is the full ref
    /// name (`refs/tags/v1`).
    AnnotatedTag {
        name: String,
    },
    /// A Git object a GWZ coordination record references (lane H proposal
    /// H3, LCM1.0c follow-up 2): `record` names the record (kind and id,
    /// for example `stash gwz_stash_0007`) and `object` the role the id plays
    /// in it (for example `base`, `index`, `worktree`, `untracked`). Nameable,
    /// so a history check verifies it like any other named root instead of
    /// treating it as [`Other`](Self::Other); as a *witness* root it is
    /// operation state, not a durable retention, and is not eligible.
    ///
    /// How an inspector receives these ids is the implementation's own
    /// constructor seam (`gwz_repo_inspect::LocalRepoInspector::
    /// with_coordination_roots`), not a port method (lane H proposals H-1/H-3
    /// as built by lane I, decided in LCM1.0c follow-up 3): the
    /// [`RepoInspector`] signatures stay context-free, the scripted fake needs
    /// no such input, and every consumer sees only this variant. An inspector
    /// reports them from `inventory_history` and excludes them from
    /// `retained_roots`.
    CoordinationRecord {
        record: String,
        object: String,
    },
    Other {
        detail: String,
    },
}

/// The discriminant of a [`RootSource`], for allowances that name a kind of
/// root rather than one root (lane T proposal T-2, LCM1.0c follow-up 3:
/// `contract_tests::object_reader_conformance_allowing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RootSourceKind {
    Ref,
    Head,
    Reflog,
    Stash,
    AnnotatedTag,
    CoordinationRecord,
    Other,
}

impl RootSource {
    pub fn kind(&self) -> RootSourceKind {
        match self {
            Self::Ref { .. } => RootSourceKind::Ref,
            Self::Head => RootSourceKind::Head,
            Self::Reflog { .. } => RootSourceKind::Reflog,
            Self::Stash { .. } => RootSourceKind::Stash,
            Self::AnnotatedTag { .. } => RootSourceKind::AnnotatedTag,
            Self::CoordinationRecord { .. } => RootSourceKind::CoordinationRecord,
            Self::Other { .. } => RootSourceKind::Other,
        }
    }
}

/// Per-call resource bounds for object reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// Largest object the reader will load; a larger object is
    /// [`ReadError::LimitExceeded`].
    pub max_object_bytes: u64,
}

impl ReadLimits {
    pub const fn new(max_object_bytes: u64) -> Self {
        Self { max_object_bytes }
    }
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Commit,
    Tree,
    Blob,
    Tag,
}

/// One object's identity, kind, size and outgoing edges (commit parents and
/// tree, tree entries, tag target). Blob bytes are never returned; history
/// verification needs edges and presence, not content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRecord {
    pub oid: ObjectId,
    pub kind: ObjectKind,
    pub size: u64,
    pub edges: Vec<ObjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The object is not locally available. Never satisfied by a fetch.
    Missing {
        oid: ObjectId,
    },
    LimitExceeded {
        oid: ObjectId,
        size: u64,
        limit: u64,
    },
    Corrupt {
        oid: ObjectId,
        detail: String,
    },
    ReadFailed {
        detail: String,
    },
    /// This reader does not read objects.
    Unimplemented {
        operation: &'static str,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { oid } => write!(f, "object {oid} is missing"),
            Self::LimitExceeded { oid, size, limit } => {
                write!(
                    f,
                    "object {oid} ({size} bytes) exceeds the read limit {limit}"
                )
            }
            Self::Corrupt { oid, detail } => write!(f, "object {oid} is corrupt: {detail}"),
            Self::ReadFailed { detail } => f.write_str(detail),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Read-only inspection of one repository.
///
/// Call order: `inspect_layout` first; its `RepositoryInfo` is the input to
/// the two observations. Each call is independent and repeatable; none
/// mutates the repository. Resource bounds: one open repository per call.
///
/// **Bounds and cancellation (lane I proposal I-4, recorded in LCM1.0c
/// follow-up 3).** `observe_work` and `inventory_history` take no
/// [`ReadLimits`] and no cancellation port: their inputs are one
/// repository's index, refs and reflogs, which the implementer bounds by
/// construction. LCM2's full work-loss scan (lane D) is the first consumer
/// that needs a bounded, cancellable observation; when it lands this trait
/// gains a bounded method through lane C, rather than a defaulted body now
/// that would ignore its bounds (boundaries §3 forbids permissive defaults).
pub trait RepoInspector {
    fn inspect_layout(&self, path: &Path) -> Result<RepositoryInfo, LayoutError>;
    fn observe_work(&self, repository: &RepositoryInfo) -> Observation<WorkObservation>;
    fn inventory_history(&self, repository: &RepositoryInfo) -> Observation<ProtectedRoots>;
}

/// Bounded read-only object access, normally to one repository's object
/// store. `retained_roots` are the reader's own roots (refs, HEAD, retained
/// reflog entries, stashes) eligible as history witnesses.
pub trait ObjectReader {
    fn retained_roots(&self) -> Result<ProtectedRoots, ReadError>;
    fn read_object(&self, oid: &ObjectId, limits: &ReadLimits) -> Result<ObjectRecord, ReadError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_carry_their_format_and_round_trip_hex() {
        let sha1 = ObjectId::parse_hex(ObjectFormat::Sha1, &"ab".repeat(20)).unwrap();
        assert_eq!(sha1.format(), ObjectFormat::Sha1);
        assert_eq!(sha1.as_bytes().len(), 20);
        assert_eq!(sha1.to_hex(), "ab".repeat(20));
        let sha256 = ObjectId::parse_hex(ObjectFormat::Sha256, &"0f".repeat(32)).unwrap();
        assert_eq!(sha256.format(), ObjectFormat::Sha256);
        assert_ne!(
            sha1,
            ObjectId::parse_hex(ObjectFormat::Sha1, &"ac".repeat(20)).unwrap()
        );
        assert_eq!(format!("{sha1:?}"), format!("Sha1:{}", "ab".repeat(20)));
    }

    #[test]
    fn object_id_length_is_checked_per_format() {
        let error = ObjectId::parse_hex(ObjectFormat::Sha256, &"ab".repeat(20)).unwrap_err();
        assert_eq!(
            error,
            ObjectIdError::Length {
                format: ObjectFormat::Sha256,
                expected: 32,
                actual: 20
            }
        );
        assert!(matches!(
            ObjectId::parse_hex(ObjectFormat::Sha1, &"zz".repeat(20)).unwrap_err(),
            ObjectIdError::Hex { .. }
        ));
        assert!(ObjectId::from_bytes(ObjectFormat::Sha1, &[0u8; 19]).is_err());
        assert!(ObjectId::from_bytes(ObjectFormat::Sha1, &[0u8; 20]).is_ok());
    }

    #[test]
    fn unknown_observations_are_never_known() {
        let observation: Observation<WorkObservation> = Observation::unimplemented("observe_work");
        assert!(observation.is_unknown());
        assert!(observation.known().is_none());
        let Observation::Unknown(reasons) = observation else {
            unreachable!()
        };
        assert_eq!(reasons[0].kind, UnknownKind::Unimplemented);
    }

    /// I-2 (LCM1.0c follow-up 3): a per-root unknown leaves the known roots
    /// in place and marks the inventory incomplete.
    #[test]
    fn protected_roots_are_complete_only_when_nothing_is_unknown() {
        let mut roots = ProtectedRoots::default();
        assert!(roots.is_complete());
        roots.roots.push(ProtectedRoot {
            source: RootSource::Head,
            oid: ObjectId::parse_hex(ObjectFormat::Sha1, &"ab".repeat(20)).unwrap(),
        });
        roots.unknown.push(UnknownReason {
            kind: UnknownKind::Unreadable,
            path: Some(b"refs/heads/broken".to_vec()),
            detail: "a reference file could not be read".to_owned(),
        });
        assert!(!roots.is_complete());
        assert_eq!(roots.roots.len(), 1, "the known roots are still listed");
    }

    /// T-2 (LCM1.0c follow-up 3): every source has exactly one kind.
    #[test]
    fn root_source_kinds_name_every_source() {
        let cases = [
            (
                RootSource::Ref {
                    name: "refs/heads/main".to_owned(),
                },
                RootSourceKind::Ref,
            ),
            (RootSource::Head, RootSourceKind::Head),
            (
                RootSource::Reflog {
                    reference: "HEAD".to_owned(),
                    index: 1,
                },
                RootSourceKind::Reflog,
            ),
            (RootSource::Stash { index: 0 }, RootSourceKind::Stash),
            (
                RootSource::AnnotatedTag {
                    name: "refs/tags/v1".to_owned(),
                },
                RootSourceKind::AnnotatedTag,
            ),
            (
                RootSource::CoordinationRecord {
                    record: "stash gwz_stash_0001".to_owned(),
                    object: "base".to_owned(),
                },
                RootSourceKind::CoordinationRecord,
            ),
            (
                RootSource::Other {
                    detail: "nested".to_owned(),
                },
                RootSourceKind::Other,
            ),
        ];
        for (source, kind) in cases {
            assert_eq!(source.kind(), kind, "{source:?}");
        }
    }

    #[test]
    fn repo_keys_display_root_and_member_ids() {
        assert_eq!(RepoKey::Root.to_string(), "@root");
        assert_eq!(
            RepoKey::Member {
                id: "mem_app".to_owned()
            }
            .to_string(),
            "mem_app"
        );
    }
}
