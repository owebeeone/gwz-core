use std::collections::BTreeSet;

use super::{KnownField, ParticipantField, PublicationField, mark, rejected};
use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;

pub(super) fn known_diff(
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
) -> ModelResult<BTreeSet<KnownField>> {
    use KnownField as F;
    use ParticipantField as P;
    use PublicationField as U;
    if old.schema != new.schema
        || old.record_schema_version != new.record_schema_version
        || old.workspace_id != new.workspace_id
        || old.merge_id != new.merge_id
        || old.operation_id != new.operation_id
        || old.source_ref != new.source_ref
        || old.mode != new.mode
        || old.created_at != new.created_at
        || old.baseline != new.baseline
        || old.selected_targets != new.selected_targets
        || old.extensions != new.extensions
        || old.participants.keys().ne(new.participants.keys())
    {
        return Err(rejected("immutable record data changed"));
    }
    let mut changed = BTreeSet::new();
    mark(
        &mut changed,
        old.writer_version != new.writer_version,
        F::WriterVersion,
    );
    mark(&mut changed, old.state != new.state, F::OperationState);
    mark(
        &mut changed,
        old.recovery_context != new.recovery_context,
        F::RecoveryContext,
    );
    mark(
        &mut changed,
        old.accepted_workspace != new.accepted_workspace,
        F::Acceptance,
    );
    mark(
        &mut changed,
        old.pending_rollback != new.pending_rollback,
        F::PendingRollback,
    );
    mark(
        &mut changed,
        old.pending_preservation != new.pending_preservation,
        F::PendingPreservation,
    );
    mark(
        &mut changed,
        old.operation_drift != new.operation_drift,
        F::OperationDrift,
    );
    for (member_id, before) in &old.participants {
        let after = &new.participants[member_id];
        if before.path != after.path
            || before.target_kind != after.target_kind
            || before.target_branch != after.target_branch
            || before.before_commit != after.before_commit
            || before.source_commit != after.source_commit
            || before.commit_message != after.commit_message
            || before.extensions != after.extensions
        {
            return Err(rejected("immutable participant data changed"));
        }
        let field = |field| F::Participant {
            member_id: member_id.clone(),
            field,
        };
        mark(
            &mut changed,
            before.pending_action != after.pending_action,
            field(P::PendingAction),
        );
        mark(
            &mut changed,
            before.state != after.state
                || before.resulting_commit != after.resulting_commit
                || before.expected_merge_head != after.expected_merge_head
                || before.conflict_paths != after.conflict_paths
                || before.conflict_snapshot != after.conflict_snapshot,
            field(P::Outcome),
        );
        mark(&mut changed, before.error != after.error, field(P::Error));
        mark(
            &mut changed,
            before.preservation != after.preservation,
            field(P::Preservation),
        );
        mark(&mut changed, before.drift != after.drift, field(P::Drift));
    }
    match (&old.publication, &new.publication) {
        (None, None) => {}
        (Some(before), Some(after)) => {
            mark(
                &mut changed,
                before.step != after.step,
                F::Publication(U::Step),
            );
            mark(
                &mut changed,
                before.candidate != after.candidate
                    || before.candidate_marker_path != after.candidate_marker_path
                    || before.candidate_lock_sha256 != after.candidate_lock_sha256,
                F::Publication(U::Candidate),
            );
            mark(
                &mut changed,
                before.composition_commit != after.composition_commit
                    || before.composition_tree != after.composition_tree
                    || before.root_merge_commit != after.root_merge_commit
                    || before.candidate_hashes != after.candidate_hashes,
                F::Publication(U::Evidence),
            );
            mark(
                &mut changed,
                before.root_preservation != after.root_preservation
                    || before.preservation_prefix != after.preservation_prefix,
                F::Publication(U::Preservation),
            );
            mark(
                &mut changed,
                before.evidence_rolled_back != after.evidence_rolled_back,
                F::Publication(U::EvidenceRollback),
            );
        }
        _ => {
            changed.insert(F::Publication(U::Decision));
        }
    }
    Ok(changed)
}
