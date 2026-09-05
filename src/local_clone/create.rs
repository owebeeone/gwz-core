//! `gwz clone --local --name <Name> [dest]`, verbatim: the composition of
//! `gwz_workspace_install::install` over the real adapters (LCM1.1, lane
//! C wiring; design §4, §4.0, §4.1).
//!
//! Order, which every refusal preserves (design §6.2, §4 step 1):
//!
//! 1. the request shape (already validated by the dispatch slot) and the
//!    mode -- clean and bare are refused as unsupported here, before any
//!    observation (LCM3.1 / LCM2.3);
//! 2. the family observation through the store contract (never creates the
//!    lock file): the addressed workspace is the root of a family, a ready
//!    member of one, or in no family, in which case this create founds one;
//! 3. the destination and its root-relative recorded path, minted by core;
//! 4. the source inventory and snapshot, **before the family lock**, so a
//!    design §4.0 hazard refuses with nothing written at all;
//! 5. the family lock; founding when there is no index yet;
//! 6. `install`: admission (name, path, destination, open merge), the
//!    `creating` row, the destination directory, the copy with §4.1's
//!    exclusions, the destination's Git configuration, the pointer and
//!    marker, the completion check, the source recheck, the manifest last,
//!    then `ready`.
//!
//! A refusal before reservation leaves nothing; a family this invocation
//! founded and did not use is un-founded again (its empty index removed)
//! so that stays true. A failure after reservation leaves the `creating`
//! row with a diagnostic and the destination as it stands, for inspection;
//! nothing is cleaned up, resumed or promoted (design §4 step 4).

use std::fs;
use std::path::{Path, PathBuf};

use gwz_copy_contract::{Cancellation, CopyErrorCategory, CopyMode};
use gwz_family_model::{
    CloneMode, FamilyChange, FamilyId, MemberName, MemberPath, MemberState, validate_member_path,
};
use gwz_family_store_contract::{FamilyLocation, FamilySession, FamilyStore};
use gwz_refcopy::SystemTreeCopier;
use gwz_workspace_install::{
    InstallEffect, InstallError, InstallFailure, InstallPortError, InstallRefusal, InstallReport,
    InstallRequest, install,
};

use super::adapters::install::{CoreInstallPorts, OpenMergeProbe, capture_source};
use super::adapters::member_paths::{
    destination_path, intended_destination, mint_allocation_id, mint_family_id, recorded_path,
};
use super::errors::{self, invalid, unsupported};
use super::family_merge::family_store;
use super::request::ValidatedCloneLocal;
use crate::model::{ErrorCode, ModelError, ModelResult};

/// What one create produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateReport {
    /// The destination as allocated (the canonical parent plus the name).
    pub destination: PathBuf,
    /// The row's recorded root-relative path.
    pub recorded_path: MemberPath,
    /// The family root, canonical.
    pub root: PathBuf,
    pub family_id: FamilyId,
    /// This invocation founded the family (the addressed workspace was in
    /// none).
    pub founded: bool,
    pub install: InstallReport,
}

impl CreateReport {
    /// One line for the response envelope.
    pub fn message(&self, name: &MemberName) -> String {
        let copy = self.install.copy.as_ref().map_or_else(
            || "no copy report".to_owned(),
            |copy| {
                format!(
                    "{} files copied ({} natively, {} ordinarily), {} directories, {} symlinks, \
                     {} logical bytes",
                    copy.files(),
                    copy.native_files,
                    copy.ordinary_files,
                    copy.directories,
                    copy.symlinks,
                    copy.logical_bytes
                )
            },
        );
        let removed = self
            .install
            .git
            .as_ref()
            .map_or(0, |git| git.removed_remotes.len());
        format!(
            "created local clone `{name}` at {} (verbatim; recorded as {}; {copy}; {removed} \
             remote URL(s) removed; family {}{})",
            self.destination.display(),
            self.recorded_path,
            self.family_id,
            if self.founded { ", founded" } else { "" }
        )
    }
}

/// Where the source sits in its family.
struct Placement {
    /// The family root, canonical.
    root: PathBuf,
    /// The workspace being copied, canonical.
    source: PathBuf,
    /// The source's recorded path; `None` is the root itself.
    source_path: Option<MemberPath>,
    /// The family id when one exists.
    family_id: Option<FamilyId>,
}

