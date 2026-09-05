//! `gwz-workspace-install`: destination construction and installation
//! ordering (lane N).
//!
//! [`install`] composes a local clone destination (gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §8, design §4):
//! inspect and recheck the source through [`InstallPorts`], reserve the
//! row through the live `FamilySession`, copy through the `TreeCopier` (or
//! construct clean/bare repositories through the construction port), install
//! fresh destination metadata (pointer and marker through the store
//! session, `gwz.conf/` through the configuration port), publish the final
//! manifest last, then mark the row ready. It never writes family files
//! itself, and an error between steps leaves an incomplete row and a
//! retained directory for inspection; there is no automatic promotion or
//! cleanup.
//!
//! LCM1.0c checkpoint state: types and the ports are frozen; `install`
//! refuses `InstallError::Unimplemented` before any port, copier or session
//! call.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_copy_contract::{Cancellation, CopyError, CopyReport, Exclusion, TreeCopier};
use gwz_family_model::{CloneMode, MemberName, Refusal};
use gwz_family_store_contract::{FamilySession, StoreError};
use gwz_repo_contract::{LayoutError, ObjectId, RepoKey, RepositoryInfo};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest {
    pub name: MemberName,
    /// The registering root of the family (the index holder).
    pub root: PathBuf,
    /// The workspace being cloned (root or a ready clone).
    pub source: PathBuf,
    pub destination: PathBuf,
    pub mode: CloneMode,
    /// `-b <branch>` for clean/bare modes.
    pub branch: Option<String>,
    /// Design §4.1 exclusions, resolved by core (family files, catalog,
    /// merge store, locks, stash bundles, `.git/worktrees`).
    pub exclusions: Vec<Exclusion>,
}

/// Source state captured once before reservation and rechecked before
/// publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub repositories: Vec<CapturedRepository>,
    /// Digest of the source manifest and lock bytes.
    pub configuration_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRepository {
    pub key: RepoKey,
    pub info: RepositoryInfo,
    pub head: Option<ObjectId>,
}

/// What the construction port must build for clean/bare modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionRequest {
    pub mode: CloneMode,
    pub branch: Option<String>,
    pub snapshot: SourceSnapshot,
    pub destination: PathBuf,
}

/// What the configuration port must install, in order: lock recapture
/// first, the final manifest last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationPlan {
    pub destination: PathBuf,
    pub mode: CloneMode,
    pub snapshot: SourceSnapshot,
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
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for InstallPortError {}

/// The narrow ports installation consumes. Core implements them over the
/// repository inspector, `gwz-repo-factory` and the sanctioned existing
/// installation helpers; installation never calls those crates directly.
pub trait InstallPorts {
    /// Inventory every included repository of `source` (root, members,
    /// nested) and capture HEADs. Refuses design §4.0 hazards.
    fn snapshot_source(&mut self, source: &Path) -> Result<SourceSnapshot, InstallPortError>;

    /// Verify the source still matches `snapshot`.
    fn recheck_source(&mut self, snapshot: &SourceSnapshot) -> Result<(), InstallPortError>;

    /// Build clean or bare repositories at the destination from the
    /// captured vector (verbatim mode never calls this).
    fn construct_repositories(
        &mut self,
        request: &ConstructionRequest,
    ) -> Result<(), InstallPortError>;

    /// Install `gwz.conf/` at the destination through the existing helpers,
    /// manifest last.
    fn install_configuration(&mut self, plan: &ConfigurationPlan) -> Result<(), InstallPortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallEffect {
    RowAllocated,
    TreeCopied,
    RepositoriesConstructed,
    PointerInstalled,
    ConfigurationInstalled,
    RowReady,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReport {
    pub copy: Option<CopyReport>,
    pub effects: Vec<InstallEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The model refused the reservation (name, path or nesting).
    Refused(Box<Refusal>),
    Source(Box<InstallPortError>),
    Copy(Box<CopyError>),
    Store(Box<StoreError>),
    Port(Box<InstallPortError>),
    Cancelled,
    Unimplemented,
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Source(error) | Self::Port(error) => write!(f, "{error}"),
            Self::Copy(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Cancelled => f.write_str("install cancelled"),
            Self::Unimplemented => f.write_str("gwz-workspace-install: install is not implemented"),
        }
    }
}

impl std::error::Error for InstallError {}

/// A failed installation: the typed cause plus the effects that completed.
/// The row stays incomplete and the directory is retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallFailure {
    pub error: InstallError,
    pub effects: Vec<InstallEffect>,
}

/// Install one local clone destination.
pub fn install(
    _request: &InstallRequest,
    _session: &mut dyn FamilySession,
    _copier: &dyn TreeCopier,
    _ports: &mut dyn InstallPorts,
    _cancellation: &dyn Cancellation,
) -> Result<InstallReport, InstallFailure> {
    Err(InstallFailure {
        error: InstallError::Unimplemented,
        effects: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingInstallPorts;
    use gwz_copy_contract::NeverCancelled;
    use gwz_copy_contract::contract_tests::ScriptedTreeCopier;
    use gwz_family_model::{AllocationId, FamilyId};
    use gwz_family_store_contract::FamilyLocation;
    use gwz_family_store_contract::FamilyStore;
    use gwz_family_store_contract::contract_tests::InMemoryFamilyStore;

    #[test]
    fn checkpoint_install_refuses_before_any_port_copier_or_session_call() {
        let store = InMemoryFamilyStore::new();
        let mut session = store.try_lock(&FamilyLocation::new("/root")).unwrap();
        session
            .found(
                FamilyId::new("fam").unwrap(),
                AllocationId::new("alloc").unwrap(),
            )
            .unwrap();
        let before = session.reread().unwrap();
        let copier = ScriptedTreeCopier::new();
        let mut ports = RecordingInstallPorts::new();
        let request = InstallRequest {
            name: MemberName::parse("A").unwrap(),
            root: PathBuf::from("/root"),
            source: PathBuf::from("/root"),
            destination: PathBuf::from("/ws-A"),
            mode: CloneMode::Verbatim,
            branch: None,
            exclusions: Vec::new(),
        };
        let failure = install(&request, &mut session, &copier, &mut ports, &NeverCancelled)
            .expect_err("the checkpoint installer refuses");
        assert_eq!(failure.error, InstallError::Unimplemented);
        assert!(failure.effects.is_empty());
        assert!(copier.calls().is_empty(), "no copy");
        assert!(ports.calls().is_empty(), "no port call");
        assert_eq!(session.reread().unwrap(), before, "no family write");
    }
}
