//! The family-merge wrapper: `gwz merge --remote <name> [<ref>]`.
//!
//! Order (design §3.2, §6.2; architecture §7):
//!
//! 1. attribution and request shape ([`super::request::validate_family_merge`]),
//!    including the unsupported family `dry_run`;
//! 2. workspace discovery (read-only);
//! 3. the family observation through the store contract (never creates the
//!    lock file);
//! 4. resolution through `gwz_family_model::resolve_remote_token` with
//!    `Verb::Merge` (family-only; no Git fallback);
//! 5. under the family lock only: pair every selected participant, capture
//!    source ids, fetch through [`super::transport::BackendLocalTransport`]
//!    into `refs/gwz/local-imports/<transfer-id>`, verify the received
//!    vector;
//! 6. clear the selector, set the common import ref as `source_ref`, and call
//!    the public [`crate::workspace_ops::handle_merge_with_events`] once. The
//!    engine takes its own locks; the wrapper holds only the family lock.
//!
//! LCM1.0c checkpoint: steps 1-3 run for real; the store implementation
//! refuses `Unimplemented` at step 3, which this wrapper reports as
//! `unsupported_operation`. Nothing after step 3 executes, so no lock file,
//! ref or record is created. Steps 4-6 land with lanes S and X.

use std::path::Path;

use gwz_family_store_contract::{FamilyLocation, FamilyStore};

use super::errors;
use super::request::validate_family_merge;
use crate::git::MergeAuthorityBackend;
use crate::model::ModelResult;
use crate::operation::{EventEmitter, EventSink, OperationRequest};
use crate::workspace_ops::resolve_workspace_root;

/// The store implementation core composes for family observations.
pub(crate) fn family_store() -> gwz_family_store::YamlFamilyStore {
    gwz_family_store::YamlFamilyStore::new()
}

pub(crate) fn handle<B>(
    _backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: String,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse>
where
    B: MergeAuthorityBackend,
{
    let context = OperationRequest::Merge(request.clone()).context(operation_id)?;
    let emitter = EventEmitter::new(&context, events, 0);
    emitter.operation_started();
    let result = (|| {
        let selector = validate_family_merge(&request)?;
        let what = format!("local family merge from `{}`", selector.token.as_str());
        let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
        let observation = family_store()
            .read_view(&FamilyLocation::new(&root))
            .map_err(|error| errors::store_in(&what, &error))?;
        let _view = observation.view();
        // Steps 4-6 (resolution, locked import, engine delegation) are the
        // lanes' work; reaching here with a real store is the next
        // checkpoint. Until then the selector is reported unsupported.
        Err(errors::unsupported(&what))
    })();
    emitter.operation_finished();
    result
}
