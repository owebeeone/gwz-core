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

