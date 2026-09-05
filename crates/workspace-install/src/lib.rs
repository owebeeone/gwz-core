//! `gwz-workspace-install`: destination construction and installation
//! ordering (lane N).
//!
//! [`install`] composes a local clone destination (gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §8, design §4) in
//! the four ordered steps the design specifies, and the order is the
//! product:
//!
//! 1. **Admit.** Aggregate the name, path, source-layout and
//!    nested-repository checks through [`InstallPorts::snapshot_source`] and
//!    [`InstallPorts::observe_destination`], refusing an overlapping name or
//!    path, a nonempty destination, a destination that is already a
//!    workspace, an unsupported source layout (design §4.0), a verbatim copy
//!    of a source with an open gwz merge (§4.1) and a `-b <branch>` that
//!    already exists at freeze time (§4.2). Nothing is fetched and nothing
//!    is written.
//! 2. **Reserve.** Write the `creating` row through the live `FamilySession`
//!    and allocate the destination directory. The snapshot captured in step
//!    1 is the one in-memory freeze vector for the rest of the invocation.
//! 3. **Build.** Copy with exclusions applied during traversal (verbatim) or
//!    construct clean/bare repositories through the construction port,
//!    install the fresh pointer and allocation marker through the store
//!    session, check the destination against design §4.1's completion column
//!    and §4.0's independence rules, recheck the source observations, then
//!    recapture the destination lock and desired branches.
//! 4. **Publish.** Write the final manifest **last**, then mark the row
//!    `ready`.
//!
//! It never writes family files itself, and an error or a cancellation
//! between steps leaves an incomplete row and a retained directory for
//! inspection: the only thing installation does after a failure is record a
//! diagnostic on the `creating` row. There is no cleanup, no rollback, no
//! resume and no promotion, even when the destination looks complete.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_copy_contract::{
    Cancellation, CopyError, CopyMode, CopyReport, CopyRequest, Exclusion, TreeCopier,
};
use gwz_family_model::{
    AllocationId, CloneMode, FamilyChange, MemberKind, MemberName, MemberPath, MemberRow,
    MemberState, PointerObservation, ROOT_PATH, Refusal, RemoteNameCollision,
    check_allocation_available, check_name_available, check_name_free_of_remotes,
    check_path_available,
};
use gwz_family_store_contract::{FamilySession, StoreError};
use gwz_repo_contract::{LayoutError, ObjectId, RepoKey, RepositoryInfo};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;

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
    fn row(&self) -> MemberRow {
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
    fn copies_the_tree(&self) -> bool {
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
    fn captured(&self, key: &RepoKey) -> bool {
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

/// What stands at the destination, as the port observed it. Installation
/// reads it twice with two rule sets: admission before reservation, and
/// design §4.1's completion column before publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationObservation {
    /// The path exists.
    pub exists: bool,
    /// It exists and holds at least one entry.
    pub nonempty: bool,
    /// It is already a GWZ workspace (a discoverable manifest or `.gwz/`).
    pub is_workspace: bool,
    /// `.gwz/local-family.yml` is present: the path is a family root.
    pub family_index: bool,
    /// The clone pointer, classified against this family and this root.
    pub pointer: PointerObservation,
    /// `.gwz/merge/` is present, empty or not.
    pub merge_store: bool,
    /// Entries from design §4.1's "absent at ready" column that are still
    /// present, as workspace-relative paths.
    pub residual: Vec<PathBuf>,
    /// Why the destination's Git metadata is not independent of the source
    /// (design §4.0 dest-complete). Empty means independent.
    pub dependencies: Vec<String>,
}

impl DestinationObservation {
    /// A path that holds nothing: the destination admission expects it, and
    /// every completion rule refuses it.
    pub fn absent() -> Self {
        Self {
            exists: false,
            nonempty: false,
            is_workspace: false,
            family_index: false,
            pointer: PointerObservation::Absent,
            merge_store: false,
            residual: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// A finished destination that passes every completion rule: a matching
    /// pointer, no index, no merge store, nothing residual and no external
    /// dependency.
    pub fn complete() -> Self {
        Self {
            exists: true,
            nonempty: true,
            is_workspace: true,
            pointer: PointerObservation::Matches,
            ..Self::absent()
        }
    }
}

impl Default for DestinationObservation {
    fn default() -> Self {
        Self::absent()
    }
}

/// What the construction port must build for clean and bare modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionRequest {
    pub mode: CloneMode,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
    pub destination: PathBuf,
}

/// What the configuration port must install. Recapture runs first and the
/// final manifest last; installation calls them in that order rather than
/// asking one port call to keep it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationPlan {
    pub destination: PathBuf,
    pub mode: CloneMode,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
}

/// What recapture did. Design §4.2: the destination lock must be recaptured
/// to the destination HEAD before dest-complete may be true, and generated
/// `gwz.conf/` changes are reported rather than hidden in a commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigurationReport {
    pub lock_recaptured: bool,
    /// Workspace-relative `gwz.conf/` paths this install generated or
    /// changed relative to the frozen commit.
    pub generated_changes: Vec<PathBuf>,
}

/// What publishing the final manifest did. The conf-integrity marker is
/// regenerated for the final manifest and lock bytes; a copied marker
/// vouching for superseded bytes is never accepted (design §4.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestReceipt {
    pub marker_regenerated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallPortError {
    Layout(LayoutError),
    /// The source changed since the snapshot; publication stops.
    Drift {
        detail: String,
    },
    Construction {
        detail: String,
    },
    Configuration {
        detail: String,
    },
    /// The destination could not be allocated or observed.
    Destination {
        path: PathBuf,
        detail: String,
    },
    Unimplemented {
        operation: &'static str,
    },
}

impl fmt::Display for InstallPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "{error}"),
            Self::Drift { detail } => write!(f, "source drift: {detail}"),
            Self::Construction { detail } => write!(f, "construction: {detail}"),
            Self::Configuration { detail } => write!(f, "configuration: {detail}"),
            Self::Destination { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for InstallPortError {}

/// The narrow ports installation consumes. Core implements them over the
/// repository inspector, `gwz-repo-factory` and the sanctioned existing
/// installation helpers; installation never calls those crates directly.
///
/// **Call order.** Installation calls them in exactly this order, and the
/// order is the contract a real adapter may rely on: `snapshot_source`,
/// `observe_destination`, `allocate_destination`, then either the tree
/// copier or `construct_repositories`, then `observe_destination` again,
/// `recheck_source`, `recapture_configuration` and `publish_manifest`.
/// Nothing follows `publish_manifest` but the row's move to `ready`.
pub trait InstallPorts {
    /// Inventory every included repository of `source` (root, members,
    /// nested) and capture HEADs, branches and remotes. Refuses design §4.0
    /// hazards, aggregated, as [`InstallPortError::Layout`].
    fn snapshot_source(&mut self, source: &Path) -> Result<SourceSnapshot, InstallPortError>;

    /// Observe what stands at `destination`. Called before reservation and
    /// again before publication; it never writes.
    fn observe_destination(
        &mut self,
        destination: &Path,
    ) -> Result<DestinationObservation, InstallPortError>;

    /// Create the destination directory (design §4 step 2, "allocate the
    /// destination"). It is empty when this returns, because the copier
    /// admits only a new or empty destination.
    fn allocate_destination(&mut self, destination: &Path) -> Result<(), InstallPortError>;

    /// Build clean or bare repositories at the destination from the
    /// captured vector, creating `-b <branch>` at the frozen commit
    /// (verbatim mode never calls this).
    fn construct_repositories(
        &mut self,
        request: &ConstructionRequest,
    ) -> Result<(), InstallPortError>;

    /// Verify the source still matches `snapshot`.
    fn recheck_source(&mut self, snapshot: &SourceSnapshot) -> Result<(), InstallPortError>;

    /// Recapture the destination lock to the destination HEAD and the
    /// destination's desired branches, before the manifest.
    fn recapture_configuration(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ConfigurationReport, InstallPortError>;

    /// Write the final manifest and regenerate the conf-integrity marker
    /// over the final manifest and lock bytes. Nothing follows it.
    fn publish_manifest(
        &mut self,
        plan: &ConfigurationPlan,
    ) -> Result<ManifestReceipt, InstallPortError>;
}

/// One reason a create is refused before reservation. Refusals aggregate:
/// every check that can be decided from what was observed is reported
/// together (design §4 step 1, §4.0, §4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallRefusal {
    /// The pure model refused the name, path, nesting or allocation.
    Family(Refusal),
    /// A source repository's layout is unsupported (design §4.0).
    SourceLayout(LayoutError),
    /// The clone name is already a Git remote in a source repository.
    NameIsRemote(RemoteNameCollision),
    DestinationNotEmpty {
        destination: PathBuf,
    },
    DestinationIsWorkspace {
        destination: PathBuf,
    },
    /// Verbatim refuses while the source has an open gwz merge (§4.1).
    SourceOpenMerge {
        detail: String,
    },
    /// The freeze vector does not cover every member including the root
    /// (§4.2), so clean and bare have nothing to build the root from.
    RootNotCaptured,
    /// `-b <branch>` already exists in a repository at freeze time (§4.2).
    BranchExists {
        branch: String,
        member: RepoKey,
    },
    /// `-b <branch>` is a clean/bare option; verbatim copies the source's
    /// branches as they sit.
    BranchNotSupported {
        branch: String,
        mode: CloneMode,
    },
}

impl fmt::Display for InstallRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Family(refusal) => write!(f, "{refusal}"),
            Self::SourceLayout(error) => write!(f, "{error}"),
            Self::NameIsRemote(collision) => write!(f, "{collision}"),
            Self::DestinationNotEmpty { destination } => {
                write!(f, "{} is not empty", destination.display())
            }
            Self::DestinationIsWorkspace { destination } => {
                write!(f, "{} is already a workspace", destination.display())
            }
            Self::SourceOpenMerge { detail } => {
                write!(
                    f,
                    "source has an open gwz merge ({detail}); abort it or use --clean"
                )
            }
            Self::RootNotCaptured => {
                f.write_str("the captured freeze vector does not include the root repository")
            }
            Self::BranchExists { branch, member } => {
                write!(f, "branch `{branch}` already exists in {member}")
            }
            Self::BranchNotSupported { branch, mode } => write!(
                f,
                "`-b {branch}` is not supported by --{} clones",
                mode.as_str()
            ),
        }
    }
}

