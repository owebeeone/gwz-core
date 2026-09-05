//! `gwz local dispose <name> --keep` and `gwz local disband` (LCM1.1, lane
//! C wiring; design §3.1, §5, §5.2, §8.4, §8.6).
//!
//! `keep` is `gwz_local_disposal::dispose` with `DisposePolicy::Keep` over
//! the real ports: under the family lock the library validates the name,
//! the root and the working directory, then removes the pointer and the
//! marker and the row -- in that order, the store's only recoverable one --
//! and consults no port: every file stays, including an incomplete or
//! interrupted tree the ordinary path would refuse (design §5.2 step 2,
//! §12 "Dirty/open/incomplete lane with keep").
//!
//! `disband` is core's own composition over the store session, as the
//! disposal crate's documentation requires (it has no entry point for it):
//! under the family lock, every member's pointer and marker are removed
//! through `remove_pointer`, then `FamilyChange::Disband` removes the index.
//! Every tree stays. It is repeatable on explicit invocation: a pointer
//! already gone is not an error, a family already disbanded is a no-op,
//! and a pointer the store cannot remove stops it with the index intact so
//! an explicit repeat can finish (design §3.1 "interrupted pointer-only
//! detach/disband").
//!
//! Ordinary deletion is not composed here: its fresh work and history
//! checks are LCM2.1, and the dispatch slot refuses it as unsupported.

use std::fs;
use std::path::{Path, PathBuf};

use gwz_family_model::{FamilyChange, FamilyId, MemberName};
use gwz_family_store_contract::{
    FamilyLocation, FamilyObservation, FamilySession, FamilyStore, MetadataEffect, StoreError,
};
use gwz_local_disposal::{
    DisposeEffect, DisposeError, DisposeFailure, DisposePolicy, DisposeReport, DisposeRequest,
    dispose,
};

use super::adapters::disposal::CoreDisposalPorts;
use super::adapters::install::OpenMergeProbe;
use super::errors::{self, invalid};
use super::family_merge::family_store;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// What `dispose --keep` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepReport {
    /// The detached member's recorded path, resolved against the root.
    pub retained_at: PathBuf,
    pub effects: Vec<DisposeEffect>,
}

impl KeepReport {
    pub fn message(&self, name: &MemberName) -> String {
        let pointer = if self.effects.contains(&DisposeEffect::PointerRemoved) {
            "its pointer and marker removed"
        } else {
            "no pointer of this family stood there"
        };
        format!(
            "detached local clone `{name}`: row removed, {pointer}; every file at {} is retained",
            self.retained_at.display()
        )
    }
}

/// Detach `name`, retaining every file.
pub(crate) fn keep(
    start: &Path,
    workspace: &Path,
    name: &MemberName,
    open_merge: OpenMergeProbe,
) -> ModelResult<KeepReport> {
    const WHAT: &str = "local dispose --keep";
    let store = family_store();
    let workspace = canonical(workspace)?;
    if store
        .read_view(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(WHAT, &error))?
        .view()
        .is_none()
    {
        return Err(errors::store_in(
            WHAT,
            &StoreError::NoFamily {
                workspace: workspace.clone(),
            },
        ));
    }
    let mut session = store
        .try_lock(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(WHAT, &error))?;
    let root = canonical(session.root())?;
    let view = session
        .reread()
        .map_err(|error| errors::store_in(WHAT, &error))?
        .ok_or_else(|| {
            errors::store_in(
                WHAT,
                &StoreError::NoFamily {
                    workspace: root.clone(),
                },
            )
        })?;
    let retained_at = view
        .members
        .get(name)
        .map(|row| root.join(&row.path))
        .unwrap_or_else(|| root.join(name.as_str()));
    let request = DisposeRequest {
        name: name.clone(),
        policy: DisposePolicy::Keep,
        root: root.clone(),
        cwd: fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf()),
    };
    let mut ports = CoreDisposalPorts::new(root, view, name.clone(), open_merge);
    match dispose(&request, &mut session, &mut ports) {
        Ok(DisposeReport { effects }) => Ok(KeepReport {
            retained_at,
            effects,
        }),
        Err(failure) => Err(failure_error(name, &retained_at, &failure)),
    }
}

