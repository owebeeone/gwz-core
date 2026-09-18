//! The ports `dispose` consults: fresh target and repository evidence, the
//! history question and answer, and the removal outcome.

use std::fmt;
use std::path::Path;

use gwz_family_model::TargetObservation;
use gwz_repo_contract::{ProtectedRoots, RepoKey, RepositoryInfo, WorkObservation};
use gwz_work_detector::CopyBaseline;

use crate::*;

/// Fresh evidence for one repository inside the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryEvidence {
    pub key: RepoKey,
    pub info: RepositoryInfo,
    pub work: gwz_repo_contract::Observation<WorkObservation>,
    pub gwz: GwzEvidence,
    pub history: gwz_repo_contract::Observation<ProtectedRoots>,
}

/// Fresh evidence for the whole target tree, including nested repositories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetEvidence {
    /// What stands at the row's recorded path: absent, undecodable, or
    /// present with its family pointer and allocation marker as core read
    /// them. [`gwz_family_model::classify_target`] turns this and the row
    /// into the [`ListState`] disposal acts on, so "validate name, pointer
    /// and path" (design §5.2 step 1) is one pure decision the model owns.
    /// A moved root, a replaced directory or an interrupted detach all reach
    /// disposal here and refuse as [`DisposeError::PathMismatch`].
    pub target: TargetObservation,
    pub repositories: Vec<RepositoryEvidence>,
    /// What the target inherited from the workspace it was copied from,
    /// where the observer could establish it (R2, R3; plan S1.5, S1.6).
    /// `None` is "nothing is established", and every hazard is then the
    /// lane's own -- the classification every gwz before this one made.
    pub copy: Option<CopyWitness>,
    /// Nested repositories or layouts the observer could not interpret.
    pub unknown: Vec<UnknownReason>,
}

/// The copy the target is, per repository, as the observer established it:
/// from the lane's own clone copy record, corroborated against the
/// surviving family (R2), or -- for a lane no record covers -- from the
/// observer's own live comparison with the family (R3).
///
/// It is **evidence, not authority**: the observer clears an entry only
/// when the surviving family still holds it, so a record inside the tree
/// being deleted can never certify a loss on its own. This library reads
/// it and never establishes it: the comparison is core's, because it needs
/// the family's own repositories, and this crate opens nothing.
///
/// It carries the classified baseline rather than the record's raw
/// protected roots: the roots' own question -- whether a witness's
/// identical reflog or stash entry may certify preservation -- is answered
/// by `gwz-history-check`'s `WitnessPolicy::IdenticalCopy` with a whole
/// subgraph proof, which no record can improve on (plan
/// `GwzLaneCleanFixesPlan.md` §8, the S1.4 adjustment). What the baseline
/// adds is the *work* half: the ignored and untracked entries, and the
/// native stash entries, that the copy brought.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyWitness {
    pub repositories: Vec<CopiedRepository>,
}

impl CopyWitness {
    /// The baseline established for `key`, or `None` when the witness says
    /// nothing about that repository.
    pub fn baseline(&self, key: &RepoKey) -> Option<&CopyBaseline> {
        self.repositories
            .iter()
            .find(|repository| &repository.key == key)
            .map(|repository| &repository.baseline)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedRepository {
    pub key: RepoKey,
    pub baseline: CopyBaseline,
}

/// What the history port is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    pub target: RepoKey,
    pub protected: ProtectedRoots,
}

/// The history port's answer, mirroring `gwz-history-check` outcomes
/// without depending on that crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryAnswer {
    Preserved,
    Unpreserved { detail: String },
    Unknown { reasons: Vec<UnknownReason> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    Evidence { detail: String },
    Removal { path: PathBuf, detail: String },
    Unimplemented { operation: &'static str },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evidence { detail } => write!(f, "evidence: {detail}"),
            Self::Removal { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for PortError {}

/// The narrow ports disposal consumes. Core implements them over the
/// repository inspector, decoded GWZ evidence, `gwz-history-check` and
/// ordinary recursive removal that never follows symlinks out of the tree.
pub trait DisposalPorts {
    fn observe_target(&mut self, target: &Path) -> Result<TargetEvidence, PortError>;
    fn check_history(&mut self, query: &HistoryQuery) -> HistoryAnswer;
    /// Remove `target` once; on error, `remaining` lists what is left.
    fn remove_directory(&mut self, target: &Path) -> Result<(), RemovalFailure>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalFailure {
    pub error: PortError,
    pub remaining: Vec<PathBuf>,
}
