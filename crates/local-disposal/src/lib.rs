//! `gwz-local-disposal`: explicit keep, disband and one-shot disposal
//! (lane D).
//!
//! [`dispose`] is the only local-clone service that removes directory
//! contents (gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §8, design §5). Under the family lock it validates an intact ready
//! target, gathers fresh work evidence through [`DisposalPorts`] and
//! classifies it with `gwz-work-detector`, asks the history port whether
//! every protected root is preserved elsewhere, refuses dirty, unpreserved
//! or unknown evidence unless a named [`HazardWaiver`] covers it, writes
//! `disposing` through the store session, then removes the validated
//! directory once. `--keep` detaches the matching row and pointer and
//! retains every file. On any error it stops and reports what remains; there
//! is no rollback or replay. The hazard vocabulary is owned here; core uses
//! it to reject unknown request hazards before any effect.
//!
//! LCM1.0c checkpoint state: types, waivers and the ports are frozen;
//! `dispose` refuses `DisposeError::Unimplemented` before any port or
//! session call.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_family_model::{MemberName, Refusal};
use gwz_family_store_contract::{FamilySession, StoreError};
use gwz_repo_contract::{ProtectedRoots, RepoKey, RepositoryInfo, UnknownReason, WorkObservation};
use gwz_work_detector::{GwzEvidence, Hazard};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// The named hazards an explicit `--force` may waive (design §5.2). Only
/// these spellings exist on the wire; core rejects any other name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HazardWaiver {
    OpenMerge,
    Dirty,
    UnpreservedHistory,
}

impl HazardWaiver {
    pub const ALL: [HazardWaiver; 3] = [Self::OpenMerge, Self::Dirty, Self::UnpreservedHistory];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMerge => "open-merge",
            Self::Dirty => "dirty",
            Self::UnpreservedHistory => "unpreserved-history",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|waiver| waiver.as_str() == name)
    }

    /// Parse a request's hazard list: every name must be known and no name
    /// may repeat. Empty means no force.
    pub fn parse_all(names: &[String]) -> Result<Vec<Self>, UnknownHazard> {
        let mut waivers = Vec::new();
        for name in names {
            let waiver = Self::parse(name).ok_or_else(|| UnknownHazard { name: name.clone() })?;
            if waivers.contains(&waiver) {
                return Err(UnknownHazard {
                    name: format!("{name} (repeated)"),
                });
            }
            waivers.push(waiver);
        }
        Ok(waivers)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownHazard {
    pub name: String,
}

impl fmt::Display for UnknownHazard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown hazard `{}`; known hazards: {}",
            self.name,
            HazardWaiver::ALL
                .iter()
                .map(|waiver| waiver.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for UnknownHazard {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposePolicy {
    /// Detach the row and pointer; retain every file.
    Keep,
    /// One-shot removal after fresh checks, with explicit waivers.
    Delete { waivers: Vec<HazardWaiver> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeRequest {
    pub name: MemberName,
    pub policy: DisposePolicy,
    /// The registering root.
    pub root: PathBuf,
    /// The invoking process's working directory; disposal refuses a target
    /// that contains it.
    pub cwd: PathBuf,
}

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
    pub repositories: Vec<RepositoryEvidence>,
    /// Nested repositories or layouts the observer could not interpret.
    pub unknown: Vec<UnknownReason>,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HazardFinding {
    pub waiver: HazardWaiver,
    pub hazards: Vec<Hazard>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeError {
    /// Root, cwd, path mismatch, non-ready target, keep+force, unknown row.
    Refused(Refusal),
    RootImmutable,
    TargetContainsCwd {
        target: PathBuf,
    },
    PathMismatch {
        expected: PathBuf,
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
                    "{} does not match recorded {observed}",
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

/// A failed disposal: the typed cause plus the effects that completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeFailure {
    pub error: DisposeError,
    pub effects: Vec<DisposeEffect>,
}

/// Keep, or check and remove, one family member.
pub fn dispose(
    request: &DisposeRequest,
    _session: &mut dyn FamilySession,
    _ports: &mut dyn DisposalPorts,
) -> Result<DisposeReport, DisposeFailure> {
    if let DisposePolicy::Delete { waivers } = &request.policy
        && waivers.len()
            != waivers
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
    {
        return Err(DisposeFailure {
            error: DisposeError::Refused(Refusal::InvalidRow {
                name: request.name.clone(),
                detail: "repeated hazard waiver".to_owned(),
            }),
            effects: Vec::new(),
        });
    }
    Err(DisposeFailure {
        error: DisposeError::Unimplemented,
        effects: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingDisposalPorts;
    use gwz_family_model::{AllocationId, FamilyId};
    use gwz_family_store_contract::contract_tests::InMemoryFamilyStore;
    use gwz_family_store_contract::{FamilyLocation, FamilyStore};

    #[test]
    fn hazard_vocabulary_is_exact() {
        assert_eq!(
            HazardWaiver::parse("open-merge"),
            Some(HazardWaiver::OpenMerge)
        );
        assert_eq!(HazardWaiver::parse("dirty"), Some(HazardWaiver::Dirty));
        assert_eq!(
            HazardWaiver::parse("unpreserved-history"),
            Some(HazardWaiver::UnpreservedHistory)
        );
        assert_eq!(HazardWaiver::parse("force"), None);
        assert_eq!(HazardWaiver::parse("Dirty"), None);
        let parsed =
            HazardWaiver::parse_all(&["dirty".to_owned(), "open-merge".to_owned()]).unwrap();
        assert_eq!(parsed, vec![HazardWaiver::Dirty, HazardWaiver::OpenMerge]);
        assert_eq!(HazardWaiver::parse_all(&[]).unwrap(), Vec::new());
        let unknown = HazardWaiver::parse_all(&["true".to_owned()]).unwrap_err();
        assert_eq!(unknown.name, "true");
        assert!(unknown.to_string().contains("unpreserved-history"));
        assert!(HazardWaiver::parse_all(&["dirty".to_owned(), "dirty".to_owned()]).is_err());
    }

    #[test]
    fn checkpoint_dispose_refuses_before_any_port_or_session_call() {
        let store = InMemoryFamilyStore::new();
        let mut session = store.try_lock(&FamilyLocation::new("/root")).unwrap();
        session
            .found(
                FamilyId::new("fam").unwrap(),
                AllocationId::new("alloc").unwrap(),
            )
            .unwrap();
        let before = session.reread().unwrap();
        let mut ports = RecordingDisposalPorts::new();
        for policy in [
            DisposePolicy::Keep,
            DisposePolicy::Delete {
                waivers: vec![HazardWaiver::UnpreservedHistory],
            },
        ] {
            let request = DisposeRequest {
                name: MemberName::parse("A").unwrap(),
                policy,
                root: PathBuf::from("/root"),
                cwd: PathBuf::from("/root"),
            };
            let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
            assert_eq!(failure.error, DisposeError::Unimplemented);
            assert!(failure.effects.is_empty());
        }
        assert!(
            ports.calls().is_empty(),
            "no evidence, history or removal call"
        );
        assert_eq!(session.reread().unwrap(), before, "no family write");
    }
}
