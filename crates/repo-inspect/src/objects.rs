//! Bounded object-store reads.
//!
//! `gwz-history-check` walks graphs; it needs each object's kind, size and
//! outgoing edges, never blob bytes (contract: "Blob bytes are never
//! returned; history verification needs edges and presence, not content").
//!
//! The size bound is applied from the object **header**, before the object is
//! loaded, so an oversized object costs a header read rather than its own
//! size in memory. Nothing here fetches: a missing object is
//! [`ReadError::Missing`], never a reason to reach a promisor remote.

use git2::{ErrorCode, ObjectType, Repository};
use gwz_repo_contract::{ObjectFormat, ObjectId, ObjectKind, ObjectRecord, ReadError, ReadLimits};

use crate::oid::{to_contract_oid, to_git_oid};

pub(crate) fn read_object(
    repository: &Repository,
    format: ObjectFormat,
    oid: &ObjectId,
    limits: &ReadLimits,
) -> Result<ObjectRecord, ReadError> {
    let git_oid = to_git_oid(oid).map_err(|detail| ReadError::ReadFailed { detail })?;
    let odb = repository.odb().map_err(|error| ReadError::ReadFailed {
        detail: error.message().to_owned(),
    })?;
    let (size, kind) = match odb.read_header(git_oid) {
        Ok(header) => header,
        Err(error) if error.code() == ErrorCode::NotFound => {
            return Err(ReadError::Missing { oid: oid.clone() });
        }
        Err(error) => {
            return Err(ReadError::ReadFailed {
                detail: format!("{oid}: {}", error.message()),
            });
        }
    };
    let size = size as u64;
    if size > limits.max_object_bytes {
        return Err(ReadError::LimitExceeded {
            oid: oid.clone(),
            size,
            limit: limits.max_object_bytes,
        });
    }
    let (kind, edges) = edges(repository, format, oid, git_oid, kind)?;
    Ok(ObjectRecord {
        oid: oid.clone(),
        kind,
        size,
        edges,
    })
}

/// A commit's edges are its tree then its parents, in that order — the same
/// shape `contract_tests::InMemoryObjectReader::commit` builds, so one
/// traversal works against either reader.
fn edges(
    repository: &Repository,
    format: ObjectFormat,
    oid: &ObjectId,
    git_oid: git2::Oid,
    kind: ObjectType,
) -> Result<(ObjectKind, Vec<ObjectId>), ReadError> {
    let corrupt = |error: git2::Error| ReadError::Corrupt {
        oid: oid.clone(),
        detail: error.message().to_owned(),
    };
    let convert = |id: git2::Oid| {
        to_contract_oid(format, id).map_err(|detail| ReadError::Corrupt {
            oid: oid.clone(),
            detail,
        })
    };
    match kind {
        ObjectType::Commit => {
            let commit = repository.find_commit(git_oid).map_err(corrupt)?;
            let mut edges = vec![convert(commit.tree_id())?];
            for parent in commit.parent_ids() {
                edges.push(convert(parent)?);
            }
            Ok((ObjectKind::Commit, edges))
        }
        ObjectType::Tree => {
            let tree = repository.find_tree(git_oid).map_err(corrupt)?;
            let mut edges = Vec::with_capacity(tree.len());
            for entry in tree.iter() {
                edges.push(convert(entry.id())?);
            }
            Ok((ObjectKind::Tree, edges))
        }
        ObjectType::Blob => Ok((ObjectKind::Blob, Vec::new())),
        ObjectType::Tag => {
            let tag = repository.find_tag(git_oid).map_err(corrupt)?;
            Ok((ObjectKind::Tag, vec![convert(tag.target_id())?]))
        }
        ObjectType::Any => Err(ReadError::Corrupt {
            oid: oid.clone(),
            detail: "the object store did not report a kind".to_owned(),
        }),
    }
}
