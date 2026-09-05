//! A recording, scripted [`LocalTransport`] fake.
//!
//! Every call is recorded in order. Answers come from what the test
//! scripted; an unscripted call fails typed rather than succeeding, so a
//! test cannot accidentally observe a "successful" transfer it never
//! arranged. Refs created by scripted fetches/pushes are tracked so tests
//! can assert retained partial effects.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gwz_repo_contract::ObjectId;

use crate::{LocalTransport, SourceSelector, TransportError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportCall {
    ResolveSource {
        source: PathBuf,
        selector: SourceSelector,
    },
    RefExists {
        repository: PathBuf,
        name: String,
    },
    FetchAnonymous {
        receiver: PathBuf,
        source: PathBuf,
        refspecs: Vec<String>,
    },
    PushAnonymous {
        source: PathBuf,
        destination: PathBuf,
        refspec: String,
    },
    ReadRef {
        repository: PathBuf,
        name: String,
    },
}

#[derive(Debug, Default)]
pub struct RecordingTransport {
    calls: Vec<TransportCall>,
    /// (source path, selector) -> object id.
    sources: BTreeMap<(PathBuf, String), ObjectId>,
    /// (repository, ref name) -> object id.
    refs: BTreeMap<(PathBuf, String), ObjectId>,
    /// Operations scripted to fail on their next call.
    failures: Vec<(&'static str, TransportError)>,
}

impl RecordingTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[TransportCall] {
        &self.calls
    }

    /// Script what `resolve_source` answers for a source and selector.
    pub fn source(&mut self, source: impl Into<PathBuf>, selector: &SourceSelector, oid: ObjectId) {
        self.sources
            .insert((source.into(), selector_key(selector)), oid);
    }

    /// Pre-populate a ref (for collision and read-back cases).
    pub fn set_ref(&mut self, repository: impl Into<PathBuf>, name: &str, oid: ObjectId) {
        self.refs.insert((repository.into(), name.to_owned()), oid);
    }

    pub fn ref_at(&self, repository: &Path, name: &str) -> Option<&ObjectId> {
        self.refs.get(&(repository.to_path_buf(), name.to_owned()))
    }

    /// Fail the next call of `operation` (`resolve_source`, `ref_exists`,
    /// `fetch_anonymous`, `push_anonymous`, `read_ref`).
    pub fn fail_next(&mut self, operation: &'static str, error: TransportError) {
        self.failures.push((operation, error));
    }

    fn take_failure(&mut self, operation: &str) -> Option<TransportError> {
        let index = self
            .failures
            .iter()
            .position(|(name, _)| *name == operation)?;
        Some(self.failures.remove(index).1)
    }
}

fn selector_key(selector: &SourceSelector) -> String {
    match selector {
        SourceSelector::Head => "HEAD".to_owned(),
        SourceSelector::Ref(name) => name.clone(),
    }
}

impl LocalTransport for RecordingTransport {
    fn resolve_source(
        &mut self,
        source: &Path,
        selector: &SourceSelector,
    ) -> Result<ObjectId, TransportError> {
        self.calls.push(TransportCall::ResolveSource {
            source: source.to_path_buf(),
            selector: selector.clone(),
        });
        if let Some(error) = self.take_failure("resolve_source") {
            return Err(error);
        }
        self.sources
            .get(&(source.to_path_buf(), selector_key(selector)))
            .cloned()
            .ok_or_else(|| TransportError::Repository {
                path: source.to_path_buf(),
                detail: "unscripted source".to_owned(),
            })
    }

    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, TransportError> {
        self.calls.push(TransportCall::RefExists {
            repository: repository.to_path_buf(),
            name: name.to_owned(),
        });
        if let Some(error) = self.take_failure("ref_exists") {
            return Err(error);
        }
        Ok(self
            .refs
            .contains_key(&(repository.to_path_buf(), name.to_owned())))
    }

    fn fetch_anonymous(
        &mut self,
        receiver: &Path,
        source: &Path,
        refspecs: &[String],
    ) -> Result<(), TransportError> {
        self.calls.push(TransportCall::FetchAnonymous {
            receiver: receiver.to_path_buf(),
            source: source.to_path_buf(),
            refspecs: refspecs.to_vec(),
        });
        if let Some(error) = self.take_failure("fetch_anonymous") {
            return Err(error);
        }
        for refspec in refspecs {
            let (src, dst) = refspec
                .split_once(':')
                .ok_or_else(|| TransportError::Failed {
                    detail: format!("refspec `{refspec}` has no destination"),
                })?;
            let src = src.trim_start_matches('+');
            let oid = self
                .sources
                .get(&(source.to_path_buf(), src.to_owned()))
                .or_else(|| self.refs.get(&(source.to_path_buf(), src.to_owned())))
                .cloned()
                .ok_or_else(|| TransportError::Repository {
                    path: source.to_path_buf(),
                    detail: format!("unscripted source ref {src}"),
                })?;
            self.refs
                .insert((receiver.to_path_buf(), dst.to_owned()), oid);
        }
        Ok(())
    }

    fn push_anonymous(
        &mut self,
        source: &Path,
        destination: &Path,
        refspec: &str,
    ) -> Result<(), TransportError> {
        self.calls.push(TransportCall::PushAnonymous {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            refspec: refspec.to_owned(),
        });
        if let Some(error) = self.take_failure("push_anonymous") {
            return Err(error);
        }
        let (src, dst) = refspec
            .split_once(':')
            .ok_or_else(|| TransportError::Failed {
                detail: format!("refspec `{refspec}` has no destination"),
            })?;
        let oid = self
            .refs
            .get(&(source.to_path_buf(), src.trim_start_matches('+').to_owned()))
            .cloned()
            .ok_or_else(|| TransportError::Repository {
                path: source.to_path_buf(),
                detail: format!("unscripted source ref {src}"),
            })?;
        self.refs
            .insert((destination.to_path_buf(), dst.to_owned()), oid);
        Ok(())
    }

    fn read_ref(
        &mut self,
        repository: &Path,
        name: &str,
    ) -> Result<Option<ObjectId>, TransportError> {
        self.calls.push(TransportCall::ReadRef {
            repository: repository.to_path_buf(),
            name: name.to_owned(),
        });
        if let Some(error) = self.take_failure("read_ref") {
            return Err(error);
        }
        Ok(self
            .refs
            .get(&(repository.to_path_buf(), name.to_owned()))
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::ObjectFormat;

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_bytes(ObjectFormat::Sha1, &[byte; 20]).unwrap()
    }

    #[test]
    fn scripted_transfers_move_refs_and_unscripted_calls_fail_typed() {
        let mut transport = RecordingTransport::new();
        transport.source("/src", &SourceSelector::Head, oid(1));
        transport.set_ref("/src", "refs/heads/main", oid(2));
        assert_eq!(
            transport.resolve_source(Path::new("/src"), &SourceSelector::Head),
            Ok(oid(1))
        );
        assert!(
            transport
                .resolve_source(Path::new("/other"), &SourceSelector::Head)
                .is_err()
        );
        transport
            .fetch_anonymous(
                Path::new("/recv"),
                Path::new("/src"),
                &["+refs/heads/main:refs/gwz/local-imports/t1".to_owned()],
            )
            .unwrap();
        assert_eq!(
            transport.read_ref(Path::new("/recv"), "refs/gwz/local-imports/t1"),
            Ok(Some(oid(2)))
        );
        assert_eq!(
            transport.ref_exists(Path::new("/recv"), "refs/gwz/local-imports/t1"),
            Ok(true)
        );
        transport.fail_next(
            "push_anonymous",
            TransportError::Rejected {
                refspec: "x".to_owned(),
                detail: "checked out".to_owned(),
            },
        );
        assert!(
            transport
                .push_anonymous(
                    Path::new("/src"),
                    Path::new("/hub"),
                    "refs/heads/main:refs/heads/main"
                )
                .is_err()
        );
        transport
            .push_anonymous(
                Path::new("/src"),
                Path::new("/hub"),
                "refs/heads/main:refs/heads/main",
            )
            .unwrap();
        assert_eq!(
            transport.ref_at(Path::new("/hub"), "refs/heads/main"),
            Some(&oid(2))
        );
        assert_eq!(transport.calls().len(), 7);
    }
}
