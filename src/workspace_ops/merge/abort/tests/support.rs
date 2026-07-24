use super::*;

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(super) struct CollectingSink(pub(super) Mutex<Vec<crate::OperationEvent>>);

impl EventSink for CollectingSink {
    fn deliver(&self, event: crate::OperationEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[derive(Default)]
pub(super) struct Store {
    pub(super) record: RefCell<Option<MergeOperationRecord>>,
    pub(super) archived: RefCell<Option<MergeOperationRecord>>,
    pub(super) writes: Cell<usize>,
    pub(super) fail_write_at: Cell<Option<usize>>,
    archives: Cell<usize>,
    pub(super) fail_archive_at: Cell<Option<usize>>,
    pub(super) move_before_archive_failure: Cell<bool>,
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
pub(super) struct Runtime {
    pub(super) calls: RefCell<Vec<String>>,
    pub(super) blocked: Option<&'static str>,
    pub(super) dirty_durable: Option<&'static str>,
    pub(super) applied: RefCell<BTreeSet<String>>,
    pub(super) mutations: Cell<usize>,
    pub(super) snapshots: Cell<usize>,
    pub(super) reconciliations: RefCell<BTreeMap<String, PendingActionReconciliation>>,
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
    fn snapshot(&self, _: &Path, record: MergeOperationRecord) -> ModelResult<MergeStatusSnapshot> {
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
                            PendingActionReconciliation::ExpectedConflict { conflict_paths } => (
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

pub(super) fn set_pending(store: &Store, target_id: &str, kind: PendingMergeActionKind) {
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

pub(super) fn reconcile(runtime: &Runtime, target_id: &str, value: PendingActionReconciliation) {
    runtime
        .reconciliations
        .borrow_mut()
        .insert(target_id.to_owned(), value);
}

pub(super) fn fixture(
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

pub(super) fn run(
    runtime: &Runtime,
    root: &crate::workspace_ops::tests::TempDir,
    store: &Store,
) -> ModelResult<crate::MergeResponse> {
    run_with_id(runtime, root, store, None)
}

pub(super) fn run_with_id(
    runtime: &Runtime,
    root: &crate::workspace_ops::tests::TempDir,
    store: &Store,
    merge_id: Option<&str>,
) -> ModelResult<crate::MergeResponse> {
    run_with_sink(runtime, root, store, merge_id, &NullSink)
}

pub(super) fn run_with_sink(
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
