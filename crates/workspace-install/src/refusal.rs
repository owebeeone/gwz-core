//! Why installation refuses or fails: the pre-reservation refusals, the
//! completion faults and the two error shapes reported over them.

use std::fmt;
use std::path::PathBuf;

use gwz_copy_contract::CopyError;
use gwz_family_model::{CloneMode, PointerObservation, Refusal, RemoteNameCollision};
use gwz_family_store_contract::StoreError;
use gwz_repo_contract::{LayoutError, RepoKey};

use crate::*;

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

fn join<T: fmt::Display>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
