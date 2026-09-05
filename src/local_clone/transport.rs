//! `gwz_local_import::LocalTransport` over the anonymous local ports of
//! [`GitBackend`].
//!
//! The import library never sees `git2`, credentials or named remotes: it
//! sees local repository paths, explicit refspecs and typed object ids. This
//! adapter translates each port method to exactly one backend call and maps
//! `ModelError` onto `TransportError`.

use std::path::Path;

use gwz_local_import::{LocalTransport, SourceSelector, TransportError};
use gwz_repo_contract::{ObjectFormat, ObjectId};

use crate::git::GitBackend;
use crate::model::ModelError;

/// The real transport port: one backend, local paths only.
pub struct BackendLocalTransport<'a, B: GitBackend> {
    backend: &'a B,
}

impl<'a, B: GitBackend> BackendLocalTransport<'a, B> {
    pub fn new(backend: &'a B) -> Self {
        Self { backend }
    }
}

/// Parse a backend hex object id into a typed id, inferring the format from
/// its length (40 hex = SHA-1, 64 hex = SHA-256).
pub(crate) fn object_id_from_hex(hex: &str) -> Result<ObjectId, TransportError> {
    let format = match hex.len() {
        40 => ObjectFormat::Sha1,
        64 => ObjectFormat::Sha256,
        other => {
            return Err(TransportError::Failed {
                detail: format!("object id `{hex}` has {other} hex digits"),
            });
        }
    };
    ObjectId::parse_hex(format, hex).map_err(|error| TransportError::Failed {
        detail: error.to_string(),
    })
}

fn map_error(path: &Path, error: ModelError) -> TransportError {
    match error.code {
        crate::model::ErrorCode::InvalidRequest => TransportError::NotLocal {
            detail: error.message,
        },
        crate::model::ErrorCode::RemoteRejected => TransportError::Rejected {
            refspec: String::new(),
            detail: error.message,
        },
        _ => TransportError::Repository {
            path: path.to_path_buf(),
            detail: error.message,
        },
    }
}

fn local_peer(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl<B: GitBackend> LocalTransport for BackendLocalTransport<'_, B> {
    fn resolve_source(
        &mut self,
        source: &Path,
        selector: &SourceSelector,
    ) -> Result<ObjectId, TransportError> {
        let hex = match selector {
            SourceSelector::Head => self
                .backend
                .head(source)
                .map_err(|error| map_error(source, error))?
                .commit
                .ok_or_else(|| TransportError::Repository {
                    path: source.to_path_buf(),
                    detail: "HEAD is unborn".to_owned(),
                })?,
            SourceSelector::Ref(name) => self
                .backend
                .read_ref(source, name)
                .map_err(|error| map_error(source, error))?
                .ok_or_else(|| TransportError::Repository {
                    path: source.to_path_buf(),
                    detail: format!("ref {name} does not resolve"),
                })?,
        };
        object_id_from_hex(&hex)
    }

    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, TransportError> {
        Ok(self
            .backend
            .read_ref(repository, name)
            .map_err(|error| map_error(repository, error))?
            .is_some())
    }

    fn fetch_anonymous(
        &mut self,
        receiver: &Path,
        source: &Path,
        refspecs: &[String],
    ) -> Result<(), TransportError> {
        let refspecs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
        self.backend
            .fetch_anonymous(receiver, &local_peer(source), &refspecs)
            .map(|_| ())
            .map_err(|error| map_error(receiver, error))
    }

    fn push_anonymous(
        &mut self,
        source: &Path,
        destination: &Path,
        refspec: &str,
    ) -> Result<(), TransportError> {
        self.backend
            .push_anonymous(source, &local_peer(destination), refspec)
            .map(|_| ())
            .map_err(|error| match map_error(source, error) {
                TransportError::Rejected { detail, .. } => TransportError::Rejected {
                    refspec: refspec.to_owned(),
                    detail,
                },
                other => other,
            })
    }

    fn read_ref(
        &mut self,
        repository: &Path,
        name: &str,
    ) -> Result<Option<ObjectId>, TransportError> {
        self.backend
            .read_ref(repository, name)
            .map_err(|error| map_error(repository, error))?
            .map(|hex| object_id_from_hex(&hex))
            .transpose()
    }
}
