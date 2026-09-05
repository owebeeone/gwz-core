//! `gwz-repo-inspect`: local Git/filesystem reads (lane I).
//!
//! [`LocalRepoInspector`] implements `gwz_repo_contract::RepoInspector` for
//! one admitted repository path, and [`LocalObjectReader`] implements
//! `gwz_repo_contract::ObjectReader` over that repository's object store,
//! following gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §4: typed object ids in the repository's format, design §4.0 layout
//! hazards (gitfiles, alternates, external common dirs, escaping metadata
//! and configuration including relative hook paths), physical observation
//! of status-suppressed paths, and no implicit fetch, index rewrite, flag
//! clearing or maintenance.
//!
//! LCM1.0c checkpoint state: both types exist with their frozen signatures
//! and refuse typed (`LayoutError::Unimplemented`, `Observation::Unknown`,
//! `ReadError::Unimplemented`). Lane I starts from the contract's
//! conformance suites failing against them on the dev-only fixture crate.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use gwz_repo_contract::{
    LayoutError, ObjectId, ObjectReader, ObjectRecord, Observation, ProtectedRoots, ReadError,
    ReadLimits, RepoInspector, RepositoryInfo, WorkObservation,
};

/// Inspects repositories on the local filesystem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalRepoInspector;

impl LocalRepoInspector {
    pub fn new() -> Self {
        Self
    }
}

impl RepoInspector for LocalRepoInspector {
    fn inspect_layout(&self, _path: &Path) -> Result<RepositoryInfo, LayoutError> {
        Err(LayoutError::Unimplemented {
            operation: "gwz-repo-inspect: LocalRepoInspector::inspect_layout",
        })
    }

    fn observe_work(&self, _repository: &RepositoryInfo) -> Observation<WorkObservation> {
        Observation::unimplemented("gwz-repo-inspect: LocalRepoInspector::observe_work")
    }

    fn inventory_history(&self, _repository: &RepositoryInfo) -> Observation<ProtectedRoots> {
        Observation::unimplemented("gwz-repo-inspect: LocalRepoInspector::inventory_history")
    }
}

/// Bounded reads of one local repository's object store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalObjectReader {
    repository: PathBuf,
}

impl LocalObjectReader {
    /// `repository` is an admitted repository path (from `inspect_layout`).
    pub fn open(repository: &RepositoryInfo) -> Self {
        Self {
            repository: repository.path.clone(),
        }
    }

    pub fn repository(&self) -> &Path {
        &self.repository
    }
}

impl ObjectReader for LocalObjectReader {
    fn retained_roots(&self) -> Result<ProtectedRoots, ReadError> {
        Err(ReadError::Unimplemented {
            operation: "gwz-repo-inspect: LocalObjectReader::retained_roots",
        })
    }

    fn read_object(
        &self,
        _oid: &ObjectId,
        _limits: &ReadLimits,
    ) -> Result<ObjectRecord, ReadError> {
        Err(ReadError::Unimplemented {
            operation: "gwz-repo-inspect: LocalObjectReader::read_object",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::contract_tests::inspector_conformance;
    use gwz_repo_contract::{HeadState, ObjectFormat, UnknownKind};

    #[test]
    fn checkpoint_inspector_refuses_typed_and_never_reports_clean() {
        let inspector = LocalRepoInspector::new();
        inspector_conformance(&inspector, Path::new("/nowhere"), None);
        assert!(matches!(
            inspector.inspect_layout(Path::new("/nowhere")),
            Err(LayoutError::Unimplemented { .. })
        ));
        let info = RepositoryInfo {
            path: PathBuf::from("/repo"),
            git_dir: PathBuf::from("/repo/.git"),
            common_dir: PathBuf::from("/repo/.git"),
            bare: false,
            object_format: ObjectFormat::Sha1,
            head: HeadState::Unborn {
                branch: "main".to_owned(),
            },
        };
        let Observation::Unknown(reasons) = inspector.observe_work(&info) else {
            panic!("unimplemented work observation is unknown, never clean");
        };
        assert_eq!(reasons[0].kind, UnknownKind::Unimplemented);
        assert!(inspector.inventory_history(&info).is_unknown());
        let reader = LocalObjectReader::open(&info);
        assert_eq!(reader.repository(), Path::new("/repo"));
        assert!(matches!(
            reader.retained_roots(),
            Err(ReadError::Unimplemented { .. })
        ));
        let oid = gwz_repo_contract::contract_tests::oid(ObjectFormat::Sha1, 1);
        assert!(matches!(
            reader.read_object(&oid, &ReadLimits::default()),
            Err(ReadError::Unimplemented { .. })
        ));
    }
}