/// Why a built destination is not complete, so the row must not be made
/// ready (design §4.0 dest-complete, §4.1 post-copy check and "at ready"
/// column, §4.2 lock recapture). The directory and the `creating` row are
/// retained for inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionFault {
    /// The destination holds a family index: it is a root, not a clone.
    FamilyIndexPresent,
    /// It holds both an index and a pointer.
    ConflictingMetadata,
    /// No pointer to the registering root.
    PointerMissing,
    /// A pointer that does not name this family and this root.
    PointerInvalid { observed: PointerObservation },
    /// `.gwz/merge/` is present, including as an empty directory.
    MergeStorePresent,
    /// An entry design §4.1 requires absent at ready is still present.
    ResidualPath { path: PathBuf },
    /// The destination's Git metadata still depends on something outside it.
    NotIndependent { detail: String },
    /// The destination lock was not recaptured to the destination HEAD.
    LockNotRecaptured,
    /// The conf-integrity marker was not regenerated over the final bytes.
    MarkerNotRegenerated,
}

impl fmt::Display for CompletionFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FamilyIndexPresent => f.write_str("the destination holds a family index"),
            Self::ConflictingMetadata => {
                f.write_str("the destination holds both a family index and a family pointer")
            }
            Self::PointerMissing => f.write_str("the destination has no family pointer"),
            Self::PointerInvalid { observed } => {
                write!(f, "the destination's family pointer is {observed:?}")
            }
            Self::MergeStorePresent => f.write_str("the destination has a `.gwz/merge/` store"),
            Self::ResidualPath { path } => {
                write!(f, "{} must be absent at ready", path.display())
            }
            Self::NotIndependent { detail } => write!(f, "not independent: {detail}"),
            Self::LockNotRecaptured => {
                f.write_str("the destination lock was not recaptured to the destination HEAD")
            }
            Self::MarkerNotRegenerated => f.write_str(
                "the conf-integrity marker was not regenerated for the final manifest and lock \
                 bytes",
            ),
        }
    }
}

