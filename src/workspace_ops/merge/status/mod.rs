mod classify;
mod drift;
mod observe;
mod pending;
mod snapshot;
#[cfg(test)]
mod tests;
mod view;

use classify::*;
use drift::*;
pub(super) use observe::observe_participant;
pub(in crate::workspace_ops::merge) use observe::validated_participant_path;
use observe::{member_result, read_live_participant};
use pending::reconcile_pending_action_from_live;
pub(crate) use pending::{PendingActionReconciliation, reconcile_pending_action};
#[allow(
    unused_imports,
    reason = "the disabled v1 lifecycle consumes the shared status seam"
)]
pub(in crate::workspace_ops::merge) use snapshot::{
    CanonicalStatusSource, MergeStatusViewObservation, observe_status_view,
    select_canonical_status_source,
};
pub(crate) use snapshot::{handle_status, snapshot_status};
pub(in crate::workspace_ops) use view::MergeStatusRecordView;
