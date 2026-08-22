use super::*;
use crate::workspace_ops::merge::{ParticipantState, PreservationEvidence};

fn write_root_prefix(
    backend: &crate::git::Git2Backend,
    root: &Path,
    record: &MergeOperationRecord,
    prefix: &str,
) {
    let candidate = record
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    let marker_relative = format!(
        "{}/{}.yaml",
        crate::artifact::MARKER_DIR,
        candidate.marker_id
    );
    crate::artifact::write_atomic(
        &root.join(crate::artifact::LOCK_PATH),
        if matches!(prefix, "lock" | "boundary") {
            &candidate.lock_yaml
        } else {
            &candidate.baseline_lock_yaml
        },
    )
    .unwrap();
    let marker = root.join(&marker_relative);
    if prefix == "baseline" {
        match fs::remove_file(&marker) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove marker: {error}"),
        }
    } else {
        crate::artifact::write_atomic(&marker, &candidate.marker_yaml).unwrap();
    }
    fs::write(
        root.join(".git/info/exclude"),
        if prefix == "boundary" {
            &candidate.boundary_text
        } else {
            &candidate.baseline_boundary_text
        },
    )
    .unwrap();
    backend
        .stage_paths(root, &[crate::artifact::LOCK_PATH, &marker_relative])
        .unwrap();
}

#[test]
fn v0_preserving_overlay_round_trips_every_recorded_root_prefix() {
    for prefix in ["baseline", "marker", "lock", "boundary"] {
        let temp = TempDir::new(&format!("v0-preserving-prefix-{prefix}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(
            temp.path(),
            &backend,
            &format!("v0-preserving-prefix-{prefix}"),
        );
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
        invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_preserving_prefix",
        )
        .unwrap_err();
        let mut record = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(
            record.publication.as_ref().unwrap().step,
            PublicationStep::PublishingCandidate
        );
        record.state = OperationState::Preserving;
        record.publication.as_mut().unwrap().preservation_prefix = Some(prefix.to_owned());
        write_root_prefix(&backend, temp.path(), &record, prefix);
        store.write_open(temp.path(), &record).unwrap();
        let open_path = temp
            .path()
            .join(format!(".gwz/merge/{}.yaml", record.merge_id));
        let before = fs::read(&open_path).unwrap();
        let status = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Status, Some(record.merge_id.clone())),
            "op_v0_preserving_prefix_status",
        )
        .unwrap();
        assert_eq!(
            status.state,
            crate::MergeOperationState::Preserving,
            "{prefix}"
        );
        assert!(status.operation_drift.is_empty(), "{prefix}");
        assert_eq!(fs::read(open_path).unwrap(), before, "{prefix}");
        let loaded = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(
            loaded.publication.unwrap().preservation_prefix.as_deref(),
            Some(prefix),
            "{prefix}"
        );
    }
}

struct PersistThenFailRollbackStore {
    fail_after_terminal_rows: usize,
    fired: Cell<bool>,
}

impl MergeStore for PersistThenFailRollbackStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        FileMergeStore.write_open(root, record)?;
        let terminal_rows = record
            .participants
            .values()
            .filter(|row| {
                matches!(
                    row.state,
                    ParticipantState::Aborted | ParticipantState::RolledBack
                )
            })
            .count();
        if !self.fired.get()
            && record.state == OperationState::RollingBack
            && terminal_rows == self.fail_after_terminal_rows
        {
            self.fired.set(true);
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("injected after {terminal_rows} durable rollback rows"),
            ));
        }
        Ok(())
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        FileMergeStore.archive(root, merge_id)
    }
}

fn invoke_rollback_store(
    backend: &crate::git::Git2Backend,
    store: &PersistThenFailRollbackStore,
    root: &Path,
    request: crate::MergeRequest,
) -> ModelResult<crate::MergeResponse> {
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request,
        "op_v0_participant_reverse",
    )
}