/// The ordered step a failure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStep {
    InventorySource,
    ObserveDestination,
    Reserve,
    AllocateDestination,
    CopyTree,
    ConstructRepositories,
    InstallPointer,
    CheckDestination,
    RecheckSource,
    RecaptureConfiguration,
    PublishManifest,
    MarkReady,
}

impl InstallStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InventorySource => "inventory source",
            Self::ObserveDestination => "observe destination",
            Self::Reserve => "reserve",
            Self::AllocateDestination => "allocate destination",
            Self::CopyTree => "copy tree",
            Self::ConstructRepositories => "construct repositories",
            Self::InstallPointer => "install pointer",
            Self::CheckDestination => "check destination",
            Self::RecheckSource => "recheck source",
            Self::RecaptureConfiguration => "recapture configuration",
            Self::PublishManifest => "publish manifest",
            Self::MarkReady => "mark ready",
        }
    }
}

impl fmt::Display for InstallStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One completed installation effect, in the order installation performs
/// them. A failure reports the prefix that actually happened; none of them
/// is undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallEffect {
    RowAllocated,
    DestinationAllocated,
    TreeCopied,
    RepositoriesConstructed,
    /// The allocation marker and then the pointer, as the store writes them.
    PointerInstalled,
    ConfigurationInstalled,
    ManifestPublished,
    /// A diagnostic was recorded on the retained `creating` row.
    ErrorRecorded,
    RowReady,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReport {
    pub copy: Option<CopyReport>,
    /// Generated `gwz.conf/` changes and lock recapture, reported rather
    /// than hidden in a commit (design §4.2).
    pub configuration: Option<ConfigurationReport>,
    pub effects: Vec<InstallEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Refused before reservation, aggregating every decided check.
    Refused(Vec<InstallRefusal>),
    Source(Box<InstallPortError>),
    Copy(Box<CopyError>),
    Store(Box<StoreError>),
    Port(Box<InstallPortError>),
    /// The destination was built but is not complete; the row stays
    /// `creating` and nothing is cleaned up.
    Incomplete(Vec<CompletionFault>),
    Cancelled,
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusals) => write!(f, "refused: {}", join(refusals)),
            Self::Source(error) | Self::Port(error) => write!(f, "{error}"),
            Self::Copy(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Incomplete(faults) => write!(f, "destination is incomplete: {}", join(faults)),
            Self::Cancelled => f.write_str("install cancelled"),
        }
    }
}

