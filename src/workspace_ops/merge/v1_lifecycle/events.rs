//! The v1 lifecycle's event surface.
//!
//! **M5d charter §4 ("Events (shape parity)").** The v0 engine is the
//! specification: "the v1 forward loop emits, per participant, `member_started`
//! **before** that participant's observation/apply and `merge_member_finished`
//! (with the `MergeRepoSummary` projection) after its durable row;
//! `operation_state_changed` at every durable state transition the store
//! commits; `artifact_written` for record create, each record commit, the
//! archive, and the publication artifacts. Same kinds, same ordering
//! discipline, same per-participant count as v0 for the same fixture; message
//! text may differ. Continue and abort arms get the same treatment."
//!
//! The v0 sites this file mirrors, one per method:
//!
//! - `merge/store/persistence.rs:21-30` (`persist_merge_record`) — one
//!   `artifact_written(".gwz/merge/<id>.yaml")` per durable record write.
//! - `merge/store/persistence.rs:8-19` (`persist_operation_transition`) — the
//!   record write, then `operation_state_changed`, never before it.
//! - `merge/store/persistence.rs:32-41` — one `artifact_written` for the
//!   archive.
//! - `merge/start/execution.rs:22` / `abort/participants.rs:37` —
//!   `member_started` once per participant, before its work.
//! - `merge/mod.rs:96-108` (`emit_merge_member_finished`), called at
//!   `start/execution.rs:59, 90, 109`, `continue_op/coordinator.rs:71, 76, 83,
//!   90`, `abort/mod.rs:153` and `abort/participants.rs:64` — the durable row's
//!   `MergeRepoSummary`, after the write that made it durable.
//! - `merge/finalize.rs:307-316` — the record write, then the four publication
//!   artifacts in the order `gwz-cli/docs/MachineOutput.md:396-406` pins.
//!
//! Nothing here names the v0 persistence seam: the call-graph gate F-3
//! (`check_checked_artifact_boundaries.py`) requires `v1_lifecycle/` to contain
//! no call into v0 merge persistence, so this is a re-derivation of the same
//! emissions from the v1 store's own commits, not a call into v0's.

use std::collections::BTreeSet;

use crate::operation::EventEmitter;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;
use crate::workspace_ops::merge::{ParticipantState, PublicationStep};

use super::authority::{ObservationKind, PhysicalActionKind};

/// The v1 lifecycle's emitter, the participant one observation has selected,
/// and the per-invocation set of participants that already announced
/// themselves.
///
/// v0 emits `member_started` once per participant per invocation — once in the
/// forward loop (`start/execution.rs:22`), once in the continue coordinator,
/// once in the abort preflight. A v1 participant is visited more than once for
/// the same work (a preparation observation, an action observation, the
/// outcome), so `started` is what makes the count one.
///
/// `selected` is what makes the POSITION match. A v1 observation on a
/// participant does not always become that participant's work: the continue
/// arm's first `ParticipantPreparation` resolves into the operation's own
/// `AwaitingResolution -> Executing` transition, which v0 takes BEFORE it
/// announces anyone (`continue_op/coordinator.rs`). So the selection is held
/// and released at the first moment the participant's work becomes visible —
/// the commit that changes its row, or the physical action that names it —
/// which is exactly where v0's three announcement sites sit.
pub(super) struct LifecycleEvents<'a> {
    emitter: Option<&'a EventEmitter<'a>>,
    selected: Option<String>,
    started: BTreeSet<String>,
}

impl<'a> LifecycleEvents<'a> {
    pub(super) fn new(emitter: &'a EventEmitter<'a>) -> Self {
        Self {
            emitter: Some(emitter),
            selected: None,
            started: BTreeSet::new(),
        }
    }

    /// The seam for callers that run the service without an operation's event
    /// stream — this tree's own suites, which drive `service::run_test`.
    #[cfg(test)]
    pub(super) fn silent() -> Self {
        Self {
            emitter: None,
            selected: None,
            started: BTreeSet::new(),
        }
    }

    /// The creation write, mirroring `merge/start.rs:119-120`: the record's
    /// artifact, then the state the store committed.
    pub(super) fn created(&self, record: &MergeOperationRecordV1) {
        let Some(emitter) = self.emitter else { return };
        emitter.artifact_written(open_record_path(&record.merge_id));
        emitter.operation_state_changed(record.state.into());
    }

    /// One observation named a participant. Held, not yet announced.
    pub(super) fn selected(&mut self, member_id: &str) {
        if !self.started.contains(member_id) {
            self.selected = Some(member_id.to_owned());
        }
    }

    /// About to commit `next`. Announce the held participant if this write is
    /// the one that changes its row.
    pub(super) fn before_commit(
        &mut self,
        current: &MergeOperationRecordV1,
        next: &MergeOperationRecordV1,
    ) {
        let Some(member_id) = self.selected.clone() else {
            return;
        };
        if current.participants.get(&member_id) != next.participants.get(&member_id) {
            self.member_started(current, &member_id);
        }
    }

    /// About to run a physical action that names a participant. This is
    /// `abort/participants.rs:37`'s position — immediately before the Git
    /// action, after the journal write that authorized it.
    pub(super) fn before_action(&mut self, record: &MergeOperationRecordV1, member_id: &str) {
        self.member_started(record, member_id);
    }

