//! Step 1 of design §5.2: the pure name, pointer and path validation that
//! produces the [`Plan`] every later step works from.

use std::path::PathBuf;

use gwz_family_model::{
    MemberName, MemberPath, MemberRow, PathError, relate_member_paths, validate_member_path,
};
use gwz_family_store_contract::FamilySession;

use crate::*;

/// The validated target of one invocation. A request is an invocation-local
/// value, never a reusable deletion authorization: nothing here is retained.
pub(crate) struct Plan {
    pub(crate) row: MemberRow,
    /// The recorded path resolved against the root, lexically normalised.
    pub(crate) target: PathBuf,
}

/// Design §5.2 step 1, and the only decision made without a port.
pub(crate) fn validate(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
) -> Result<Plan, DisposeError> {
    let root = resolve(&request.root);
    // Checkpoint §11 (lane D): the lock is held on the root the store found;
    // a request naming a different one is addressing a moved or replaced
    // root, and its `root.join(path)` would resolve somewhere else entirely.
    if root != resolve(session.root()) {
        return Err(DisposeError::PathMismatch {
            expected: request.root.clone(),
            observed: format!(
                "the family lock is held on {}, not this root",
                session.root().display()
            ),
        });
    }
    let view = session
        .reread()
        .map_err(DisposeError::Store)?
        .ok_or_else(|| {
            DisposeError::Store(StoreError::NoFamily {
                workspace: request.root.clone(),
            })
        })?;
    let row = view
        .members
        .get(&request.name)
        .ok_or_else(|| {
            DisposeError::Refused(Refusal::NotFound {
                name: request.name.clone(),
            })
        })?
        .clone();
    let recorded = member_path(&request.name, &row.path)?;
    let target = resolve(&request.root.join(recorded.as_str()));

    // The original tree is never deleted (design §5, §8.4). The pure path
    // rules already refuse `.` and `..` spellings; this catches the ones only
    // the host paths reveal, such as `../<the root's own directory name>`.
    if target == root || root.starts_with(&target) {
        return Err(DisposeError::RootImmutable);
    }
    if target.starts_with(&root) {
        return Err(DisposeError::Refused(Refusal::NestedPath {
            path: row.path.clone(),
            other: gwz_family_model::ROOT_PATH.to_owned(),
        }));
    }
    if resolve(&request.cwd).starts_with(&target) {
        return Err(DisposeError::TargetContainsCwd { target });
    }
    for (other_name, other) in &view.members {
        if other_name == &request.name {
            continue;
        }
        let other_path = member_path(other_name, &other.path)?;
        if relate_member_paths(&recorded, &other_path).overlaps() {
            return Err(DisposeError::Refused(Refusal::NestedPath {
                path: row.path.clone(),
                other: other.path.clone(),
            }));
        }
    }
    Ok(Plan { row, target })
}

pub(crate) fn member_path(name: &MemberName, path: &str) -> Result<MemberPath, DisposeError> {
    validate_member_path(path).map_err(|error| match error {
        // A row that names the root or an ancestor of it is refused as the
        // root, not as a malformed row: the answer the operator needs is
        // that the original tree is never deleted.
        PathError::RootItself { .. } | PathError::ContainsRoot { .. } => {
            DisposeError::RootImmutable
        }
        PathError::NotNormalised { path, normalised } => {
            DisposeError::Refused(Refusal::PathNotNormalised {
                name: name.clone(),
                path,
                normalised,
            })
        }
        other => DisposeError::Refused(Refusal::InvalidRow {
            name: name.clone(),
            detail: other.to_string(),
        }),
    })
}