fn join<T: fmt::Display>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl std::error::Error for InstallError {}

/// A failed installation: the step that stopped, the typed cause and the
/// effects that completed. The row stays incomplete and the directory is
/// retained; nothing is rolled back, resumed or promoted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallFailure {
    pub step: InstallStep,
    pub error: InstallError,
    pub effects: Vec<InstallEffect>,
}

impl fmt::Display for InstallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed: {}", self.step, self.error)
    }
}

impl std::error::Error for InstallFailure {}

/// What has already happened, so a failure can report it.
#[derive(Debug, Default)]
struct Progress {
    effects: Vec<InstallEffect>,
    copy: Option<CopyReport>,
    configuration: Option<ConfigurationReport>,
}

impl Progress {
    fn did(&mut self, effect: InstallEffect) {
        self.effects.push(effect);
    }

    /// Stop. The `creating` row keeps its reservation and gains a
    /// diagnostic (design §3's `last_error`, §4's "diagnostic row"); that
    /// best-effort write is the only thing a failure does, it undoes
    /// nothing, and its own failure never masks `error`.
    fn stop(
        mut self,
        session: &mut dyn FamilySession,
        name: &MemberName,
        step: InstallStep,
        error: InstallError,
    ) -> InstallFailure {
        if self.effects.contains(&InstallEffect::RowAllocated)
            && !self.effects.contains(&InstallEffect::RowReady)
            && session
                .apply(&FamilyChange::RecordError {
                    name: name.clone(),
                    last_error: format!("{step}: {error}"),
                })
                .is_ok()
        {
            self.effects.push(InstallEffect::ErrorRecorded);
        }
        InstallFailure {
            step,
            error,
            effects: self.effects,
        }
    }
}

