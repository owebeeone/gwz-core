use super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeStore, MergeTargetKind, OperationState,
    ParticipantState,
};
use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, OperationContext, WorkspaceMutatorLock};
use crate::workspace::{MemberPath, WORKSPACE_MANIFEST};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
pub(crate) fn handle_abort<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    events: &dyn EventSink,
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
        events,
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
}
fn abort_with_runtime<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    requested_id: Option<&str>,
    context: &OperationContext,
    events: &dyn EventSink,
) -> ModelResult<crate::MergeResponse> {
    let mut record = store.discover_open(root)?.ok_or_else(|| {
        ModelError::new(ErrorCode::OperationNotFound, "no coordinated merge is open")
    })?;
    super::validate::validate_open_merge_id(requested_id, &record.merge_id)?;
    let has_root = record
        .participants
        .values()
        .any(|participant| participant.target_kind == MergeTargetKind::Root)
        || record.baseline.root_head.is_some()
        || record.publication.as_ref().is_some_and(|publication| {
            publication.root_merge_commit.is_some() || publication.composition_commit.is_some()
        });
    if has_root {
        return Err(ModelError::new(
            ErrorCode::RootMergeNotYetSupported,
            "coordinated abort for root state is not available",
        ));
    }
    if matches!(
        record.state,
        OperationState::Completed | OperationState::RecoveryRequired
    ) {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!("merge in state {:?} cannot be aborted", record.state),
        ));
    }

    let snapshot = runtime.snapshot(root, record)?;
    preflight(&snapshot)?;
    record = snapshot.record;
    let emitter = EventEmitter::new(context, events, 0);

    if record.state == OperationState::Aborted {
        verify_baseline(root, &record)?;
        store.archive(root, &record.merge_id)?;
        return record.to_response(context);
    }
    if record.state == OperationState::Executing {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::Halted,
            &emitter,
        )?;
    }
    if record.state != OperationState::RollingBack {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::RollingBack,
            &emitter,
        )?;
    }
    for target_id in record.selected_targets.clone().into_iter().rev() {
        let (participant_path, prior, next) = {
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
            match prior {
                ParticipantState::Conflicted => runtime.abort_merge(
                    &path,
                    &participant.before_commit,
                    participant
                        .expected_merge_head
                        .as_deref()
                        .unwrap_or(&participant.source_commit),
                )?,
                ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued => runtime.reset_branch(
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
                ParticipantState::Planned
                | ParticipantState::UpToDate
                | ParticipantState::Failed
                | ParticipantState::Unattempted => {}
                ParticipantState::Aborted | ParticipantState::RolledBack => unreachable!(),
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
            (participant.path.clone(), prior, next)
        };
        record.participants.get_mut(&target_id).unwrap().state = prior.transition(next)?;
        store.write_open(root, &record)?;
        emitter.member_finished(&target_id, &participant_path);
    }
    verify_baseline(root, &record)?;
    super::persist_operation_transition(
        store,
        root,
        &mut record,
        OperationState::Aborted,
        &emitter,
    )?;
    store.archive(root, &record.merge_id)?;
    record.to_response(context)
}
fn preflight(snapshot: &MergeStatusSnapshot) -> ModelResult<()> {
    if let Some(drift) = snapshot.operation_drift.first() {
        return Err(ModelError::new(ErrorCode::MergeDrift, &drift.message));
    }
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
        if !already_restored(snapshot.record.state, participant, observation)
            && !observation.abort_eligibility.eligible
        {
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
    }
    Ok(())
}
fn already_restored(
    operation: OperationState,
    participant: &super::MergeParticipantRecord,
    observation: &super::MergeParticipantObservation,
) -> bool {
    if operation != OperationState::RollingBack
        || observation.live_commit.as_deref() != Some(&participant.before_commit)
        || observation.drift.is_empty()
    {
        return false;
    }
    observation
        .drift
        .iter()
        .all(|drift| match participant.state {
            ParticipantState::Conflicted => {
                drift.kind == super::ParticipantDriftKind::MergeStateMissing
            }
            ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Continued => matches!(
                drift.kind,
                super::ParticipantDriftKind::TargetRefChanged
                    | super::ParticipantDriftKind::HeadRewound
            ),
            _ => false,
        })
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
    use crate::operation::{ActionKind, NullSink};
    use crate::workspace_ops::merge::{
        MergeParticipantObservation, MergeParticipantRecord, ParticipantDrift,
        ParticipantDriftKind, RollbackEligibility,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeSet;
    #[derive(Default)]
    struct Store {
        record: RefCell<Option<MergeOperationRecord>>,
        writes: Cell<usize>,
        fail_write_at: Cell<Option<usize>>,
    }
    impl MergeStore for Store {
        fn discover_open(&self, _: &Path) -> ModelResult<Option<MergeOperationRecord>> {
            Ok(self.record.borrow().clone())
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
        fn archive(&self, _: &Path, _: &str) -> ModelResult<()> {
            Ok(())
        }
    }
    #[derive(Default)]
    struct Runtime {
        calls: RefCell<Vec<String>>,
        blocked: Option<&'static str>,
        applied: RefCell<BTreeSet<String>>,
        mutations: Cell<usize>,
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
                    let kind = if participant.state == ParticipantState::Conflicted {
                        ParticipantDriftKind::MergeStateMissing
                    } else {
                        ParticipantDriftKind::HeadRewound
                    };
                    let drift = stale.then(|| test_drift(kind)).into_iter().collect();
                    (
                        id.clone(),
                        MergeParticipantObservation {
                            live_commit: stale.then(|| participant.before_commit.clone()),
                            conflict_paths: Vec::new(),
                            drift,
                            continue_eligibility: Default::default(),
                            abort_eligibility: RollbackEligibility {
                                eligible: !stale && self.blocked != Some(id.as_str()),
                                blockers: Vec::new(),
                            },
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
    fn fixture(
        states: &[(&str, ParticipantState)],
    ) -> (crate::workspace_ops::tests::TempDir, Store) {
        let root = crate::workspace_ops::tests::TempDir::new(&format!(
            "merge-abort-{}",
            states.first().map_or("empty", |(id, _)| id)
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
        let context = OperationContext {
            operation_id: "op_abort".into(),
            request_id: "req".into(),
            schema_version: "gwz.v0".into(),
            action: ActionKind::Merge,
            dry_run: false,
            attribution: None,
        };
        abort_with_runtime(runtime, store, root.path(), None, &context, &NullSink)
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
    fn post_merge_drift_rejects_before_any_rollback_or_record_write() {
        let (root, store) = fixture(&[
            ("lib", ParticipantState::Merged),
            ("docs", ParticipantState::Conflicted),
        ]);
        let runtime = Runtime {
            blocked: Some("lib"),
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
        assert_eq!(&*runtime.calls.borrow(), &["abort:docs", "abort:docs"]);
        assert_eq!(runtime.mutations.get(), 1);
    }
}
