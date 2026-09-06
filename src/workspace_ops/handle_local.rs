//! Dispatch slots for the local clone family.
//!
//! `clone_local_workspace` (ActionKind 27), `local_family` (ActionKind 28)
//! and the family branch of `merge` (`MergeRequest.local_source_name`).
//! Each slot validates attribution and request shape, refuses an
//! unsupported family `dry_run`, discovers the workspace, observes the
//! family through the store contract and only then reserves, copies,
//! imports or removes (design §6.2).
//!
//! LCM1.1 (lane C wiring): `clone --local` in verbatim mode runs end to end
//! through `local_clone::create` (`gwz_workspace_install::install` over the
//! real adapters); `local list` observes every member's target through the
//! store; `dispose --keep` and `disband` are composed over
//! `gwz_local_disposal::dispose` and the store session. Still refusing:
//! clean and bare clones (LCM3.1 / LCM2.3), ordinary `dispose` (its fresh
//! work/history checks are LCM2.1) and a family merge's import and
//! delegation (LCM1.2), each as `unsupported_operation` after the family
//! observation and before any effect.

use std::path::Path;

use gwz_family_store_contract::{FamilyLocation, FamilyStore};

use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::local_clone::request::{
    ValidatedLocalFamily, validate_clone_local, validate_local_family,
};
use crate::local_clone::{create, dispose, errors, family_merge, list};
use crate::model::{ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationRequest};

use super::*;

/// The merge store's own envelope classifier, as the open-merge probe the
/// local-clone adapters take: `Some(merge_id)` while a record is open under
/// `.gwz/merge/`; a record that cannot be classified is an error, never
/// "no merge".
pub(crate) fn open_merge_probe(root: &Path) -> ModelResult<Option<String>> {
    Ok(super::merge::classify_open_record(root)?.map(|envelope| envelope.merge_id))
}

/// `gwz clone --local --name <name> [dest]`.
pub fn handle_clone_local_workspace<B>(
    backend: &B,
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
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let report = create::clone_local(
            backend,
            start,
            &root,
            &validated,
            open_merge_probe,
            &gwz_copy_contract::NeverCancelled,
        )?;
        let mut response =
            response_envelope(context.clone(), crate::AggregateStatus::Ok, Vec::new());
        response.meta.message = Some(report.message(&validated.name));
        Ok(crate::CloneLocalWorkspaceResponse { response })
    })();
    emitter.operation_finished();
    result
}

/// `gwz local list | dispose <name> [--keep | --force <hazards>] | disband`.
///
/// `list` is observation-only (design §3.1): on an observed family it
/// projects the model's listing -- the root first, then every member in name
/// order with its recorded and observed state -- into
/// `LocalFamilyResponse.members` (design §7, operator ruling 2026-09-05),
/// each member's target observed at its recorded path through the store; a
/// workspace that holds neither an index nor a pointer lists nothing.
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
        let what = match &validated {
            ValidatedLocalFamily::List => "local family list",
            ValidatedLocalFamily::Dispose { keep: true, .. } => "local dispose --keep",
            ValidatedLocalFamily::Dispose { .. } => "local dispose",
            ValidatedLocalFamily::Disband => "local disband",
        };
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let observation = family_merge::family_store()
            .read_view(&FamilyLocation::new(&root))
            .map_err(|error| errors::store_in(what, &error))?;
        let envelope = |status: crate::AggregateStatus, message: Option<String>| {
            let mut response = response_envelope(context.clone(), status, Vec::new());
            response.meta.message = message;
            response
        };
        match validated {
            ValidatedLocalFamily::List => {
                let members = match &observation {
                    gwz_family_store_contract::FamilyObservation::Family {
                        root: family_root,
                        view,
                        ..
                    } => list::members(view, &list::observe_members(family_root, view)?),
                    gwz_family_store_contract::FamilyObservation::NoFamily => Vec::new(),
                };
                Ok(crate::LocalFamilyResponse {
                    response: envelope(crate::AggregateStatus::Ok, None),
                    members,
                    // Present exactly when `members` is (operator ruling
                    // 2026-09-06): the observed root, for a driver to join
                    // with each member's root-relative `path`.
                    root_path: list::root_path(&observation),
                })
            }
            ValidatedLocalFamily::Dispose {
                name, keep: true, ..
            } => {
                let report = dispose::keep(start, &root, &name, open_merge_probe)?;
                Ok(crate::LocalFamilyResponse {
                    response: envelope(crate::AggregateStatus::Ok, Some(report.message(&name))),
                    members: Vec::new(),
                    root_path: None,
                })
            }
            // Ordinary deletion's fresh work and history checks are LCM2.1;
            // the ports behind them are wired (`local_clone::adapters::
            // disposal`) but the slot refuses before any effect.
            ValidatedLocalFamily::Dispose { .. } => {
                Err::<crate::LocalFamilyResponse, ModelError>(errors::unsupported(what))
            }
            ValidatedLocalFamily::Disband => {
                let (status, message) = match dispose::disband(&root)? {
                    Some(report) => (crate::AggregateStatus::Ok, report.message()),
                    None => (
                        crate::AggregateStatus::Noop,
                        format!(
                            "{} is in no local family; nothing to disband",
                            root.display()
                        ),
                    ),
                };
                Ok(crate::LocalFamilyResponse {
                    response: envelope(status, Some(message)),
                    members: Vec::new(),
                    root_path: None,
                })
            }
        }
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
