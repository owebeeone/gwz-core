use super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeStore, MergeTargetKind, OperationState,
    ParticipantState,
};
use crate::artifact;
use crate::git::{GitBackend, GitHeadState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext, WorkspaceMutatorLock};
use crate::workspace::{MemberPath, WORKSPACE_MANIFEST};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::{fs, path::Path};
pub(crate) fn handle_abort<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    if request.preserve == Some(true) {
        return Err(ModelError::new(
            ErrorCode::MergePhaseUnsupported,
            "preserve-abort is not available",
        ));
    }
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    abort_with_runtime(
        &GitAbortRuntime(backend),
        store,
        root,
        request.merge_id.as_deref(),
        context,
        emitter,
    )
}
trait AbortRuntime {
    fn snapshot(
        &self,
        root: &Path,
        record: MergeOperationRecord,
    ) -> ModelResult<MergeStatusSnapshot>;
    fn abort_merge(&self, path: &Path, before: &str, merge_head: &str) -> ModelResult<()>;
    fn reset_branch(
        &self,
        path: &Path,
        branch: &str,
        current: &str,
        before: &str,
    ) -> ModelResult<()>;
    fn head(&self, _path: &Path) -> ModelResult<GitHeadState> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence inspection",
        ))
    }
    fn delete_branch(&self, _path: &Path, _branch: &str, _current: &str) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root branch deletion",
        ))
    }
    fn stage_paths(&self, _path: &Path, _paths: &[&str]) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "abort runtime does not support root evidence staging",
        ))
    }
}
struct GitAbortRuntime<'a, B>(&'a B);
impl<B: GitBackend> AbortRuntime for GitAbortRuntime<'_, B> {
    fn snapshot(
        &self,
        root: &Path,
        record: MergeOperationRecord,
    ) -> ModelResult<MergeStatusSnapshot> {
        super::status::snapshot_status(self.0, root, record)
    }

    fn abort_merge(&self, path: &Path, before: &str, merge_head: &str) -> ModelResult<()> {
        self.0.abort_merge(path, before, merge_head)
    }

    fn reset_branch(
        &self,
        path: &Path,
        branch: &str,
        current: &str,
        before: &str,
    ) -> ModelResult<()> {
        self.0
            .set_branch_target_checked(path, branch, current, before)
            .map(|_| ())
    }

    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        self.0.head(path)
    }

    fn delete_branch(&self, path: &Path, branch: &str, current: &str) -> ModelResult<()> {
        self.0.delete_branch_target_checked(path, branch, current)
    }

    fn stage_paths(&self, path: &Path, paths: &[&str]) -> ModelResult<()> {
        self.0.stage_paths(path, paths).map(|_| ())
    }
}
fn abort_with_runtime<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    requested_id: Option<&str>,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let Some(mut record) = store.discover_open(root)? else {
        return closed_or_missing(store, root, requested_id, context, emitter);
    };
    super::validate::validate_open_merge_id(requested_id, &record.merge_id)?;
    let has_root_participant = record
        .participants
        .values()
        .any(|participant| participant.target_kind == MergeTargetKind::Root);
    if has_root_participant {
        return Err(ModelError::new(
            ErrorCode::RootMergeNotYetSupported,
            "coordinated abort for root state is not available",
        ));
    }
    if record.state == OperationState::Completed {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("merge in state {:?} cannot be aborted", record.state),
        ));
    }

    // A terminal record in the open directory is archive-pending. Its baseline
    // and participant outcomes were verified before Aborted was written, so a
    // retry must finish closing it without allowing later unrelated repository
    // work to strand the durable terminal record.
    if record.state == OperationState::Aborted {
        super::archive_merge_record(store, root, &record.merge_id, emitter)?;
        return record.to_response(context);
    }

    let evidence = preflight_evidence(runtime, root, &record)?;
    record
        .operation_drift
        .retain(|drift| drift.kind != super::OperationDriftKind::RootCandidateStateChanged);
    let snapshot = runtime.snapshot(root, record)?;
    let preflight = preflight(&snapshot)?;
    record = snapshot.record;
    for target_id in preflight.pending.keys() {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        emitter.member_started(target_id, &participant.path);
    }
    if apply_pending_reconciliations(&mut record, &preflight.pending)? {
        super::persist_merge_record(store, root, &record, emitter)?;
        for target_id in preflight.pending.keys() {
            super::emit_merge_member_finished(emitter, &record, target_id)?;
        }
    }

    if record.state == OperationState::Executing {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::Halted,
            emitter,
        )?;
    }
    if record.state != OperationState::RollingBack {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::RollingBack,
            emitter,
        )?;
    }
    if let Some(evidence) = evidence.as_ref() {
        rollback_evidence(runtime, store, root, &mut record, evidence, emitter)?;
    }
    for target_id in record.selected_targets.clone().into_iter().rev() {
        let (prior, next) = {
            let participant = record.participants.get(&target_id).ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    format!("merge record is missing participant '{target_id}'"),
                )
            })?;
            let path = root.join(MemberPath::parse(&participant.path)?.as_str());
            let prior = participant.state;
            if matches!(
                prior,
                ParticipantState::Aborted | ParticipantState::RolledBack
            ) {
                continue;
            }
            emitter.member_started(&target_id, &participant.path);
            match (preflight.no_op_targets.contains(&target_id), prior) {
                (true, _) => {}
                (false, ParticipantState::Conflicted) => runtime.abort_merge(
                    &path,
                    &participant.before_commit,
                    participant
                        .expected_merge_head
                        .as_deref()
                        .unwrap_or(&participant.source_commit),
                )?,
                (
                    false,
                    ParticipantState::FastForwarded
                    | ParticipantState::Merged
                    | ParticipantState::Continued,
                ) => runtime.reset_branch(
                    &path,
                    &participant.target_branch,
                    participant.resulting_commit.as_deref().ok_or_else(|| {
                        ModelError::new(
                            ErrorCode::MergeRecordUnreadable,
                            format!("merge participant '{target_id}' has no resulting commit"),
                        )
                    })?,
                    &participant.before_commit,
                )?,
                (
                    false,
                    ParticipantState::Planned
                    | ParticipantState::UpToDate
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted,
                ) => {}
                (false, ParticipantState::Aborted | ParticipantState::RolledBack) => unreachable!(),
            }
            let next = if matches!(
                prior,
                ParticipantState::FastForwarded
                    | ParticipantState::Merged
                    | ParticipantState::Continued
            ) {
                ParticipantState::RolledBack
            } else {
                ParticipantState::Aborted
            };
            (prior, next)
        };
        record.participants.get_mut(&target_id).unwrap().state = prior.transition(next)?;
        super::persist_merge_record(store, root, &record, emitter)?;
        super::emit_merge_member_finished(emitter, &record, &target_id)?;
    }
    verify_baseline(root, &record)?;
    if let Some(evidence) = evidence.as_ref() {
        verify_evidence_baseline(runtime, root, evidence)?;
    }
    super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Aborted,
        emitter,
    )?;
    super::archive_merge_record(store, root, &record.merge_id, emitter)?;
    record.to_response(context)
}

