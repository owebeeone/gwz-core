use super::*;

pub(super) fn observe<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    crate::workspace_ops::merge::v1_lifecycle::authority::observe_preservation(
        backend, context, current, request,
    )
}
