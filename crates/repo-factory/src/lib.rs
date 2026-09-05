//! `gwz-repo-factory`: independent bare/clean repository construction
//! (lane B).
//!
//! [`construct`] builds destination repositories from one captured vector
//! (HEAD/branch per repository) through the [`RepoBuildPort`] it owns:
//! bare hubs (LCM2.3) and clean checkouts with optional branch creation
//! (LCM3.1), following gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §8 and design
//! §4.2/§4.3. Every selected branch and ref is checked in every repository
//! before anything is allocated; destinations borrow no object store, use
//! no implicit network, make no hidden commit and write no family metadata.
//!
//! LCM1.0c checkpoint state: types and the port are frozen; `construct`
//! refuses `FactoryError::Unimplemented` before any port call.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{ObjectFormat, ObjectId, RepoKey};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactoryMode {
    /// Clean worktree checkout at the captured commit; `branch` creates and
    /// attaches that branch in every repository.
    Clean { branch: Option<String> },
    /// Bare repository (`core.bare=true`), no worktree, HEAD unchanged.
    Bare,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRepo {
    pub key: RepoKey,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub object_format: ObjectFormat,
    /// The frozen commit every destination uses.
    pub head: ObjectId,
    /// The source's attached branch, when any.
    pub branch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactoryRequest {
    pub mode: FactoryMode,
    pub vector: Vec<CapturedRepo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactoryReport {
    pub built: Vec<RepoKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    Repository { path: PathBuf, detail: String },
    Failed { detail: String },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Failed { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for BuildError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactoryError {
    InvalidRequest {
        detail: String,
    },
    /// `-b` names a branch that already exists in a source; refused before
    /// any allocation (aggregating).
    BranchExists {
        conflicts: Vec<(RepoKey, String)>,
    },
    RefMissing {
        key: RepoKey,
        name: String,
    },
    Build {
        key: RepoKey,
        error: BuildError,
        /// Repositories fully built before the failure; retained.
        built: Vec<RepoKey>,
    },
    Unimplemented,
}

impl fmt::Display for FactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { detail } => write!(f, "invalid factory request: {detail}"),
            Self::BranchExists { conflicts } => write!(f, "branch already exists: {conflicts:?}"),
            Self::RefMissing { key, name } => write!(f, "{key}: ref {name} is missing"),
            Self::Build { key, error, .. } => write!(f, "{key}: {error}"),
            Self::Unimplemented => f.write_str("gwz-repo-factory: construct is not implemented"),
        }
    }
}

impl std::error::Error for FactoryError {}

/// The repository-builder port. Core implements it over the Git backend.
pub trait RepoBuildPort {
    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, BuildError>;

    /// Create an empty repository with its own object store.
    fn init_repository(
        &mut self,
        destination: &Path,
        bare: bool,
        object_format: ObjectFormat,
    ) -> Result<(), BuildError>;

    /// Copy objects reachable from `refspecs` from `source` into
    /// `destination` without borrowing the source store.
    fn transfer_objects(
        &mut self,
        source: &Path,
        destination: &Path,
        refspecs: &[String],
    ) -> Result<(), BuildError>;

    /// Point `branch` at `target` and attach HEAD to it; `checkout`
    /// materializes the worktree for clean mode.
    fn set_head(
        &mut self,
        repository: &Path,
        branch: &str,
        target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError>;
}

/// Build every repository of the captured vector.
pub fn construct(
    request: &FactoryRequest,
    _port: &mut dyn RepoBuildPort,
) -> Result<FactoryReport, FactoryError> {
    if request.vector.is_empty() {
        return Err(FactoryError::InvalidRequest {
            detail: "a factory request needs at least one repository".to_owned(),
        });
    }
    Err(FactoryError::Unimplemented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingBuildPort;
    use gwz_repo_contract::contract_tests::oid;

    #[test]
    fn checkpoint_factory_refuses_before_any_port_call() {
        let mut port = RecordingBuildPort::new();
        let request = FactoryRequest {
            mode: FactoryMode::Bare,
            vector: vec![CapturedRepo {
                key: RepoKey::Root,
                source: PathBuf::from("/src"),
                destination: PathBuf::from("/hub"),
                object_format: ObjectFormat::Sha1,
                head: oid(ObjectFormat::Sha1, 7),
                branch: Some("main".to_owned()),
            }],
        };
        assert_eq!(
            construct(&request, &mut port).unwrap_err(),
            FactoryError::Unimplemented
        );
        assert!(port.calls().is_empty());
        let empty = FactoryRequest {
            mode: FactoryMode::Clean { branch: None },
            vector: Vec::new(),
        };
        assert!(matches!(
            construct(&empty, &mut port).unwrap_err(),
            FactoryError::InvalidRequest { .. }
        ));
        assert!(port.calls().is_empty());
    }
}