    fn member_started(&mut self, record: &MergeOperationRecordV1, member_id: &str) {
        self.selected = None;
        let Some(emitter) = self.emitter else { return };
        let Some(participant) = record.participants.get(member_id) else {
            return;
        };
        if self.started.insert(member_id.to_owned()) {
            emitter.member_started(member_id, &participant.path);
        }
    }

    /// One durable record rewrite the store committed.
    ///
    /// The artifact comes first, then the participant rows the write made
    /// durable, then the publication artifacts a verified publication earns,
    /// then the operation's new state — v0's order at
    /// `store/persistence.rs:16-17` and `finalize.rs:307-316`, so no event ever
    /// precedes the write that justifies it.
    pub(super) fn committed(
        &self,
        before: &MergeOperationRecordV1,
        after: &MergeOperationRecordV1,
    ) {
        let Some(emitter) = self.emitter else { return };
        emitter.artifact_written(open_record_path(&after.merge_id));
        for target_id in &after.selected_targets {
            let Some(row) = after.participants.get(target_id) else {
                continue;
            };
            // A row with a journalled action has not reached its outcome, and
            // `Planned` is the pre-work state: v0 emits for neither.
            if row.pending_action.is_some() || row.state == ParticipantState::Planned {
                continue;
            }
            let settled_before = before
                .participants
                .get(target_id)
                .is_some_and(|prior| prior.state == row.state && prior.pending_action.is_none());
            if settled_before {
                continue;
            }
            emitter.merge_member_finished(row.to_protocol(target_id, &after.source_ref));
        }
        if publication_became_verified(before, after) {
            emit_publication_artifacts(emitter, after);
        }
        if before.state != after.state {
            emitter.operation_state_changed(after.state.into());
        }
    }

    /// The archive write, mirroring `merge/store/persistence.rs:32-41`.
    pub(super) fn archived(&self, merge_id: &str) {
        let Some(emitter) = self.emitter else { return };
        emitter.artifact_written(done_record_path(merge_id));
    }
}

/// The member one observation request belongs to, if it belongs to one.
///
/// `observation_owner` (`authority/dispatcher.rs:383-397`) already answers this
/// for the forward kinds; the reverse cursors are owned by `@preservation` and
/// `@rollback`, and announce their participant through the physical action
/// instead — see `action_member`.
pub(super) fn observation_member(kind: &ObservationKind) -> Option<&str> {
    match kind {
        ObservationKind::ParticipantPreparation { member_id }
        | ObservationKind::ParticipantAction { member_id } => Some(member_id.as_str()),
        _ => None,
    }
}

/// The member one physical action belongs to, if it belongs to one.
///
/// This is the reverse arms' `member_started` site, matching
/// `abort/participants.rs:37` — the announcement immediately precedes the Git
/// action, not the journal write that authorized it.
pub(super) fn action_member(action: &PhysicalActionKind) -> Option<&str> {
    use crate::workspace_ops::merge::model::v1::{
        PendingPreservationActionV1, PendingRollbackActionV1, PreservationOwnerV1,
    };
    match action {
        PhysicalActionKind::Participant { member_id, .. } => Some(member_id.as_str()),
        PhysicalActionKind::Rollback(PendingRollbackActionV1::Participant {
            member_id, ..
        }) => Some(member_id.as_str()),
        PhysicalActionKind::Preservation(
            PendingPreservationActionV1::BackupRef { owner, .. }
            | PendingPreservationActionV1::Stash { owner, .. }
            | PendingPreservationActionV1::ResetAttachedRef { owner, .. },
        ) => match owner {
            PreservationOwnerV1::Participant { member_id } => Some(member_id.as_str()),
            PreservationOwnerV1::PublicationRoot => None,
        },
        PhysicalActionKind::Publication(_)
        | PhysicalActionKind::Archive
        | PhysicalActionKind::Rollback(_) => None,
    }
}

/// `VerifyingPublication -> Complete` is v0's `verify_publication` returning
/// true (`finalize.rs:297-317`): the one commit that earns the four artifacts.
fn publication_became_verified(
    before: &MergeOperationRecordV1,
    after: &MergeOperationRecordV1,
) -> bool {
    before.publication.as_ref().map(|value| value.step) == Some(PublicationStep::VerifyingPublication)
        && after.publication.as_ref().map(|value| value.step) == Some(PublicationStep::Complete)
}

/// The documented composition-evidence order, `MachineOutput.md:396-406`.
fn emit_publication_artifacts(emitter: &EventEmitter<'_>, record: &MergeOperationRecordV1) {
    let Some(publication) = record.publication.as_ref() else {
        return;
    };
    let (Some(commit), Some(marker)) = (
        publication.composition_commit.as_deref(),
        publication.candidate_marker_path.as_deref(),
    ) else {
        return;
    };
    emitter.artifact_written(format!("git:@root/{commit}"));
    emitter.artifact_written(marker);
    emitter.artifact_written(crate::artifact::LOCK_PATH);
    emitter.artifact_written(".git/info/exclude");
}

fn open_record_path(merge_id: &str) -> String {
    format!(".gwz/merge/{merge_id}.yaml")
}

fn done_record_path(merge_id: &str) -> String {
    format!(".gwz/merge/done/{merge_id}.yaml")
}
