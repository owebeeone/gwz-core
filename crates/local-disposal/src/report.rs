//! What disposal reports back: the effects it applied, the refusal or
//! failure vocabulary, and the two outcome shapes.

use std::fmt;
use std::path::PathBuf;

use gwz_family_model::Refusal;
use gwz_family_store_contract::StoreError;
use gwz_repo_contract::UnknownReason;

use crate::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeEffect {
    PointerRemoved,
    RowDetached,
    RowDisposing,
    DirectoryRemoved,
    RowRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeReport {
    pub effects: Vec<DisposeEffect>,
    /// Every finding the fresh inspection raised under a waiver the
    /// operator named, in inspection order: what the deletion was actually
    /// forced past. It includes a finding that refuses nothing
    /// ([`HazardFinding::refuses`]), so [`required_waivers`] over it names
    /// the waivers that did waive something and any other named waiver
    /// waived nothing. Empty for `--keep` and for the stale-row exit, which
    /// inspect nothing.
    pub waived: Vec<HazardFinding>,
}

/// A failed disposal: the typed cause plus the effects that completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeFailure {
    pub error: DisposeError,
    pub effects: Vec<DisposeEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeError {
    /// Root, cwd, path mismatch, non-ready target, keep+force, unknown row.
    Refused(Refusal),
    /// The row's recorded path resolves to the registering root itself or to
    /// a directory containing it. The original tree is never deleted; `gwz
    /// local disband` retires the family instead (design §5, §8.4).
    RootImmutable,
    TargetContainsCwd {
        target: PathBuf,
    },
    /// The validated target is not the recorded member: a moved root, a
    /// replaced or foreign directory, an interrupted detach, undecodable
    /// metadata, or an observation that reached outside the deletion tree.
    /// **Never forceable** (design §5.2 step 3): a waiver authorises losing
    /// *this member's* known work, not deleting something else.
    PathMismatch {
        /// The validated target path.
        expected: PathBuf,
        /// What was found there instead.
        observed: String,
    },
    /// Hazards present and not waived.
    Hazards(Vec<HazardFinding>),
    /// Evidence or history could not be established.
    Unknown(Vec<UnknownReason>),
    Store(StoreError),
    Port(PortError),
    /// Removal stopped; `remaining` is what is left for manual cleanup.
    RemovalStopped {
        remaining: Vec<PathBuf>,
        detail: String,
    },
    /// **Never returned.** [`dispose`] implements the whole design §5.2
    /// sequence; a port that implements nothing surfaces as
    /// [`Port`](Self::Port)`(`[`PortError::Unimplemented`]`)` instead. The
    /// variant is retained only so a consumer written against the LCM1.0c
    /// stub still compiles; drop that arm and this variant goes with it.
    Unimplemented,
}

impl fmt::Display for DisposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::RootImmutable => f.write_str("root is never disposed; use disband"),
            Self::TargetContainsCwd { target } => {
                write!(f, "{} contains the working directory", target.display())
            }
            Self::PathMismatch { expected, observed } => {
                write!(
                    f,
                    "{} is not the recorded target: {observed}",
                    expected.display()
                )
            }
            Self::Hazards(findings) => write!(f, "unwaived hazards: {findings:?}"),
            Self::Unknown(reasons) => write!(f, "unknown evidence: {reasons:?}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Port(error) => write!(f, "{error}"),
            Self::RemovalStopped { remaining, detail } => {
                write!(
                    f,
                    "removal stopped ({detail}); {} path(s) remain",
                    remaining.len()
                )
            }
            Self::Unimplemented => f.write_str("gwz-local-disposal: dispose is not implemented"),
        }
    }
}

impl std::error::Error for DisposeError {}