struct EvidenceRollback {
    branch: String,
    composition_commit: String,
    baseline_commit: Option<String>,
    marker_id: String,
    baseline_lock_yaml: String,
    baseline_boundary_text: String,
}

fn preflight_evidence<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<EvidenceRollback>> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(None);
    };
    let Some(composition_commit) = publication.composition_commit.clone() else {
        return Ok(None);
    };
    let candidate = publication.candidate.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "composition commit has no durable publication candidate",
        )
    })?;
    let head = runtime.head(root)?;
    if head.is_detached || head.branch.as_deref() != Some(candidate.root_branch.as_str()) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root branch changed after merge evidence creation",
        )
        .with_member("@root", "."));
    }
    let at_composition = head.commit.as_deref() == Some(composition_commit.as_str());
    let at_baseline = head.commit == record.baseline.root_head;
    if !at_composition && !at_baseline {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root moved after merge evidence creation",
        )
        .with_member("@root", "."));
    }
    let hash = |path: &Path| {
        fs::read(path)
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
    };
    let lock = hash(&root.join(artifact::LOCK_PATH));
    let marker = hash(&artifact::marker_path(root, &candidate.marker_id));
    let boundary = hash(&super::super::workspace_exclude_path(root));
    let baseline_lock_sha256 = format!(
        "{:x}",
        Sha256::digest(candidate.baseline_lock_yaml.as_bytes())
    );
    let baseline_boundary_sha256 = format!(
        "{:x}",
        Sha256::digest(candidate.baseline_boundary_text.as_bytes())
    );
    let baseline_lock = lock.as_deref() == Some(baseline_lock_sha256.as_str());
    let candidate_lock = lock.as_deref() == publication.candidate_lock_sha256.as_deref();
    let marker_absent = marker.is_none();
    let candidate_marker = marker.as_deref() == Some(candidate.marker_sha256.as_str());
    let baseline_boundary = boundary.as_deref() == Some(baseline_boundary_sha256.as_str())
        || (boundary.is_none() && candidate.baseline_boundary_text.is_empty());
    let candidate_boundary = boundary.as_deref() == Some(candidate.boundary_sha256.as_str());
    let valid_prefix = (baseline_lock && baseline_boundary && (marker_absent || candidate_marker))
        || (candidate_lock && candidate_marker && (baseline_boundary || candidate_boundary));
    if !valid_prefix {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts changed after evidence creation",
        )
        .with_member("@root", "."));
    }
    Ok(Some(EvidenceRollback {
        branch: candidate.root_branch.clone(),
        composition_commit,
        baseline_commit: record.baseline.root_head.clone(),
        marker_id: candidate.marker_id.clone(),
        baseline_lock_yaml: candidate.baseline_lock_yaml.clone(),
        baseline_boundary_text: candidate.baseline_boundary_text.clone(),
    }))
}

