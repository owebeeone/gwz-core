//! Dispatch slots for the local clone family (LCM1.0c checkpoint).
//!
//! `clone_local_workspace` (ActionKind 27), `local_family` (ActionKind 28)
//! and the family branch of `merge` (`MergeRequest.local_source_name`).
//! Each slot validates attribution and request shape, refuses an
//! unsupported family `dry_run`, discovers the workspace, observes the
//! family through the store contract and only then would reserve, copy,
//! import or remove. At this checkpoint the composed store refuses
//! `Unimplemented`, so every slot returns `unsupported_operation` with no
//! effect: no lock file, no metadata, no copy, no import.

use std::path::Path;

use gwz_family_store_contract::{FamilyLocation, FamilyStore};

use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::local_clone::request::{
    ValidatedLocalFamily, validate_clone_local, validate_local_family,
};
use crate::local_clone::{errors, family_merge};
use crate::model::ModelResult;
use crate::operation::{EventEmitter, EventSink, OperationRequest};

use super::*;

/// `gwz clone --local --name <name> [dest]`.
pub fn handle_clone_local_workspace<B>(
    _backend: &B,
    start: &Path,
    request: crate::CloneLocalWorkspaceRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::CloneLocalWorkspaceResponse>
where
    B: GitBackend,
{
    let context =
        OperationRequest::CloneLocalWorkspace(request.clone()).context(operation_id.into())?;
    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    let result = (|| {
        let validated = validate_clone_local(&request)?;
        let what = format!("local clone ({} mode)", validated.mode.as_str());
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let _observation = family_merge::family_store()
            .read_view(&FamilyLocation::new(&root))
            .map_err(|error| errors::store_in(&what, &error))?;
        Err(errors::unsupported(&what))
    })();
    emitter.operation_finished();
    result
}

/// `gwz local list | dispose <name> [--keep | --force <hazards>] | disband`.
pub fn handle_local_family<B>(
    _backend: &B,
    start: &Path,
    request: crate::LocalFamilyRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::LocalFamilyResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::LocalFamily(request.clone()).context(operation_id.into())?;
    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    let result = (|| {
        let validated = validate_local_family(&request)?;
        let what = match validated {
            ValidatedLocalFamily::List => "local family list",
            ValidatedLocalFamily::Dispose { keep: true, .. } => "local dispose --keep",
            ValidatedLocalFamily::Dispose { .. } => "local dispose",
            ValidatedLocalFamily::Disband => "local disband",
        };
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let _observation = family_merge::family_store()
            .read_view(&FamilyLocation::new(&root))
            .map_err(|error| errors::store_in(what, &error))?;
        Err(errors::unsupported(what))
    })();
    emitter.operation_finished();
    result
}

/// The merge entry drivers dispatch every `MergeRequest` through: a request
/// carrying `local_source_name` takes the family wrapper; any other request
/// is the existing public merge engine entry, unchanged.
pub fn handle_merge_with_local_family<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    if request.local_source_name.is_none() {
        return handle_merge_with_events(backend, start, request, operation_id, events);
    }
    family_merge::handle(backend, start, request, operation_id.into(), events)
}