/// Create the clone `request` names from the workspace at `workspace`
/// (the addressed workspace, as `resolve_workspace_root` found it), with
/// `start` as the invocation's own directory.
pub(crate) fn clone_local(
    start: &Path,
    workspace: &Path,
    request: &ValidatedCloneLocal,
    open_merge: OpenMergeProbe,
    cancellation: &dyn Cancellation,
) -> ModelResult<CreateReport> {
    let what = format!("local clone ({} mode)", request.mode.as_str());
    if request.mode != CloneMode::Verbatim {
        return Err(unsupported(&what));
    }
    let store = family_store();
    let placement = place(&store, workspace)?;
    let destination = intended_destination(&destination_path(
        start,
        request.dest.as_deref(),
        &placement.root,
        &request.name,
    )?)?;
    let path = recorded_path(&placement.root, &destination)?;

    // Step 4: the source is inventoried and captured before the lock, so a
    // §4.0 hazard refuses here with nothing written.
    let capture = capture_source(&placement.source, open_merge)
        .map_err(|error| port_error(&request.name, &destination, &error))?;

    // Step 5: the family lock, founding when there is no index yet.
    let mut session = store
        .try_lock(&FamilyLocation::new(&placement.source))
        .map_err(|error| errors::store_in(&what, &error))?;
    let (family_id, founded) = match session
        .reread()
        .map_err(|error| errors::store_in(&what, &error))?
    {
        Some(view) => (view.family_id, false),
        None => {
            let family_id = placement
                .family_id
                .clone()
                .map_or_else(mint_family_id, Ok)?;
            session
                .found(family_id.clone(), mint_allocation_id()?)
                .map_err(|error| errors::store_in(&what, &error))?;
            (family_id, true)
        }
    };
    let allocation = mint_allocation_id()?;
    let install_request = InstallRequest {
        name: request.name.clone(),
        root: placement.root.clone(),
        source: placement.source.clone(),
        destination: destination.clone(),
        path: path.clone(),
        source_path: placement.source_path.clone(),
        allocation: allocation.clone(),
        mode: CloneMode::Verbatim,
        branch: None,
        exclusions: capture.exclusions.clone(),
        copy_mode: CopyMode::Auto,
    };
    let mut ports = CoreInstallPorts::new(
        placement.root.clone(),
        family_id.clone(),
        allocation,
        placement.source.clone(),
        open_merge,
        capture,
    );
    let copier = SystemTreeCopier::new();

    // Step 6.
    match install(
        &install_request,
        &mut session,
        &copier,
        &mut ports,
        cancellation,
    ) {
        Ok(report) => Ok(CreateReport {
            destination,
            recorded_path: path,
            root: placement.root,
            family_id,
            founded,
            install: report,
        }),
        Err(failure) => {
            // A family founded for a create that reserved nothing is
            // un-founded again: the refusal leaves nothing. This removes
            // only the empty index this invocation just wrote; a directory
            // is never touched.
            let unfounded = founded
                && failure.effects.is_empty()
                && session.apply(&FamilyChange::Disband).is_ok();
            Err(failure_error(
                &request.name,
                &destination,
                &failure,
                founded && !unfounded,
            ))
        }
    }
}

/// Step 2: the family root, the copy source and its recorded path.
fn place(store: &gwz_family_store::YamlFamilyStore, workspace: &Path) -> ModelResult<Placement> {
    let source = canonical(workspace)?;
    match store
        .read_view(&FamilyLocation::new(workspace))
        .map_err(|error| errors::store_in("local clone", &error))?
    {
        gwz_family_store_contract::FamilyObservation::NoFamily => Ok(Placement {
            root: source.clone(),
            source,
            source_path: None,
            family_id: None,
        }),
        gwz_family_store_contract::FamilyObservation::Family { root, view, .. } => {
            let root = canonical(&root)?;
            if root == source {
                return Ok(Placement {
                    root,
                    source,
                    source_path: None,
                    family_id: Some(view.family_id),
                });
            }
            // A clone of a clone registers on the root; the copy source
            // must be a ready member (design §4: "any ready family member,
            // or root").
            let member = view.members.iter().find(|(_, row)| {
                fs::canonicalize(root.join(&row.path)).ok().as_deref() == Some(&source)
            });
            match member {
                Some((name, row)) if row.state == MemberState::Ready => Ok(Placement {
                    root,
                    source,
                    source_path: Some(validate_member_path(&row.path).map_err(|error| {
                        invalid(format!(
                            "member `{name}` has an unusable recorded path: {error}"
                        ))
                    })?),
                    family_id: Some(view.family_id),
                }),
                Some((name, row)) => Err(invalid(format!(
                    "local clone: `{name}` is {}, not ready; only the root or a ready family \
                     member is a copy source",
                    row.state.as_str()
                ))),
                None => Err(invalid(format!(
                    "local clone: {} reaches family {} through its pointer but is not a recorded \
                     member of it; repair the family before cloning from here",
                    source.display(),
                    view.family_id
                ))),
            }
        }
    }
}

