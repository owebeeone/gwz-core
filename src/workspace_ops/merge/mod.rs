mod model;
mod plan;
mod response;
mod start;
mod validate;

pub(crate) use model::*;
pub(crate) use validate::validate_merge_request;

use std::path::Path;

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventSink, OperationRequest};
use crate::runtime::clock::Clock;
use crate::runtime::ids::IdProvider;

/// Persistence seam frozen at I0. M1 provides the filesystem implementation.
#[allow(dead_code)] // Remove when M1 wires the durable merge store.
pub(crate) trait MergeStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        unsupported_store("discover_open")
    }
    fn load(&self, _root: &Path, _merge_id: &str) -> ModelResult<MergeOperationRecord> {
        unsupported_store("load")
    }
    fn write_open(&self, _root: &Path, _record: &MergeOperationRecord) -> ModelResult<()> {
        unsupported_store("write_open")
    }
    fn archive(&self, _root: &Path, _merge_id: &str) -> ModelResult<()> {
        unsupported_store("archive")
    }
    fn gc(&self, _root: &Path, _merge_id: Option<&str>) -> ModelResult<()> {
        unsupported_store("gc")
    }
}

#[allow(dead_code)] // Used by the M1 store seam once its default methods are live.
fn unsupported_store<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("merge store method '{method}' is not implemented"),
    ))
}

/// All environmental dependencies used by the merge lifecycle are explicit.
#[allow(dead_code)] // Remove when M1 routes the service through durable dependencies.
pub(crate) struct MergeDependencies<'a, B, S, C, I> {
    pub backend: &'a B,
    pub store: &'a S,
    pub clock: &'a C,
    pub ids: &'a mut I,
    pub events: &'a dyn EventSink,
}

/// First-class merge service entry. I0 validates and dispatches only; feature
/// milestones replace typed phase errors without changing this public signature.
pub fn handle_merge<B>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
{
    validate_merge_request(&request)?;
    let context = OperationRequest::Merge(request.clone()).context(operation_id.into())?;
    dispatch_merge(backend, start, request, context)
}

/// Dependency-injected lifecycle seam used by the persistence milestones.
#[allow(dead_code)] // Remove when M1 becomes the primary merge dispatch path.
pub(crate) fn handle_merge_with_dependencies<B, S, C, I>(
    dependencies: MergeDependencies<'_, B, S, C, I>,
    start: &Path,
    request: crate::MergeRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::MergeResponse>
where
    B: GitBackend,
    S: MergeStore,
    C: Clock,
    I: IdProvider,
{
    validate_merge_request(&request)?;
    let context = OperationRequest::Merge(request.clone()).context(operation_id.into())?;
    let _ = (
        dependencies.store,
        dependencies.clock,
        dependencies.ids,
        dependencies.events,
    );
    dispatch_merge(dependencies.backend, start, request, context)
}

fn dispatch_merge<B: GitBackend>(
    backend: &B,
    start: &Path,
    request: crate::MergeRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::MergeResponse> {
    match request.op {
        crate::MergeOp::Start => start::handle_start(backend, start, &request, context),
        op => Err(ModelError::new(
            ErrorCode::MergePhaseUnsupported,
            format!("merge operation '{op:?}' is reserved but not implemented in M0"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::NullSink;
    use crate::runtime::clock::{FixedClock, TimestampMs};
    use crate::runtime::ids::SequentialIdProvider;

    struct EmptyStore;
    impl MergeStore for EmptyStore {}

    fn request() -> crate::MergeRequest {
        crate::MergeRequest {
            meta: crate::RequestMeta {
                request_id: "req".to_owned(),
                schema_version: "gwz.v0".to_owned(),
                ..crate::RequestMeta::default()
            },
            op: crate::MergeOp::Status,
            source_ref: None,
            merge_id: None,
            mode: None,
            message: None,
            preserve: None,
        }
    }

    #[test]
    fn handler_validates_before_dispatch() {
        let backend = crate::git::Git2Backend::new();
        let store = EmptyStore;
        let clock = FixedClock::new(TimestampMs(1));
        let mut ids = SequentialIdProvider::new();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &NullSink,
        };
        let mut invalid = request();
        invalid.op = crate::MergeOp::Start;
        let error = handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_1")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeValidationFailed);

        let mut ids = SequentialIdProvider::new();
        let dependencies = MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &NullSink,
        };
        let error = handle_merge_with_dependencies(dependencies, Path::new("."), request(), "op_2")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergePhaseUnsupported);
    }

    #[test]
    fn public_handler_exposes_the_frozen_service_entry() {
        let backend = crate::git::Git2Backend::new();
        let error = handle_merge(&backend, Path::new("."), request(), "op_1").unwrap_err();
        assert_eq!(error.code, ErrorCode::MergePhaseUnsupported);
    }
}
