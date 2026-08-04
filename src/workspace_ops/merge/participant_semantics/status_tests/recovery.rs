use super::*;

#[test]
fn conflict_validation_outcomes_keep_error_order_and_distinct_eligibility() {
    let record = participant(ParticipantState::Conflicted);
    let live = ParticipantLiveState {
        branch: Some("main".to_owned()),
        head: Some("before".to_owned()),
        target_ref: Some("before".to_owned()),
        status: GitStatus::clean(),
        repository_state: GitRepositoryState::Merge,
        merge_state: Some(GitNativeMergeState {
            merge_head: "source".to_owned(),
            conflict_paths: Vec::new(),
            unresolved_entries: 0,
        }),
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Equal,
    };
    let mut abort_invalid = observation_from_projection(
        &record,
        project_participant_drift("mem_app", &record, &live),
    );
    apply_conflict_validation(
        "mem_app",
        &record,
        &live,
        ConflictValidationOutcomes {
            abort: ConflictValidationOutcome::Invalid("abort error".to_owned()),
            resolution: ConflictValidationOutcome::NotChecked,
        },
        &mut abort_invalid,
    );
    assert!(
        abort_invalid
            .drift
            .last()
            .unwrap()
            .message
            .contains("abort error")
    );
    assert!(!abort_invalid.continue_eligibility.eligible);
    assert!(!abort_invalid.abort_eligibility.eligible);

    let mut resolution_invalid = observation_from_projection(
        &record,
        project_participant_drift("mem_app", &record, &live),
    );
    apply_conflict_validation(
        "mem_app",
        &record,
        &live,
        ConflictValidationOutcomes {
            abort: ConflictValidationOutcome::Valid,
            resolution: ConflictValidationOutcome::Invalid("resolution error".to_owned()),
        },
        &mut resolution_invalid,
    );
    assert!(
        resolution_invalid
            .drift
            .last()
            .unwrap()
            .message
            .contains("resolution error")
    );
    assert!(!resolution_invalid.continue_eligibility.eligible);
    assert!(resolution_invalid.abort_eligibility.eligible);
}

#[test]
fn root_overrides_change_only_the_frozen_recovery_fields() {
    let mut observation = MergeParticipantObservation {
        live_commit: Some("live".to_owned()),
        conflict_paths: vec!["conflict.txt".to_owned()],
        drift: vec![missing_repository_drift(
            "@root",
            &participant(ParticipantState::Conflicted),
        )],
        continue_eligibility:
            super::super::super::continue_eligibility::missing_repository_continue_eligibility(),
        abort_eligibility: super::super::super::rollback::missing_repository_abort_eligibility(
            ParticipantState::Conflicted,
            false,
        ),
        pending_action: None,
    };
    apply_exact_root_finalization_override(&mut observation);
    assert_eq!(observation.live_commit.as_deref(), Some("live"));
    assert!(observation.conflict_paths.is_empty());
    assert!(observation.drift.is_empty());
    assert!(observation.continue_eligibility.eligible);
    assert!(observation.abort_eligibility.eligible);
}

#[test]
fn interrupted_root_override_normalizes_abort_and_preserves_continue_projection() {
    let mut root = participant(ParticipantState::Continued);
    root.path = ".".to_owned();
    root.target_kind = MergeTargetKind::Root;
    let continue_eligibility =
        super::super::super::continue_eligibility::missing_repository_continue_eligibility();
    let mut participants = BTreeMap::new();
    participants.insert("@root".to_owned(), root.clone());
    let mut observations = BTreeMap::new();
    observations.insert(
        "@root".to_owned(),
        MergeParticipantObservation {
            live_commit: Some("before".to_owned()),
            conflict_paths: vec!["conflict.txt".to_owned()],
            drift: vec![missing_repository_drift("@root", &root)],
            continue_eligibility: continue_eligibility.clone(),
            abort_eligibility: super::super::super::rollback::missing_repository_abort_eligibility(
                ParticipantState::Continued,
                false,
            ),
            pending_action: None,
        },
    );
    let mut snapshot = MergeStatusSnapshot {
        record: MergeOperationRecord {
            schema: MERGE_RECORD_SCHEMA.to_owned(),
            record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
            writer_version: "test".to_owned(),
            workspace_id: "ws".to_owned(),
            merge_id: "merge_test".to_owned(),
            operation_id: "op_test".to_owned(),
            state: OperationState::RollingBack,
            source_ref: "feature".to_owned(),
            mode: MergeExecutionMode::Normal,
            created_at: "now".to_owned(),
            baseline: MergeBaseline {
                lock_sha256: "lock".to_owned(),
                manifest_sha256: "manifest".to_owned(),
                lock_yaml: None,
                manifest_yaml: None,
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
                root_head: Some("before".to_owned()),
                root_branch: Some("main".to_owned()),
                extensions: BTreeMap::new(),
            },
            selected_targets: vec!["@root".to_owned()],
            participants,
            publication: None,
            operation_drift: Vec::new(),
            extensions: BTreeMap::new(),
        },
        participants: observations,
        operation_drift: vec![
            OperationDrift {
                kind: OperationDriftKind::RootCandidateStateChanged,
                message: "candidate".to_owned(),
            },
            OperationDrift {
                kind: OperationDriftKind::BaselineLockChanged,
                message: "baseline".to_owned(),
            },
        ],
    };

    apply_interrupted_root_rollback_override(&mut snapshot).unwrap();

    let observation = snapshot.participants.get("@root").unwrap();
    assert_eq!(observation.live_commit.as_deref(), Some("result"));
    assert!(observation.conflict_paths.is_empty());
    assert!(observation.drift.is_empty());
    assert_eq!(observation.continue_eligibility, continue_eligibility);
    assert!(observation.abort_eligibility.eligible);
    assert_eq!(
        snapshot
            .operation_drift
            .iter()
            .map(|drift| drift.kind)
            .collect::<Vec<_>>(),
        vec![OperationDriftKind::BaselineLockChanged]
    );
}
