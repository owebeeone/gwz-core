#![forbid(clippy::disallowed_methods)]

use super::super::super::model::v1::rollback_cursor;
use super::*;
use crate::git::GitBackend;
use crate::operation::OperationContext;

mod archive;
mod finalization;
mod forward;
mod reverse;

pub(in crate::workspace_ops::merge::v1_lifecycle) use finalization::{
    RecordEvidenceOr, observe_finalization, observe_reverse_publication_handoff,
    verify_finalization_action, verify_finalization_recovery_origin,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use forward::{
    observe_forward, verify_participant_action,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use reverse::{
    prepare_direct_rollback_entry, prepare_exhausted_rollback_entry, prepare_preservation_entry,
    preservation_durability_fact, preservation_execution_prefix_is_exact, preservation_reset_step,
    preservation_stash_guard, preservation_stash_step, preserving_verify_recovery_origin,
    require_rollback_aggregate, rolling_back_verify_recovery_origin,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_preservation<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    reverse::observe_preservation(backend, context, current, request)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_rollback<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    reverse::observe_rollback(backend, context, current, request)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_archive(
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    archive::observe(current, request)
}

mod reverse_entry_visitor_seal {
    pub(super) trait Visitor {}
    pub(super) trait AuthorityResult {}
}

impl reverse_entry_visitor_seal::AuthorityResult for RecordEvidenceOr<VerifiedPublicationHandoff> {}
impl reverse_entry_visitor_seal::AuthorityResult for VerifiedPreservationEntryPreflight {}
impl reverse_entry_visitor_seal::AuthorityResult for VerifiedRollbackEntryPreflight {}

#[allow(private_bounds)]
pub(in crate::workspace_ops::merge::v1_lifecycle) trait SealedReverseEntryVisitor:
    reverse_entry_visitor_seal::Visitor
{
    type SealedAuthority: reverse_entry_visitor_seal::AuthorityResult;

    fn inspect(
        &mut self,
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        request: V1LifecycleRequest,
        kind: ReverseEntryKind,
        anticipated_model_sha256: [u8; 32],
    ) -> ModelResult<Self::SealedAuthority>;
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn no_mutation_abort(
    current: &StoredV1Record,
) -> ModelResult<VerifiedNoMutationAbort> {
    let RollbackCursor::NoMutationParticipant { member_id } = rollback_cursor(current.record())
    else {
        return Err(authority_error(
            "rollback cursor does not identify a no-mutation participant",
        ));
    };
    VerifiedNoMutationAbort::issue(
        &AuthorityIssuer::for_observer(current),
        member_id,
        "record_no_mutation_abort",
        "cursor_verified",
        member_id.into(),
    )
}

pub(super) fn rollback_exhausted(
    current: &StoredV1Record,
    prefix: VerifiedRollbackPrefix,
) -> ModelResult<VerifiedRollbackExhausted> {
    if !prefix.matches(current) {
        return Err(authority_error(
            "rollback exhaustion lacks its exact aggregate prefix proof",
        ));
    }
    let payload = match rollback_cursor(current.record()) {
        RollbackCursor::Complete => RollbackExhaustedPayload {
            selected_root_manifest_sha256: None,
            selected_root_lock_sha256: None,
        },
        RollbackCursor::SelectedRootMetadata => selected_root_baseline(current)?,
        _ => return Err(authority_error("rollback cursor is not complete")),
    };
    VerifiedRollbackExhausted::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "rollback_exhausted",
        "cursor_verified",
        payload,
    )
}

#[cfg(test)]
pub(in crate::workspace_ops::merge::v1_lifecycle) fn rollback_exhausted_for_test(
    current: &StoredV1Record,
) -> ModelResult<VerifiedRollbackExhausted> {
    let value = RollbackAggregatePayload {
        position: RollbackAggregatePosition::Exhaustion,
        completed_participants: Vec::new(),
        publication_evidence_complete: false,
        selected_root_projection: None,
        projection_sha256: [0; 32],
    };
    let prefix = VerifiedRollbackPrefix::issue(&AuthorityIssuer::for_observer(current), value)?;
    rollback_exhausted(current, prefix)
}

fn selected_root_baseline(current: &StoredV1Record) -> ModelResult<RollbackExhaustedPayload> {
    let fact = crate::workspace_ops::merge::root::observe_v1_selected_root_baseline(
        current.location().root(),
        current.record(),
    )?;
    Ok(RollbackExhaustedPayload {
        selected_root_manifest_sha256: Some(fact.0),
        selected_root_lock_sha256: Some(fact.1),
    })
}