fn rollback_evidence<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    evidence: &EvidenceRollback,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    if head.commit.as_deref() == Some(evidence.composition_commit.as_str()) {
        if let Some(baseline) = evidence.baseline_commit.as_deref() {
            runtime.reset_branch(
                root,
                &evidence.branch,
                &evidence.composition_commit,
                baseline,
            )?;
        } else {
            runtime.delete_branch(root, &evidence.branch, &evidence.composition_commit)?;
        }
    }
    artifact::write_atomic(
        &root.join(artifact::LOCK_PATH),
        &evidence.baseline_lock_yaml,
    )?;
    super::super::publish_workspace_exclude_candidate(root, &evidence.baseline_boundary_text)?;
    let marker_path = artifact::marker_path(root, &evidence.marker_id);
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ModelError::new(
                ErrorCode::IoError,
                format!("failed to remove merge marker during abort: {error}"),
            ));
        }
    }
    let marker_relative = format!("{}/{}.yaml", artifact::MARKER_DIR, evidence.marker_id);
    runtime.stage_paths(root, &[artifact::LOCK_PATH, &marker_relative])?;
    if let Some(publication) = record.publication.as_mut() {
        publication.evidence_rolled_back = true;
    }
    super::persist_merge_record(store, root, record, emitter)
}

fn verify_evidence_baseline<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    evidence: &EvidenceRollback,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    let marker_absent = !artifact::marker_path(root, &evidence.marker_id).exists();
    let lock_matches = fs::read(root.join(artifact::LOCK_PATH)).ok().as_deref()
        == Some(evidence.baseline_lock_yaml.as_bytes());
    let boundary_matches = fs::read(super::super::workspace_exclude_path(root))
        .ok()
        .as_deref()
        == Some(evidence.baseline_boundary_text.as_bytes());
    if head.is_detached
        || head.branch.as_deref() != Some(evidence.branch.as_str())
        || head.commit != evidence.baseline_commit
        || !marker_absent
        || !lock_matches
        || !boundary_matches
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root changed during merge evidence rollback",
        )
        .with_member("@root", "."));
    }
    Ok(())
}

#[derive(Default)]
struct AbortPreflight {
    no_op_targets: BTreeSet<String>,
    pending: BTreeMap<String, super::status::PendingActionReconciliation>,
}

fn preflight(snapshot: &MergeStatusSnapshot) -> ModelResult<AbortPreflight> {
    if let Some(drift) = snapshot.operation_drift.first() {
        return Err(ModelError::new(ErrorCode::MergeDrift, &drift.message));
    }
    let mut preflight = AbortPreflight::default();
    for target_id in &snapshot.record.selected_targets {
        let observation = snapshot.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge status is missing participant '{target_id}'"),
            )
        })?;
        let participant = snapshot.record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "status participant is missing",
            )
        })?;
        if participant.pending_action.is_some() {
            let reconciliation = pending_reconciliation(target_id, participant, observation)?;
            preflight.pending.insert(target_id.clone(), reconciliation);
            continue;
        }
        if !observation.abort_eligibility.eligible {
            let message = observation
                .drift
                .first()
                .map(|drift| drift.message.clone())
                .unwrap_or_else(|| "participant is not eligible for coordinated abort".to_owned());
            let mut error = ModelError::new(ErrorCode::MergeDrift, message);
            error.member_id = Some(target_id.clone());
            error.member_path = Some(participant.path.clone());
            return Err(error);
        }
        if verified_no_op(snapshot.record.state, participant, observation) {
            preflight.no_op_targets.insert(target_id.clone());
        }
    }
    Ok(preflight)
}

