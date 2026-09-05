//! A recording, scripted [`RepoBuildPort`] fake.
//!
//! It records every call in order and keeps a small faithful model of what
//! each destination holds, so a test can assert the construction ordering
//! *and* the state a failed construction leaves behind. It is faithful in
//! the ways the port promises: an object or ref exists only if it was
//! planted (or created by a call), `transfer_objects` moves only the
//! planted objects a refspec names, and re-initialising a destination is
//! an error rather than a silent overwrite. It has no removal operation,
//! because the port has none.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gwz_repo_contract::{HeadState, ObjectFormat, ObjectId};

use crate::{BuildError, RepoBuildPort};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildCall {
    RefExists {
        repository: PathBuf,
        name: String,
    },
    ObjectExists {
        repository: PathBuf,
        oid: ObjectId,
    },
    InitRepository {
        destination: PathBuf,
        bare: bool,
    },
    TransferObjects {
        source: PathBuf,
        destination: PathBuf,
        refspecs: Vec<String>,
    },
    SetHead {
        repository: PathBuf,
        branch: String,
        target: ObjectId,
        checkout: bool,
    },
    SetDetachedHead {
        repository: PathBuf,
        target: ObjectId,
        checkout: bool,
    },
    SetOrigin {
        repository: PathBuf,
        url: String,
    },
}

/// What one destination repository holds in the fake.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuiltRepository {
    pub bare: bool,
    pub object_format: Option<ObjectFormat>,
    /// Refspecs handed to `transfer_objects`, in order.
    pub transferred: Vec<String>,
    pub branches: BTreeMap<String, ObjectId>,
    pub head: Option<HeadState>,
    pub origin: Option<String>,
    objects: BTreeSet<ObjectId>,
}

impl BuiltRepository {
    pub fn holds_object(&self, oid: &ObjectId) -> bool {
        self.objects.contains(oid)
    }
}

#[derive(Debug, Default)]
pub struct RecordingBuildPort {
    calls: Vec<BuildCall>,
    refs: BTreeSet<(PathBuf, String)>,
    objects: BTreeSet<(PathBuf, ObjectId)>,
    repositories: BTreeMap<PathBuf, BuiltRepository>,
    failures: Vec<(&'static str, Option<PathBuf>, BuildError)>,
}

impl RecordingBuildPort {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[BuildCall] {
        &self.calls
    }

    /// Every destination the fake was asked to create, in path order.
    pub fn repositories(&self) -> &BTreeMap<PathBuf, BuiltRepository> {
        &self.repositories
    }

    pub fn repository(&self, path: impl AsRef<Path>) -> Option<&BuiltRepository> {
        self.repositories.get(path.as_ref())
    }

    /// Plant a ref (a full ref name) in a repository.
    pub fn set_ref(&mut self, repository: impl Into<PathBuf>, name: &str) {
        self.refs.insert((repository.into(), name.to_owned()));
    }

    /// Plant an object in a repository's store.
    pub fn set_object(&mut self, repository: impl Into<PathBuf>, oid: ObjectId) {
        self.objects.insert((repository.into(), oid));
    }

    /// Fail the next call of `operation` (`ref_exists`, `object_exists`,
    /// `init_repository`, `transfer_objects`, `set_head`,
    /// `set_detached_head`, `set_origin`).
    pub fn fail_next(&mut self, operation: &'static str, error: BuildError) {
        self.failures.push((operation, None, error));
    }

    /// Fail the next call of `operation` that names `path`: the repository
    /// for every operation but `transfer_objects`, whose destination it is.
    pub fn fail_next_for(
        &mut self,
        operation: &'static str,
        path: impl Into<PathBuf>,
        error: BuildError,
    ) {
        self.failures.push((operation, Some(path.into()), error));
    }

    fn take_failure(&mut self, operation: &str, path: &Path) -> Option<BuildError> {
        let index = self.failures.iter().position(|(name, at, _)| {
            *name == operation && at.as_ref().is_none_or(|at| at == path)
        })?;
        Some(self.failures.remove(index).2)
    }

    fn repository_mut(&mut self, path: &Path) -> Result<&mut BuiltRepository, BuildError> {
        self.repositories
            .get_mut(path)
            .ok_or_else(|| BuildError::Repository {
                path: path.to_path_buf(),
                detail: "the repository was never initialised".to_owned(),
            })
    }
}

impl RepoBuildPort for RecordingBuildPort {
    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, BuildError> {
        self.calls.push(BuildCall::RefExists {
            repository: repository.to_path_buf(),
            name: name.to_owned(),
        });
        if let Some(error) = self.take_failure("ref_exists", repository) {
            return Err(error);
        }
        Ok(self
            .refs
            .contains(&(repository.to_path_buf(), name.to_owned())))
    }

    fn object_exists(&mut self, repository: &Path, oid: &ObjectId) -> Result<bool, BuildError> {
        self.calls.push(BuildCall::ObjectExists {
            repository: repository.to_path_buf(),
            oid: oid.clone(),
        });
        if let Some(error) = self.take_failure("object_exists", repository) {
            return Err(error);
        }
        Ok(self
            .objects
            .contains(&(repository.to_path_buf(), oid.clone())))
    }

    fn init_repository(
        &mut self,
        destination: &Path,
        bare: bool,
        object_format: ObjectFormat,
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::InitRepository {
            destination: destination.to_path_buf(),
            bare,
        });
        if let Some(error) = self.take_failure("init_repository", destination) {
            return Err(error);
        }
        if self.repositories.contains_key(destination) {
            return Err(BuildError::Repository {
                path: destination.to_path_buf(),
                detail: "a repository is already there".to_owned(),
            });
        }
        self.repositories.insert(
            destination.to_path_buf(),
            BuiltRepository {
                bare,
                object_format: Some(object_format),
                ..BuiltRepository::default()
            },
        );
        Ok(())
    }