/// Install one local clone destination.
pub fn install(
    request: &InstallRequest,
    session: &mut dyn FamilySession,
    copier: &dyn TreeCopier,
    ports: &mut dyn InstallPorts,
    cancellation: &dyn Cancellation,
) -> Result<InstallReport, InstallFailure> {
    let mut progress = Progress::default();

    // Step 1: admit or refuse, before anything is written.
    let snapshot = match admit(request, session, ports) {
        Ok(snapshot) => snapshot,
        Err((step, error)) => return Err(progress.stop(session, &request.name, step, error)),
    };

    // Step 2: reserve the row, then allocate the destination directory.
    macro_rules! attempt {
        ($step:expr, $result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error) => return Err(progress.stop(session, &request.name, $step, error)),
            }
        };
    }
    macro_rules! checkpoint {
        ($step:expr) => {
            if cancellation.is_cancelled() {
                return Err(progress.stop(session, &request.name, $step, InstallError::Cancelled));
            }
        };
    }

    checkpoint!(InstallStep::Reserve);
    attempt!(
        InstallStep::Reserve,
        session
            .apply(&FamilyChange::Allocate {
                name: request.name.clone(),
                row: request.row(),
            })
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::RowAllocated);

    attempt!(
        InstallStep::AllocateDestination,
        ports
            .allocate_destination(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.did(InstallEffect::DestinationAllocated);

    // Step 3: build the destination, install its metadata, check it, and
    // recheck the source before anything is published.
    if request.copies_the_tree() {
        checkpoint!(InstallStep::CopyTree);
        let report = attempt!(
            InstallStep::CopyTree,
            copier
                .copy_tree(&copy_request(request), cancellation)
                .map_err(|error| InstallError::Copy(Box::new(error)))
        );
        progress.copy = Some(report);
        progress.did(InstallEffect::TreeCopied);
    } else {
        checkpoint!(InstallStep::ConstructRepositories);
        attempt!(
            InstallStep::ConstructRepositories,
            ports
                .construct_repositories(&ConstructionRequest {
                    mode: request.mode,
                    branch: request.branch.clone(),
                    snapshot: snapshot.clone(),
                    destination: request.destination.clone(),
                })
                .map_err(|error| InstallError::Port(Box::new(error)))
        );
        progress.did(InstallEffect::RepositoriesConstructed);
    }

    attempt!(
        InstallStep::InstallPointer,
        session
            .install_pointer(&request.name, &request.destination)
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::PointerInstalled);

    let observed = attempt!(
        InstallStep::CheckDestination,
        ports
            .observe_destination(&request.destination)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    let faults = completion_faults(&observed);
    if !faults.is_empty() {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::CheckDestination,
            InstallError::Incomplete(faults),
        ));
    }

    attempt!(
        InstallStep::RecheckSource,
        ports
            .recheck_source(&snapshot)
            .map_err(|error| InstallError::Source(Box::new(error)))
    );

    let plan = ConfigurationPlan {
        destination: request.destination.clone(),
        mode: request.mode,
        branch: request.branch.clone(),
        snapshot,
    };
    let configuration = attempt!(
        InstallStep::RecaptureConfiguration,
        ports
            .recapture_configuration(&plan)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    let recapture_required = request.mode != CloneMode::Verbatim;
    let recaptured = configuration.lock_recaptured;
    progress.configuration = Some(configuration);
    progress.did(InstallEffect::ConfigurationInstalled);
    if recapture_required && !recaptured {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::RecaptureConfiguration,
            InstallError::Incomplete(vec![CompletionFault::LockNotRecaptured]),
        ));
    }

    // Step 4: the final manifest last, then ready.
    checkpoint!(InstallStep::PublishManifest);
    let receipt = attempt!(
        InstallStep::PublishManifest,
        ports
            .publish_manifest(&plan)
            .map_err(|error| InstallError::Port(Box::new(error)))
    );
    progress.did(InstallEffect::ManifestPublished);
    if !receipt.marker_regenerated {
        return Err(progress.stop(
            session,
            &request.name,
            InstallStep::PublishManifest,
            InstallError::Incomplete(vec![CompletionFault::MarkerNotRegenerated]),
        ));
    }

    attempt!(
        InstallStep::MarkReady,
        session
            .apply(&FamilyChange::MarkReady {
                name: request.name.clone(),
                expected_allocation: request.allocation.clone(),
            })
            .map_err(|error| InstallError::Store(Box::new(error)))
    );
    progress.did(InstallEffect::RowReady);

    Ok(InstallReport {
        copy: progress.copy,
        configuration: progress.configuration,
        effects: progress.effects,
    })
}

fn copy_request(request: &InstallRequest) -> CopyRequest {
    CopyRequest {
        source: request.source.clone(),
        destination: request.destination.clone(),
        exclusions: request.exclusions.clone(),
        mode: request.copy_mode,
    }
}

