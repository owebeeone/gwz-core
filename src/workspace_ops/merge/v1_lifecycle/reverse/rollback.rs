use super::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::observe_rollback;

pub(super) fn observe<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    observe_rollback(backend, context, current, request)
}

#[cfg(test)]
#[path = "../tests/reverse_rollback/mod.rs"]
mod tests;