    fn transfer_objects(
        &mut self,
        source: &Path,
        destination: &Path,
        refspecs: &[String],
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::TransferObjects {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            refspecs: refspecs.to_vec(),
        });
        if let Some(error) = self.take_failure("transfer_objects", destination) {
            return Err(error);
        }
        let transferred: Vec<ObjectId> = self
            .objects
            .iter()
            .filter(|(path, oid)| {
                path == source && refspecs.iter().any(|spec| *spec == oid.to_hex())
            })
            .map(|(_, oid)| oid.clone())
            .collect();
        let repository = self.repository_mut(destination)?;
        repository.transferred.extend(refspecs.iter().cloned());
        repository.objects.extend(transferred);
        Ok(())
    }

    fn set_head(
        &mut self,
        repository: &Path,
        branch: &str,
        target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::SetHead {
            repository: repository.to_path_buf(),
            branch: branch.to_owned(),
            target: target.clone(),
            checkout,
        });
        if let Some(error) = self.take_failure("set_head", repository) {
            return Err(error);
        }
        let state = self.repository_mut(repository)?;
        state.branches.insert(branch.to_owned(), target.clone());
        state.head = Some(HeadState::Attached {
            branch: branch.to_owned(),
            target: target.clone(),
        });
        self.refs
            .insert((repository.to_path_buf(), format!("refs/heads/{branch}")));
        Ok(())
    }

    fn set_detached_head(
        &mut self,
        repository: &Path,
        target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::SetDetachedHead {
            repository: repository.to_path_buf(),
            target: target.clone(),
            checkout,
        });
        if let Some(error) = self.take_failure("set_detached_head", repository) {
            return Err(error);
        }
        self.repository_mut(repository)?.head = Some(HeadState::Detached {
            target: target.clone(),
        });
        Ok(())
    }

    fn set_origin(&mut self, repository: &Path, url: &str) -> Result<(), BuildError> {
        self.calls.push(BuildCall::SetOrigin {
            repository: repository.to_path_buf(),
            url: url.to_owned(),
        });
        if let Some(error) = self.take_failure("set_origin", repository) {
            return Err(error);
        }
        self.repository_mut(repository)?.origin = Some(url.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::contract_tests::oid;

    #[test]
    fn recorded_calls_and_scripted_failures_are_faithful() {
        let mut port = RecordingBuildPort::new();
        port.set_ref("/src", "refs/heads/main");
        port.set_object("/src", oid(ObjectFormat::Sha1, 1));
        assert_eq!(
            port.ref_exists(Path::new("/src"), "refs/heads/main"),
            Ok(true)
        );
        assert_eq!(
            port.ref_exists(Path::new("/src"), "refs/heads/x"),
            Ok(false)
        );
        assert_eq!(
            port.object_exists(Path::new("/src"), &oid(ObjectFormat::Sha1, 1)),
            Ok(true)
        );
        assert_eq!(
            port.object_exists(Path::new("/src"), &oid(ObjectFormat::Sha1, 2)),
            Ok(false)
        );
        port.fail_next(
            "init_repository",
            BuildError::Failed {
                detail: "disk".to_owned(),
            },
        );
        assert!(
            port.init_repository(Path::new("/hub"), true, ObjectFormat::Sha1)
                .is_err()
        );
        assert!(port.repositories().is_empty());
        assert!(
            port.init_repository(Path::new("/hub"), true, ObjectFormat::Sha1)
                .is_ok()
        );
        // A destination is created once.
        assert!(
            port.init_repository(Path::new("/hub"), true, ObjectFormat::Sha1)
                .is_err()
        );
        port.transfer_objects(
            Path::new("/src"),
            Path::new("/hub"),
            &[oid(ObjectFormat::Sha1, 1).to_hex()],
        )
        .unwrap();
        assert!(
            port.repository("/hub")
                .expect("built")
                .holds_object(&oid(ObjectFormat::Sha1, 1))
        );
        port.set_head(
            Path::new("/hub"),
            "lane",
            &oid(ObjectFormat::Sha1, 1),
            false,
        )
        .unwrap();
        assert_eq!(
            port.ref_exists(Path::new("/hub"), "refs/heads/lane"),
            Ok(true)
        );
        assert_eq!(
            port.repository("/hub").expect("built").head,
            Some(HeadState::Attached {
                branch: "lane".to_owned(),
                target: oid(ObjectFormat::Sha1, 1),
            })
        );
        assert_eq!(port.calls().len(), 10);
    }

    #[test]
    fn a_scripted_failure_can_name_the_repository_it_hits() {
        let mut port = RecordingBuildPort::new();
        port.fail_next_for(
            "init_repository",
            "/dest/app",
            BuildError::Failed {
                detail: "app only".to_owned(),
            },
        );
        assert!(
            port.init_repository(Path::new("/dest"), false, ObjectFormat::Sha1)
                .is_ok()
        );
        assert!(
            port.init_repository(Path::new("/dest/app"), false, ObjectFormat::Sha1)
                .is_err()
        );
        // Untouched repositories are never invented, and a call against a
        // repository that was never initialised is an error, not a panic.
        assert!(port.repository("/dest/app").is_none());
        assert!(
            port.set_detached_head(Path::new("/dest/app"), &oid(ObjectFormat::Sha1, 3), true)
                .is_err()
        );
    }
}