/// Step 1. Every check that can be decided is decided, and the refusals are
/// reported together. A port error is not a refusal: it stops here, with no
/// effects, rather than presenting a partial aggregate as the reasons.
fn admit(
    request: &InstallRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn InstallPorts,
) -> Result<SourceSnapshot, (InstallStep, InstallError)> {
    let mut refusals = Vec::new();

    let snapshot = match ports.snapshot_source(&request.source) {
        Ok(snapshot) => Some(snapshot),
        Err(InstallPortError::Layout(error)) => {
            refusals.push(InstallRefusal::SourceLayout(error));
            None
        }
        Err(error) => {
            return Err((
                InstallStep::InventorySource,
                InstallError::Source(Box::new(error)),
            ));
        }
    };

    let observed = ports
        .observe_destination(&request.destination)
        .map_err(|error| {
            (
                InstallStep::ObserveDestination,
                InstallError::Port(Box::new(error)),
            )
        })?;
    if observed.nonempty {
        refusals.push(InstallRefusal::DestinationNotEmpty {
            destination: request.destination.clone(),
        });
    }
    if observed.is_workspace {
        refusals.push(InstallRefusal::DestinationIsWorkspace {
            destination: request.destination.clone(),
        });
    }

    match session.reread() {
        // No index yet: `Allocate` refuses `NoFamily` on its own, and the
        // model has nothing to compare against here.
        Ok(None) => {}
        Ok(Some(view)) => {
            for refusal in [
                check_name_available(&view, &request.name),
                check_path_available(&view, &request.path),
                check_allocation_available(&view, &request.name, &request.allocation),
            ]
            .into_iter()
            .filter_map(Result::err)
            {
                refusals.push(InstallRefusal::Family(refusal));
            }
        }
        Err(error) => return Err((InstallStep::Reserve, InstallError::Store(Box::new(error)))),
    }

    if let Some(snapshot) = &snapshot {
        refusals.extend(source_refusals(request, snapshot));
    }

    let Some(snapshot) = snapshot else {
        // Only a source-layout refusal returns no snapshot, so `refusals`
        // is non-empty here.
        return Err((
            InstallStep::InventorySource,
            InstallError::Refused(refusals),
        ));
    };
    if !refusals.is_empty() {
        return Err((InstallStep::Reserve, InstallError::Refused(refusals)));
    }
    Ok(snapshot)
}

/// The checks the captured source answers, all before the `creating` row.
fn source_refusals(request: &InstallRequest, snapshot: &SourceSnapshot) -> Vec<InstallRefusal> {
    let mut refusals = Vec::new();
    let remotes: Vec<(String, String)> = snapshot
        .repositories
        .iter()
        .flat_map(|repo| {
            repo.remotes
                .iter()
                .map(|remote| (repo.key.to_string(), remote.clone()))
        })
        .collect();
    if let Err(collision) = check_name_free_of_remotes(
        &request.name,
        remotes
            .iter()
            .map(|(member, remote)| (member.as_str(), remote.as_str())),
    ) {
        refusals.push(InstallRefusal::NameIsRemote(collision));
    }

    if request.mode == CloneMode::Verbatim
        && let Some(detail) = &snapshot.open_gwz_merge
    {
        refusals.push(InstallRefusal::SourceOpenMerge {
            detail: detail.clone(),
        });
    }

    if !request.copies_the_tree() && !snapshot.captured(&RepoKey::Root) {
        refusals.push(InstallRefusal::RootNotCaptured);
    }

    if let Some(branch) = &request.branch {
        if request.copies_the_tree() {
            refusals.push(InstallRefusal::BranchNotSupported {
                branch: branch.clone(),
                mode: request.mode,
            });
        } else {
            for repo in &snapshot.repositories {
                if repo.branches.iter().any(|existing| existing == branch) {
                    refusals.push(InstallRefusal::BranchExists {
                        branch: branch.clone(),
                        member: repo.key.clone(),
                    });
                }
            }
        }
    }
    refusals
}

/// Design §4.1's post-copy check and "at ready" column, plus §4.0's
/// dest-complete independence rule, applied to what the port observed.
fn completion_faults(observed: &DestinationObservation) -> Vec<CompletionFault> {
    let mut faults = Vec::new();
    match (observed.family_index, observed.pointer) {
        (true, PointerObservation::Absent) => faults.push(CompletionFault::FamilyIndexPresent),
        (true, _) => faults.push(CompletionFault::ConflictingMetadata),
        (false, PointerObservation::Matches) => {}
        (false, PointerObservation::Absent) => faults.push(CompletionFault::PointerMissing),
        (false, observed) => faults.push(CompletionFault::PointerInvalid { observed }),
    }
    if observed.merge_store {
        faults.push(CompletionFault::MergeStorePresent);
    }
    for path in &observed.residual {
        faults.push(CompletionFault::ResidualPath { path: path.clone() });
    }
    for detail in &observed.dependencies {
        faults.push(CompletionFault::NotIndependent {
            detail: detail.clone(),
        });
    }
    faults
}
