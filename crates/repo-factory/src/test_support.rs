//! A recording, scripted [`RepoBuildPort`] fake.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{ObjectFormat, ObjectId};

use crate::{BuildError, RepoBuildPort};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildCall {
    RefExists {
        repository: PathBuf,
        name: String,
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
        checkout: bool,
    },
}

#[derive(Debug, Default)]
pub struct RecordingBuildPort {
    calls: Vec<BuildCall>,
    refs: BTreeSet<(PathBuf, String)>,
    failures: Vec<(&'static str, BuildError)>,
}

impl RecordingBuildPort {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[BuildCall] {
        &self.calls
    }

    pub fn set_ref(&mut self, repository: impl Into<PathBuf>, name: &str) {
        self.refs.insert((repository.into(), name.to_owned()));
    }

    /// Fail the next call of `operation` (`ref_exists`, `init_repository`,
    /// `transfer_objects`, `set_head`).
    pub fn fail_next(&mut self, operation: &'static str, error: BuildError) {
        self.failures.push((operation, error));
    }

    fn take_failure(&mut self, operation: &str) -> Option<BuildError> {
        let index = self
            .failures
            .iter()
            .position(|(name, _)| *name == operation)?;
        Some(self.failures.remove(index).1)
    }
}

impl RepoBuildPort for RecordingBuildPort {
    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, BuildError> {
        self.calls.push(BuildCall::RefExists {
            repository: repository.to_path_buf(),
            name: name.to_owned(),
        });
        if let Some(error) = self.take_failure("ref_exists") {
            return Err(error);
        }
        Ok(self
            .refs
            .contains(&(repository.to_path_buf(), name.to_owned())))
    }

    fn init_repository(
        &mut self,
        destination: &Path,
        bare: bool,
        _object_format: ObjectFormat,
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::InitRepository {
            destination: destination.to_path_buf(),
            bare,
        });
        self.take_failure("init_repository").map_or(Ok(()), Err)
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
        self.take_failure("transfer_objects").map_or(Ok(()), Err)
    }

    fn set_head(
        &mut self,
        repository: &Path,
        branch: &str,
        _target: &ObjectId,
        checkout: bool,
    ) -> Result<(), BuildError> {
        self.calls.push(BuildCall::SetHead {
            repository: repository.to_path_buf(),
            branch: branch.to_owned(),
            checkout,
        });
        if let Some(error) = self.take_failure("set_head") {
            return Err(error);
        }
        self.refs
            .insert((repository.to_path_buf(), format!("refs/heads/{branch}")));
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
        assert_eq!(
            port.ref_exists(Path::new("/src"), "refs/heads/main"),
            Ok(true)
        );
        assert_eq!(
            port.ref_exists(Path::new("/src"), "refs/heads/x"),
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
        assert!(
            port.init_repository(Path::new("/hub"), true, ObjectFormat::Sha1)
                .is_ok()
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
        assert_eq!(port.calls().len(), 6);
    }
}