fn canonical(path: &Path) -> ModelResult<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        ModelError::new(
            ErrorCode::IoError,
            format!("{} does not resolve: {error}", path.display()),
        )
    })
}

/// A port error before the lock (the capture): the same mapping a failed
/// inventory step gets.
fn port_error(name: &MemberName, destination: &Path, error: &InstallPortError) -> ModelError {
    ModelError::new(
        port_code(error),
        format!(
            "local clone `{name}` -> {}: inventory source failed: {error}; nothing was reserved",
            destination.display()
        ),
    )
}

fn port_code(error: &InstallPortError) -> ErrorCode {
    match error {
        InstallPortError::Layout(_) => ErrorCode::UnsupportedOperation,
        InstallPortError::Unimplemented { .. } => ErrorCode::UnsupportedOperation,
        InstallPortError::Drift { .. }
        | InstallPortError::Construction { .. }
        | InstallPortError::Configuration { .. }
        | InstallPortError::Destination { .. } => ErrorCode::IoError,
    }
}

fn refusal_code(refusal: &InstallRefusal) -> ErrorCode {
    match refusal {
        InstallRefusal::Family(refusal) => errors::refusal(refusal).code,
        InstallRefusal::SourceLayout(_) => ErrorCode::UnsupportedOperation,
        InstallRefusal::NameIsRemote(_)
        | InstallRefusal::RootNotCaptured
        | InstallRefusal::BranchExists { .. }
        | InstallRefusal::BranchNotSupported { .. } => ErrorCode::InvalidRequest,
        InstallRefusal::DestinationNotEmpty { .. }
        | InstallRefusal::DestinationIsWorkspace { .. } => ErrorCode::PathCollision,
        InstallRefusal::SourceOpenMerge { .. } => ErrorCode::OpenOperation,
    }
}

/// An `InstallFailure` as a `ModelError`: the code follows the typed cause,
/// the message names the step, the cause, every completed effect and what
/// is left for inspection.
fn failure_error(
    name: &MemberName,
    destination: &Path,
    failure: &InstallFailure,
    founded_and_kept: bool,
) -> ModelError {
    let code = match &failure.error {
        InstallError::Refused(refusals) => refusals
            .first()
            .map_or(ErrorCode::InvalidRequest, refusal_code),
        InstallError::Source(error) | InstallError::Port(error) => port_code(error),
        InstallError::Copy(error) => match error.category {
            CopyErrorCategory::DestinationNotEmpty => ErrorCode::PathCollision,
            _ => ErrorCode::IoError,
        },
        InstallError::Store(error) => errors::store(error).code,
        InstallError::Incomplete(_) | InstallError::Cancelled => ErrorCode::IoError,
    };
    let reserved = failure.effects.contains(&InstallEffect::RowAllocated);
    let left = if reserved {
        format!(
            "the `creating` row `{name}` and {} are retained for inspection (`gwz local list` \
             reports it; `gwz local dispose {name} --keep` detaches it)",
            destination.display()
        )
    } else if founded_and_kept {
        "nothing was reserved; the family founded for this create keeps its empty index".to_owned()
    } else {
        "nothing was reserved".to_owned()
    };
    ModelError::new(
        code,
        format!(
            "local clone `{name}` -> {}: {failure}; effects: {:?}; {left}",
            destination.display(),
            failure.effects
        ),
    )
}