fn pending_reconciliation(
    target_id: &str,
    participant: &super::MergeParticipantRecord,
    observation: &super::MergeParticipantObservation,
) -> ModelResult<super::status::PendingActionReconciliation> {
    let pending = observation.pending_action.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "status snapshot omitted a durable pending action",
        )
        .with_member(target_id, &participant.path)
    })?;
    if participant
        .pending_action
        .as_ref()
        .is_none_or(|durable| durable.kind != pending.kind)
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "status snapshot pending-action kind does not match the durable record",
        )
        .with_member(target_id, &participant.path));
    }
    match pending.state {
        super::PendingActionObservationState::NotStarted => {
            if pending.kind == super::PendingMergeActionKind::ResolveConflict
                && !observation.abort_eligibility.eligible
            {
                let message = observation.drift.first().map_or_else(
                    || {
                        format!(
                            "participant '{target_id}' at '{}': the staged resolution cannot be discarded by ordinary abort",
                            participant.path
                        )
                    },
                    |drift| drift.message.clone(),
                );
                return Err(ModelError::new(ErrorCode::MergeDrift, message)
                    .with_member(target_id, &participant.path));
            }
            Ok(super::status::PendingActionReconciliation::NotStarted)
        }
        super::PendingActionObservationState::ExpectedConflict => Ok(
            super::status::PendingActionReconciliation::ExpectedConflict {
                conflict_paths: observation.conflict_paths.clone(),
            },
        ),
        super::PendingActionObservationState::CompletedExactly => {
            let resulting_commit = observation.live_commit.clone().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    "completed pending action has no exact live commit",
                )
                .with_member(target_id, &participant.path)
            })?;
            Ok(super::status::PendingActionReconciliation::Completed { resulting_commit })
        }
        super::PendingActionObservationState::Ambiguous => {
            let reason = pending
                .message
                .as_deref()
                .unwrap_or("pending action is not at an exact recovery point");
            let message = observation.drift.first().map_or_else(
                || {
                    format!(
                        "participant '{target_id}' at '{}': {reason}",
                        participant.path
                    )
                },
                |drift| drift.message.clone(),
            );
            Err(ModelError::new(ErrorCode::MergeDrift, message)
                .with_member(target_id, &participant.path))
        }
    }
}

fn apply_pending_reconciliations(
    record: &mut MergeOperationRecord,
    reconciliations: &BTreeMap<String, super::status::PendingActionReconciliation>,
) -> ModelResult<bool> {
    for (target_id, reconciliation) in reconciliations {
        let participant = record.participants.get_mut(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        let pending = participant.pending_action.clone().ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge participant '{target_id}' lost its pending action"),
            )
        })?;
        match reconciliation {
            super::status::PendingActionReconciliation::NotStarted => {}
            super::status::PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
                participant.state = participant.state.transition(ParticipantState::Conflicted)?;
                participant.expected_merge_head = Some(pending.source_commit);
                participant.conflict_paths.clone_from(conflict_paths);
                participant.resulting_commit = None;
                participant.error = None;
            }
            super::status::PendingActionReconciliation::Completed { resulting_commit } => {
                let next = match pending.kind {
                    super::PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                    super::PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                    super::PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                    super::PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
                };
                participant.state = participant.state.transition(next)?;
                participant.resulting_commit = Some(resulting_commit.clone());
                participant.expected_merge_head = None;
                participant.conflict_paths.clear();
                participant.error = None;
            }
            super::status::PendingActionReconciliation::Ambiguous { .. } => {
                return Err(ModelError::new(
                    ErrorCode::InternalError,
                    "ambiguous pending action escaped abort preflight",
                ));
            }
        }
        participant.pending_action = None;
    }
    Ok(!reconciliations.is_empty())
}

/// Select only no-ops already accepted by the shared status classifier. This
/// function decides whether Git must be called; it never overrides an
/// ineligible snapshot (in particular, foreign sequencer state).
fn verified_no_op(
    operation: OperationState,
    participant: &super::MergeParticipantRecord,
    observation: &super::MergeParticipantObservation,
) -> bool {
    if !observation.abort_eligibility.eligible {
        return false;
    }
    if matches!(
        participant.state,
        ParticipantState::Aborted | ParticipantState::RolledBack
    ) {
        return true;
    }
    if observation.live_commit.as_deref() != Some(&participant.before_commit)
        || observation.drift.is_empty()
    {
        return false;
    }
    match participant.state {
        ParticipantState::Conflicted => observation
            .drift
            .iter()
            .all(|drift| drift.kind == super::ParticipantDriftKind::MergeStateMissing),
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
            if operation == OperationState::RollingBack =>
        {
            observation.drift.iter().all(|drift| {
                matches!(
                    drift.kind,
                    super::ParticipantDriftKind::TargetRefChanged
                        | super::ParticipantDriftKind::HeadRewound
                )
            })
        }
        _ => false,
    }
}