#[test]
fn v0_participant_rollback_has_restartable_durable_reverse_prefixes() {
    for terminal_rows in 0..=3 {
        let temp = TempDir::new(&format!("v0-participant-reverse-{terminal_rows}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
        let started = handle_merge(
            &backend,
            temp.path(),
            request(false),
            "op_v0_participant_reverse_start",
        )
        .unwrap();
        let merge_id = started.merge_id.unwrap();
        let store = PersistThenFailRollbackStore {
            fail_after_terminal_rows: terminal_rows,
            fired: Cell::new(false),
        };
        let error = invoke_rollback_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
        let interrupted = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(interrupted.state, OperationState::RollingBack);
        if terminal_rows == 0 {
            super::compatibility_v0::assert_i2_valid_unlisted_fixture(
                &backend,
                temp.path(),
                &interrupted,
                "rollback/participant",
                "0",
            );
        }
        let reversed = interrupted
            .selected_targets
            .iter()
            .rev()
            .take(terminal_rows)
            .collect::<Vec<_>>();
        for target_id in &interrupted.selected_targets {
            let is_terminal = matches!(
                interrupted.participants[target_id].state,
                ParticipantState::Aborted | ParticipantState::RolledBack
            );
            assert_eq!(
                is_terminal,
                reversed.contains(&target_id),
                "{terminal_rows}: {target_id}"
            );
        }
        let aborted = invoke_rollback_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(merge_id)),
        )
        .unwrap();
        assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    }
}

#[test]
fn v0_evidence_rollback_records_each_reverse_artifact_prefix() {
    use crate::workspace_ops::merge::{
        EvidenceRollbackMutation, fail_next_evidence_rollback_after,
    };
    for mutation in [
        EvidenceRollbackMutation::Boundary,
        EvidenceRollbackMutation::Lock,
        EvidenceRollbackMutation::Marker,
        EvidenceRollbackMutation::Staging,
    ] {
        let temp = TempDir::new(&format!("v0-evidence-reverse-{mutation:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(
            temp.path(),
            &backend,
            &format!("v0-evidence-reverse-{mutation:?}"),
        );
        backend
            .stage_paths(temp.path(), &["gwz.conf", crate::artifact::LOCK_PATH])
            .unwrap();
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let candidate_store = FaultingMergeStore::new(FinalizationFault::AfterCandidatePersistence);
        invoke_with_store(
            &backend,
            &candidate_store,
            temp.path(),
            request(false),
            "op_v0_evidence_candidate",
        )
        .unwrap_err();
        let mut record = candidate_store.discover_open(temp.path()).unwrap().unwrap();
        let candidate = record
            .publication
            .as_mut()
            .unwrap()
            .candidate
            .as_mut()
            .unwrap();
        candidate.boundary_text.push_str("# v0-reverse-prefix\n");
        candidate.boundary_sha256 =
            format!("{:x}", Sha256::digest(candidate.boundary_text.as_bytes()));
        FileMergeStore.write_open(temp.path(), &record).unwrap();
        let publication_store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
        invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
            "op_v0_evidence_publish",
        )
        .unwrap_err();
        let published = publication_store
            .discover_open(temp.path())
            .unwrap()
            .unwrap();
        let candidate = published
            .publication
            .as_ref()
            .unwrap()
            .candidate
            .as_ref()
            .unwrap()
            .clone();
        fail_next_evidence_rollback_after(mutation);
        let error = invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(published.merge_id.clone())),
            "op_v0_evidence_reverse",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
        let interrupted = publication_store
            .discover_open(temp.path())
            .unwrap()
            .unwrap();
        assert_eq!(interrupted.state, OperationState::RollingBack);
        assert!(!interrupted.publication.unwrap().evidence_rolled_back);
        assert_eq!(
            fs::read_to_string(temp.path().join(".git/info/exclude")).unwrap(),
            candidate.baseline_boundary_text,
            "{mutation:?}"
        );
        let lock = fs::read_to_string(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
        assert_eq!(
            lock,
            if mutation == EvidenceRollbackMutation::Boundary {
                candidate.lock_yaml.as_str()
            } else {
                candidate.baseline_lock_yaml.as_str()
            },
            "{mutation:?}"
        );
        assert_eq!(
            crate::artifact::marker_path(temp.path(), &candidate.marker_id).is_file(),
            matches!(
                mutation,
                EvidenceRollbackMutation::Boundary | EvidenceRollbackMutation::Lock
            ),
            "{mutation:?}"
        );
        let marker_relative = format!(
            "{}/{}.yaml",
            crate::artifact::MARKER_DIR,
            candidate.marker_id
        );
        let candidate_dirty = backend
            .status(temp.path())
            .unwrap()
            .files
            .iter()
            .any(|file| {
                matches!(file.path.as_str(), crate::artifact::LOCK_PATH)
                    || file.path == marker_relative
            });
        assert_eq!(
            candidate_dirty,
            mutation != EvidenceRollbackMutation::Staging,
            "{mutation:?}"
        );
        let aborted = invoke_with_store(
            &backend,
            &publication_store,
            temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(published.merge_id)),
            "op_v0_evidence_reverse_retry",
        )
        .unwrap();
        assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    }
}