/// What `disband` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisbandReport {
    pub family_id: FamilyId,
    /// Members whose pointer was removed.
    pub pointers_removed: Vec<String>,
    /// Members whose allocation marker was removed.
    pub markers_removed: Vec<String>,
    /// Members at whose recorded path no pointer of this family stood
    /// (already detached, directory gone, or foreign metadata retained).
    pub nothing_to_remove: Vec<String>,
    pub index_removed: bool,
}

impl DisbandReport {
    pub fn message(&self) -> String {
        format!(
            "disbanded local family {}: {} pointer(s) and {} marker(s) removed, {} member(s) \
             held none, index {}; every tree is retained",
            self.family_id,
            self.pointers_removed.len(),
            self.markers_removed.len(),
            self.nothing_to_remove.len(),
            if self.index_removed {
                "removed"
            } else {
                "retained"
            }
        )
    }
}

/// Remove every member's pointer and marker, then the index. `None` when
/// the workspace is in no family (a repeat after a completed disband).
pub(crate) fn disband(workspace: &Path) -> ModelResult<Option<DisbandReport>> {
    const WHAT: &str = "local disband";
    let store = family_store();
    let workspace = canonical(workspace)?;
    match store
        .read_view(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(WHAT, &error))?
    {
        FamilyObservation::NoFamily => return Ok(None),
        FamilyObservation::Family { .. } => {}
    }
    let mut session = store
        .try_lock(&FamilyLocation::new(&workspace))
        .map_err(|error| errors::store_in(WHAT, &error))?;
    let Some(view) = session
        .reread()
        .map_err(|error| errors::store_in(WHAT, &error))?
    else {
        return Ok(None);
    };
    let mut report = DisbandReport {
        family_id: view.family_id.clone(),
        pointers_removed: Vec::new(),
        markers_removed: Vec::new(),
        nothing_to_remove: Vec::new(),
        index_removed: false,
    };
    for name in view.members.keys() {
        let applied = session.remove_pointer(name).map_err(|error| {
            ModelError::new(
                errors::store(&error).code,
                format!(
                    "{WHAT}: removing member `{name}`'s pointer stopped: {error}; the index is \
                     retained and an explicit repeat may finish the disband"
                ),
            )
        })?;
        let mut removed_anything = false;
        for effect in &applied.effects {
            match effect {
                MetadataEffect::PointerRemoved { .. } => {
                    removed_anything = true;
                    report.pointers_removed.push(name.as_str().to_owned());
                }
                MetadataEffect::MarkerRemoved { .. } => {
                    removed_anything = true;
                    report.markers_removed.push(name.as_str().to_owned());
                }
                _ => {}
            }
        }
        if !removed_anything {
            report.nothing_to_remove.push(name.as_str().to_owned());
        }
    }
    session
        .apply(&FamilyChange::Disband)
        .map_err(|error| errors::store_in(WHAT, &error))?;
    report.index_removed = true;
    Ok(Some(report))
}

fn canonical(path: &Path) -> ModelResult<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("{} does not resolve: {error}", path.display()),
        )
    })
}

/// A `DisposeFailure` as a `ModelError`: the code follows the typed cause,
/// the message names every completed effect.
fn failure_error(name: &MemberName, retained_at: &Path, failure: &DisposeFailure) -> ModelError {
    let error = match &failure.error {
        DisposeError::Refused(refusal) => errors::refusal(refusal),
        DisposeError::RootImmutable
        | DisposeError::TargetContainsCwd { .. }
        | DisposeError::PathMismatch { .. } => invalid(failure.error.to_string()),
        DisposeError::Hazards(_) => {
            ModelError::new(ErrorCode::PermissionDenied, failure.error.to_string())
        }
        DisposeError::Unknown(_) | DisposeError::Unimplemented => {
            ModelError::new(ErrorCode::UnsupportedOperation, failure.error.to_string())
        }
        DisposeError::Store(error) => errors::store(error),
        DisposeError::Port(_) | DisposeError::RemovalStopped { .. } => {
            ModelError::new(ErrorCode::IoError, failure.error.to_string())
        }
    };
    ModelError::new(
        error.code,
        format!(
            "local dispose `{name}` --keep at {}: {}; effects: {:?}",
            retained_at.display(),
            error.message,
            failure.effects
        ),
    )
}