fn closed_or_missing<S: MergeStore>(
    store: &S,
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let Some(merge_id) = merge_id else {
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            "no coordinated merge is open",
        ));
    };
    let record = store.load(root, merge_id)?;
    if record.state != OperationState::Aborted {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "merge '{merge_id}' in state {:?} cannot be aborted",
                record.state
            ),
        ));
    }
    // This is idempotent when the prior archive rename succeeded but a later
    // sync, verification, retention, or response step failed.
    super::archive_merge_record(store, root, merge_id, emitter)?;
    record.to_response(context)
}
fn verify_baseline(root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
    for (relative, expected) in [
        (artifact::LOCK_PATH, record.baseline.lock_sha256.as_str()),
        (WORKSPACE_MANIFEST, record.baseline.manifest_sha256.as_str()),
    ] {
        let actual = fs::read(root.join(relative))
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        if actual.as_deref() != Some(expected) {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("workspace artifact '{relative}' does not match the abort baseline"),
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{ActionKind, EventSink, NullSink};
    use crate::workspace_ops::merge::{
        MergeParticipantObservation, MergeParticipantRecord, ParticipantDrift,
        ParticipantDriftKind, PendingActionObservation, PendingActionObservationState,
        PendingMergeAction, PendingMergeActionKind, RollbackEligibility,
        status::PendingActionReconciliation,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct CollectingSink(Mutex<Vec<crate::OperationEvent>>);

    impl EventSink for CollectingSink {
        fn deliver(&self, event: crate::OperationEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    #[derive(Default)]
    struct Store {
        record: RefCell<Option<MergeOperationRecord>>,
        archived: RefCell<Option<MergeOperationRecord>>,
        writes: Cell<usize>,
        fail_write_at: Cell<Option<usize>>,
        archives: Cell<usize>,
        fail_archive_at: Cell<Option<usize>>,
        move_before_archive_failure: Cell<bool>,
    }
    impl MergeStore for Store {
        fn discover_open(&self, _: &Path) -> ModelResult<Option<MergeOperationRecord>> {
            Ok(self.record.borrow().clone())
        }
        fn load(&self, _: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
            let record = self
                .record
                .borrow()
                .clone()
                .or_else(|| self.archived.borrow().clone());
            record
                .filter(|record| record.merge_id == merge_id)
                .ok_or_else(|| ModelError::new(ErrorCode::OperationNotFound, "record not found"))
        }
        fn write_open(&self, _: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
            let write = self.writes.get() + 1;
            self.writes.set(write);
            if self.fail_write_at.get() == Some(write) {
                return Err(ModelError::new(ErrorCode::IoError, "record write failed"));
            }
            self.record.replace(Some(record.clone()));
            Ok(())
        }
        fn archive(&self, _: &Path, merge_id: &str) -> ModelResult<()> {
            let call = self.archives.get() + 1;
            self.archives.set(call);
            let should_fail = self.fail_archive_at.get() == Some(call);
            if (!should_fail || self.move_before_archive_failure.get())
                && let Some(record) = self.record.borrow_mut().take()
            {
                assert_eq!(record.merge_id, merge_id);
                self.archived.replace(Some(record));
            }
            if should_fail {
                return Err(ModelError::new(ErrorCode::IoError, "archive failed"));
            }
            Ok(())
        }
    }
    #[derive(Default)]
    struct Runtime {
        calls: RefCell<Vec<String>>,
        blocked: Option<&'static str>,
        dirty_durable: Option<&'static str>,
        applied: RefCell<BTreeSet<String>>,
        mutations: Cell<usize>,
        snapshots: Cell<usize>,
        reconciliations: RefCell<BTreeMap<String, PendingActionReconciliation>>,
    }
    impl Runtime {
        fn act(&self, verb: &str, path: &Path) -> ModelResult<()> {
            let id = path.file_name().unwrap().to_string_lossy().into_owned();
            self.calls.borrow_mut().push(format!("{verb}:{id}"));
            if self.applied.borrow_mut().insert(id) {
                self.mutations.set(self.mutations.get() + 1);
            }
            Ok(())
        }
    }
    impl AbortRuntime for Runtime {
        fn snapshot(
            &self,
            _: &Path,
            record: MergeOperationRecord,
        ) -> ModelResult<MergeStatusSnapshot> {
            self.snapshots.set(self.snapshots.get() + 1);
            let participants = record
                .selected_targets
                .iter()
                .map(|id| {
                    let participant = &record.participants[id];
                    let stale = self.applied.borrow().contains(id)
                        && matches!(
                            participant.state,
                            ParticipantState::Conflicted
                                | ParticipantState::FastForwarded
                                | ParticipantState::Merged
                                | ParticipantState::Continued
                        );
                    let mut drift: Vec<_> = stale
                        .then(|| {
                            test_drift(if participant.state == ParticipantState::Conflicted {
                                ParticipantDriftKind::MergeStateMissing
                            } else {
                                ParticipantDriftKind::HeadRewound
                            })
                        })
                        .into_iter()
                        .collect();
                    if self.blocked == Some(id.as_str()) {
                        drift.push(test_drift(ParticipantDriftKind::ForeignIntegrationState));
                    }
                    if self.dirty_durable == Some(id.as_str()) {
                        drift.push(test_drift(ParticipantDriftKind::WorktreeModified));
                    }
                    let (pending_action, pending_live_commit, pending_conflicts) = self
                        .reconciliations
                        .borrow()
                        .get(id)
                        .map_or((None, None, Vec::new()), |reconciliation| {
                            let (state, message, live, paths) = match reconciliation {
                                PendingActionReconciliation::NotStarted => (
                                    PendingActionObservationState::NotStarted,
                                    None,
                                    None,
                                    Vec::new(),
                                ),
                                PendingActionReconciliation::ExpectedConflict {
                                    conflict_paths,
                                } => (
                                    PendingActionObservationState::ExpectedConflict,
                                    None,
                                    None,
                                    conflict_paths.clone(),
                                ),
                                PendingActionReconciliation::Completed { resulting_commit } => (
                                    PendingActionObservationState::CompletedExactly,
                                    None,
                                    Some(resulting_commit.clone()),
                                    Vec::new(),
                                ),
                                PendingActionReconciliation::Ambiguous { reason, .. } => (
                                    PendingActionObservationState::Ambiguous,
                                    Some(reason.clone()),
                                    None,
                                    Vec::new(),
                                ),
                            };
                            (
                                Some(PendingActionObservation {
                                    kind: participant.pending_action.as_ref().unwrap().kind,
                                    state,
                                    message,
                                }),
                                live,
                                paths,
                            )
                        });
                    (
                        id.clone(),
                        MergeParticipantObservation {
                            live_commit: pending_live_commit.or_else(|| {
                                (stale
                                    || self.dirty_durable == Some(id.as_str())
                                    || matches!(
                                        participant.state,
                                        ParticipantState::Aborted | ParticipantState::RolledBack
                                    ))
                                .then(|| participant.before_commit.clone())
                            }),
                            conflict_paths: pending_conflicts,
                            drift,
                            continue_eligibility: Default::default(),
                            abort_eligibility: RollbackEligibility {
                                eligible: self.blocked != Some(id.as_str()),
                                blockers: Vec::new(),
                            },
                            pending_action,
                        },
                    )
                })
                .collect();
            Ok(MergeStatusSnapshot {
                record,
                participants,
                operation_drift: Vec::new(),
            })
        }
        fn abort_merge(&self, path: &Path, _: &str, _: &str) -> ModelResult<()> {
            self.act("abort", path)
        }
        fn reset_branch(&self, path: &Path, _: &str, _: &str, _: &str) -> ModelResult<()> {
            self.act("reset", path)
        }
    }
    fn participant(path: &str, state: ParticipantState) -> MergeParticipantRecord {
        let result = matches!(state, ParticipantState::UpToDate | ParticipantState::Merged)
            .then(|| format!("resulting_commit: {path}-result\n"))
            .unwrap_or_default();
        let merge_head = (state == ParticipantState::Conflicted)
            .then_some(format!("expected_merge_head: {path}-source\n"))
            .unwrap_or_default();
        serde_yaml::from_str(&format!(
            "path: {path}\ntarget_kind: member\ntarget_branch: main\nbefore_commit: {path}-before\
             \nsource_commit: {path}-source\ncommit_message: merge\nstate: {}\n{result}{merge_head}",
            serde_yaml::to_string(&state).unwrap().trim()
        ))
        .unwrap()
    }
    fn test_drift(kind: ParticipantDriftKind) -> ParticipantDrift {
        serde_yaml::from_str(&format!(
            "kind: {}\nmessage: rollback applied before record write",
            serde_yaml::to_string(&kind).unwrap().trim()
        ))
        .unwrap()
    }
    fn pending(kind: PendingMergeActionKind, path: &str) -> PendingMergeAction {
        PendingMergeAction {
            kind,
            target_branch: "main".to_owned(),
            before_commit: format!("{path}-before"),
            source_commit: format!("{path}-source"),
            commit_message: "merge".to_owned(),
            expected_result: None,
            commit_spec: None,
            extensions: Default::default(),
        }
    }
    fn set_pending(store: &Store, target_id: &str, kind: PendingMergeActionKind) {
        store
            .record
            .borrow_mut()
            .as_mut()
            .unwrap()
            .participants
            .get_mut(target_id)
            .unwrap()
            .pending_action = Some(pending(kind, target_id));
    }
    fn reconcile(runtime: &Runtime, target_id: &str, value: PendingActionReconciliation) {
        runtime
            .reconciliations
            .borrow_mut()
            .insert(target_id.to_owned(), value);
    }
    fn fixture(
        states: &[(&str, ParticipantState)],
    ) -> (crate::workspace_ops::tests::TempDir, Store) {
        let root = crate::workspace_ops::tests::TempDir::new(&format!(
            "merge-abort-{}-{}",
            states.first().map_or("empty", |(id, _)| id),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.path().join("gwz.conf")).unwrap();
        fs::write(root.path().join(artifact::LOCK_PATH), b"lock").unwrap();
        fs::write(root.path().join(WORKSPACE_MANIFEST), b"manifest").unwrap();
        let digest = |path| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
        let mut record: MergeOperationRecord = serde_yaml::from_str(
            r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_start, state: awaiting_resolution, source_ref: feature/x, created_at: now, baseline: {lock_sha256: unused, manifest_sha256: unused}, selected_targets: [], participants: {}}"#,
        )
        .unwrap();
        record.baseline.lock_sha256 = digest(root.path().join(artifact::LOCK_PATH));
        record.baseline.manifest_sha256 = digest(root.path().join(WORKSPACE_MANIFEST));
        record.selected_targets = states.iter().map(|(id, _)| (*id).into()).collect();
        record.participants = states
            .iter()
            .map(|(id, state)| ((*id).into(), participant(id, *state)))
            .collect();
        (
            root,
            Store {
                record: RefCell::new(Some(record)),
                ..Store::default()
            },
        )
    }
    fn run(
        runtime: &Runtime,
        root: &crate::workspace_ops::tests::TempDir,
        store: &Store,
    ) -> ModelResult<crate::MergeResponse> {
        run_with_id(runtime, root, store, None)
    }
    fn run_with_id(
        runtime: &Runtime,
        root: &crate::workspace_ops::tests::TempDir,
        store: &Store,
        merge_id: Option<&str>,
    ) -> ModelResult<crate::MergeResponse> {
        run_with_sink(runtime, root, store, merge_id, &NullSink)
    }

    fn run_with_sink(
        runtime: &Runtime,
        root: &crate::workspace_ops::tests::TempDir,
        store: &Store,
        merge_id: Option<&str>,
        sink: &dyn EventSink,
    ) -> ModelResult<crate::MergeResponse> {
        let context = OperationContext {
            operation_id: "op_abort".into(),
            request_id: "req".into(),
            schema_version: "gwz.v0".into(),
            action: ActionKind::Merge,
            dry_run: false,
            attribution: None,
        };
        let emitter = EventEmitter::new(&context, sink, 0);
        abort_with_runtime(runtime, store, root.path(), merge_id, &context, &emitter)
    }
    #[test]
    fn mixed_three_member_abort_unwinds_only_mutated_rows() {
        let (root, store) = fixture(&[
            ("app", ParticipantState::UpToDate),
            ("lib", ParticipantState::Merged),
            ("docs", ParticipantState::Conflicted),
        ]);
        let runtime = Runtime::default();
        let response = run(&runtime, &root, &store).unwrap();
        assert_eq!(&*runtime.calls.borrow(), &["abort:docs", "reset:lib"]);
        assert_eq!(response.participant_counts.aborted, 2);
        assert_eq!(response.participant_counts.rolled_back, 1);
    }
    #[test]
    fn foreign_state_in_earlier_app_rejects_before_later_docs_rollback() {
        let (root, store) = fixture(&[
            ("app", ParticipantState::Merged),
            ("docs", ParticipantState::Conflicted),
        ]);
        let runtime = Runtime {
            blocked: Some("app"),
            ..Runtime::default()
        };
        let error = run(&runtime, &root, &store).unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeDrift);
        assert!(runtime.calls.borrow().is_empty());
        assert_eq!(store.writes.get(), 0);
    }
    #[test]
    fn rollback_applied_before_record_failure_is_recognized_on_resume() {
        let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
        let runtime = Runtime::default();
        store.fail_write_at.set(Some(2));
        let error = run(&runtime, &root, &store).unwrap_err();
        assert_eq!(error.code, ErrorCode::IoError);
        store.fail_write_at.set(None);
        run(&runtime, &root, &store).unwrap();
        assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
        assert_eq!(runtime.mutations.get(), 1);
    }

    #[test]
    fn externally_restored_conflict_is_persisted_without_a_second_git_abort() {
        let (root, store) = fixture(&[
            ("lib", ParticipantState::Merged),
            ("docs", ParticipantState::Conflicted),
        ]);
        let runtime = Runtime::default();
        runtime.applied.borrow_mut().insert("docs".to_owned());

        let response = run(&runtime, &root, &store).unwrap();

        assert_eq!(&*runtime.calls.borrow(), &["reset:lib"]);
        assert_eq!(response.participant_counts.aborted, 1);
        assert_eq!(response.participant_counts.rolled_back, 1);
    }

    #[test]
    fn recovery_required_can_enter_guarded_rollback() {
        let (root, store) = fixture(&[("lib", ParticipantState::Merged)]);
        store.record.borrow_mut().as_mut().unwrap().state = OperationState::RecoveryRequired;

        let response = run(&Runtime::default(), &root, &store).unwrap();

        assert_eq!(response.state, crate::MergeOperationState::Aborted);
        assert!(!response.open);
    }

    #[test]
    fn durable_rollback_row_ignores_later_worktree_changes() {
        let (root, store) = fixture(&[
            ("app", ParticipantState::RolledBack),
            ("docs", ParticipantState::Conflicted),
        ]);
        store.record.borrow_mut().as_mut().unwrap().state = OperationState::RollingBack;
        let runtime = Runtime {
            dirty_durable: Some("app"),
            ..Runtime::default()
        };

        run(&runtime, &root, &store).unwrap();

        assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
    }

    #[test]
    fn terminal_archive_failure_is_retryable_without_reobserving_repositories() {
        let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
        let runtime = Runtime::default();
        store.fail_archive_at.set(Some(1));

        assert_eq!(
            run(&runtime, &root, &store).unwrap_err().code,
            ErrorCode::IoError
        );
        assert_eq!(
            store.record.borrow().as_ref().unwrap().state,
            OperationState::Aborted
        );
        let calls = runtime.calls.borrow().clone();
        let snapshots = runtime.snapshots.get();

        store.fail_archive_at.set(None);
        let sink = CollectingSink::default();
        let response = run_with_sink(&runtime, &root, &store, None, &sink).unwrap();
        assert!(!response.open);
        assert_eq!(&*runtime.calls.borrow(), &calls);
        assert_eq!(runtime.snapshots.get(), snapshots);
        assert!(store.record.borrow().is_none());
        assert_eq!(
            sink.0
                .lock()
                .unwrap()
                .last()
                .and_then(|event| event.artifact_path.as_deref()),
            Some(".gwz/merge/done/merge_1.yaml")
        );
    }

    #[test]
    fn retry_by_id_succeeds_when_archive_moved_before_reporting_failure() {
        let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
        let runtime = Runtime::default();
        store.fail_archive_at.set(Some(1));
        store.move_before_archive_failure.set(true);

        assert_eq!(
            run(&runtime, &root, &store).unwrap_err().code,
            ErrorCode::IoError
        );
        assert!(store.record.borrow().is_none());
        assert!(store.archived.borrow().is_some());

        store.fail_archive_at.set(None);
        let response = run_with_id(&runtime, &root, &store, Some("merge_1")).unwrap();
        assert_eq!(response.state, crate::MergeOperationState::Aborted);
        assert!(!response.open);
    }

    #[test]
    fn completed_pending_action_is_adopted_then_rolled_back() {
        let (root, store) = fixture(&[("app", ParticipantState::Planned)]);
        set_pending(&store, "app", PendingMergeActionKind::FastForward);
        let runtime = Runtime::default();
        reconcile(
            &runtime,
            "app",
            PendingActionReconciliation::Completed {
                resulting_commit: "app-source".to_owned(),
            },
        );

        run(&runtime, &root, &store).unwrap();

        assert_eq!(&*runtime.calls.borrow(), &["reset:app"]);
        let archived = store.archived.borrow();
        let app = &archived.as_ref().unwrap().participants["app"];
        assert_eq!(app.state, ParticipantState::RolledBack);
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn expected_pending_conflict_is_adopted_then_aborted() {
        let (root, store) = fixture(&[("docs", ParticipantState::Planned)]);
        set_pending(&store, "docs", PendingMergeActionKind::TrueMerge);
        let runtime = Runtime::default();
        reconcile(
            &runtime,
            "docs",
            PendingActionReconciliation::ExpectedConflict {
                conflict_paths: vec!["conflicted.txt".to_owned()],
            },
        );

        run(&runtime, &root, &store).unwrap();

        assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
        let archived = store.archived.borrow();
        let docs = &archived.as_ref().unwrap().participants["docs"];
        assert_eq!(docs.state, ParticipantState::Aborted);
        assert!(docs.pending_action.is_none());
    }

    #[test]
    fn ambiguous_pending_action_blocks_before_record_or_git_mutation() {
        let (root, store) = fixture(&[("app", ParticipantState::Planned)]);
        set_pending(&store, "app", PendingMergeActionKind::TrueMerge);
        let runtime = Runtime::default();
        reconcile(
            &runtime,
            "app",
            PendingActionReconciliation::Ambiguous {
                reason: "unexpected live commit".to_owned(),
                drift: Vec::new(),
            },
        );

        let error = run(&runtime, &root, &store).unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeDrift);
        assert_eq!(error.member_id.as_deref(), Some("app"));
        assert_eq!(store.writes.get(), 0);
        assert!(runtime.calls.borrow().is_empty());
    }

    #[test]
    fn pending_resolution_not_started_still_requires_abort_eligible_index() {
        let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
        set_pending(&store, "docs", PendingMergeActionKind::ResolveConflict);
        let runtime = Runtime {
            blocked: Some("docs"),
            ..Runtime::default()
        };
        reconcile(&runtime, "docs", PendingActionReconciliation::NotStarted);

        let error = run(&runtime, &root, &store).unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeDrift);
        assert_eq!(store.writes.get(), 0);
        assert!(runtime.calls.borrow().is_empty());
    }
}