#[test]
fn v0_preservation_restart_rebuilds_missing_stash_bundle_from_recorded_evidence() {
    let temp = TempDir::new("v0-stash-bundle-restart");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started =
        handle_merge(&backend, temp.path(), request(false), "op_v0_bundle_start").unwrap();
    let merge_id = started.merge_id.unwrap();
    let lib = temp.path().join("lib");
    fs::write(lib.join("bundle-restart.txt"), "preserve me\n").unwrap();
    let stash = backend
        .stash_for_merge_preservation(&lib, &merge_id, true)
        .unwrap();
    let stash_id = format!("stash_{merge_id}");
    let mut record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    record.state = OperationState::Preserving;
    record.participants.get_mut("mem_lib").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: Some(stash_id.clone()),
        stash_object_id: Some(stash.object_id.clone()),
        noop_commit: None,
        reset_commit: None,
    }];
    FileMergeStore.write_open(temp.path(), &record).unwrap();
    super::compatibility_v0::assert_i2_valid_unlisted_fixture(
        &backend,
        temp.path(),
        &record,
        "preserving/stash",
        "single",
    );
    assert!(!crate::stash::bundle_path(temp.path(), &stash_id).exists());
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id));
    abort.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), abort, "op_v0_bundle_retry").unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    let bundle = crate::stash::read_bundle(temp.path(), &stash_id).unwrap();
    assert!(bundle.members.iter().any(|member| {
        member.member_id == "mem_lib"
            && member.native_stash_object_id.as_deref() == Some(stash.object_id.as_str())
    }));
    assert_eq!(
        backend
            .stash_list(&lib)
            .unwrap()
            .iter()
            .filter(|entry| entry.object_id == stash.object_id)
            .count(),
        1
    );
}

#[test]
fn v0_gc_restarts_after_one_of_two_recorded_backup_refs_was_deleted() {
    let temp = TempDir::new("v0-partial-multi-ref-gc");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    backend.switch_branch(&app, "feature/source").unwrap();
    commit_file(
        &app,
        "app-source.txt",
        "source\n",
        "app source",
        &[git2::Oid::from_str(&fixture.app_before).unwrap()],
    )
    .unwrap();
    backend.switch_branch(&app, "main").unwrap();
    let started = handle_merge(&backend, temp.path(), request(false), "op_v0_gc_start").unwrap();
    for target in ["mem_app", "mem_lib"] {
        let repo = merge_repo(&started, target);
        let path = temp.path().join(&repo.path);
        let parent = backend.head(&path).unwrap().commit.unwrap();
        commit_file(
            &path,
            &format!("{target}-after.txt"),
            "preserve\n",
            "post-merge work",
            &[git2::Oid::from_str(&parent).unwrap()],
        )
        .unwrap();
    }
    let mut abort = recovery_request(crate::MergeOp::Abort, started.merge_id);
    abort.preserve = Some(true);
    let aborted = handle_merge(&backend, temp.path(), abort, "op_v0_gc_abort").unwrap();
    let merge_id = aborted.merge_id.unwrap();
    let archived = FileMergeStore.load(temp.path(), &merge_id).unwrap();
    let evidence = ["mem_app", "mem_lib"].map(|target| {
        let row = &archived.participants[target];
        let item = row.preservation.first().unwrap();
        (
            temp.path().join(&row.path),
            item.backup_ref.clone().unwrap(),
            item.backup_commit.clone().unwrap(),
        )
    });
    backend
        .delete_backup_ref_checked(&evidence[0].0, &evidence[0].1, &evidence[0].2)
        .unwrap();
    assert!(
        backend
            .read_ref(&evidence[0].0, &evidence[0].1)
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .read_ref(&evidence[1].0, &evidence[1].1)
            .unwrap()
            .is_some()
    );
    let mut gc = recovery_request(crate::MergeOp::Gc, Some(merge_id.clone()));
    gc.preserve = None;
    let response = handle_merge(&backend, temp.path(), gc, "op_v0_gc_retry").unwrap();
    assert!(
        backend
            .read_ref(&evidence[1].0, &evidence[1].1)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        FileMergeStore
            .load(temp.path(), &merge_id)
            .unwrap_err()
            .code,
        ErrorCode::OperationNotFound
    );
    assert!(response.preservation.is_none());
}
