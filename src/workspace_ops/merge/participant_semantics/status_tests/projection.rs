use super::*;

#[test]
fn status_policy_exhaustively_matches_the_frozen_table() {
    let expected = [
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreBeforeOrAbort,
            RootAttemptedRole::NotAttempted,
        ),
        (
            ExpectedHeadSource::ResultingCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::ResultingCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::ResultingCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::NativeMerge,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreBeforeOrAbort,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreBeforeOrAbort,
            RootAttemptedRole::NotAttempted,
        ),
        (
            ExpectedHeadSource::ResultingCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
        (
            ExpectedHeadSource::BeforeCommit,
            ConflictRole::None,
            HeadDriftGuidance::RestoreRecordedResult,
            RootAttemptedRole::Attempted,
        ),
    ];
    for (state, expected) in STATES.into_iter().zip(expected) {
        let actual = status_policy(state);
        assert_eq!(
            (
                actual.expected_head_source,
                actual.conflict_role,
                actual.head_drift_guidance,
                actual.root_attempted_role
            ),
            expected,
            "state={state:?}"
        );
    }
}

#[test]
fn expected_head_is_exhaustive_and_requires_only_result_bearing_rows() {
    for state in STATES {
        let mut record = participant(state);
        let source = status_policy(state).expected_head_source;
        assert_eq!(
            expected_head(&record).unwrap(),
            if source == ExpectedHeadSource::ResultingCommit {
                "result"
            } else {
                "before"
            }
        );
        record.resulting_commit = None;
        match source {
            ExpectedHeadSource::BeforeCommit => {
                assert_eq!(expected_head(&record).unwrap(), "before")
            }
            ExpectedHeadSource::ResultingCommit => assert_eq!(
                expected_head(&record).unwrap_err().code,
                ErrorCode::MergeRecordUnreadable
            ),
        }
    }
}

#[test]
fn missing_repository_projection_covers_every_state_and_pending_cell() {
    for state in STATES {
        for has_pending in [false, true] {
            let mut record = participant(state);
            if has_pending {
                record.pending_action = Some(PendingMergeAction {
                    kind: PendingMergeActionKind::TrueMerge,
                    target_branch: "main".to_owned(),
                    before_commit: "before".to_owned(),
                    source_commit: "source".to_owned(),
                    commit_message: "merge".to_owned(),
                    expected_result: None,
                    commit_spec: None,
                    extensions: BTreeMap::new(),
                });
            }
            let observation = missing_repository_observation("mem_app", &record);
            let expected_abort = !has_pending
                && matches!(
                    state,
                    ParticipantState::UpToDate | ParticipantState::Unattempted
                );
            assert_eq!(
                observation.abort_eligibility.eligible, expected_abort,
                "state={state:?}, pending={has_pending}"
            );
            assert!(!observation.continue_eligibility.eligible);
            assert_eq!(
                observation.drift[0].expected_head.as_deref(),
                Some("before")
            );
            assert_eq!(
                observation
                    .drift
                    .iter()
                    .map(|item| item.kind)
                    .collect::<Vec<_>>(),
                if has_pending {
                    vec![
                        ParticipantDriftKind::RepositoryMissing,
                        ParticipantDriftKind::PendingActionAmbiguous,
                    ]
                } else {
                    vec![ParticipantDriftKind::RepositoryMissing]
                }
            );
            assert_eq!(
                observation.continue_eligibility.blockers,
                vec![ParticipantDriftKind::RepositoryMissing]
            );
            assert_eq!(
                observation.abort_eligibility.blockers,
                if expected_abort {
                    Vec::new()
                } else {
                    vec![ParticipantDriftKind::RepositoryMissing]
                }
            );
        }
    }
}

#[test]
fn drift_projection_preserves_order_and_duplicate_kinds() {
    let record = participant(ParticipantState::Merged);
    let live = ParticipantLiveState {
        branch: Some("other".to_owned()),
        head: Some("live".to_owned()),
        target_ref: Some("live".to_owned()),
        status: GitStatus {
            is_dirty: true,
            staged: 1,
            unstaged: 1,
            untracked: 1,
            unresolved: 1,
            ..GitStatus::clean()
        },
        repository_state: GitRepositoryState::CherryPick,
        merge_state: None,
        native_detail_error: None,
        missing_objects: vec![MissingObject {
            role: "source commit".to_owned(),
            oid: "source".to_owned(),
        }],
        head_relation: HeadRelation::ObjectUnavailable,
    };
    assert_eq!(
        project_participant_drift("mem_app", &record, &live)
            .drift
            .into_iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>(),
        vec![
            ParticipantDriftKind::ObjectMissing,
            ParticipantDriftKind::BranchChanged,
            ParticipantDriftKind::TargetRefChanged,
            ParticipantDriftKind::ObjectMissing,
            ParticipantDriftKind::ForeignIntegrationState,
            ParticipantDriftKind::IndexModified,
            ParticipantDriftKind::WorktreeModified,
        ]
    );
}

#[test]
fn nominal_projection_covers_every_participant_state() {
    for state in STATES {
        let record = participant(state);
        let projection = project_participant_drift("mem_app", &record, &live_for(&record));
        assert!(projection.drift.is_empty(), "state={state:?}");
        assert_eq!(
            projection.facts.conflicted,
            state == ParticipantState::Conflicted,
            "state={state:?}"
        );
    }
}

#[test]
fn every_head_relation_maps_to_the_frozen_drift_kind() {
    let record = participant(ParticipantState::Planned);
    let cases = [
        (HeadRelation::Equal, None),
        (
            HeadRelation::Advanced,
            Some(ParticipantDriftKind::HeadAdvanced),
        ),
        (
            HeadRelation::Rewound,
            Some(ParticipantDriftKind::HeadRewound),
        ),
        (
            HeadRelation::Diverged,
            Some(ParticipantDriftKind::HeadDiverged),
        ),
        (
            HeadRelation::Missing,
            Some(ParticipantDriftKind::ObjectMissing),
        ),
        (
            HeadRelation::ObjectUnavailable,
            Some(ParticipantDriftKind::ObjectMissing),
        ),
    ];
    for (relation, expected) in cases {
        let mut live = live_for(&record);
        live.head_relation = relation;
        let kinds = project_participant_drift("mem_app", &record, &live)
            .drift
            .into_iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds.last().copied(), expected, "relation={relation:?}");
    }
}

#[test]
fn every_foreign_repository_state_has_one_ordered_foreign_drift() {
    let record = participant(ParticipantState::Planned);
    let foreign = [
        GitRepositoryState::Revert,
        GitRepositoryState::RevertSequence,
        GitRepositoryState::CherryPick,
        GitRepositoryState::CherryPickSequence,
        GitRepositoryState::Bisect,
        GitRepositoryState::Rebase,
        GitRepositoryState::RebaseInteractive,
        GitRepositoryState::RebaseMerge,
        GitRepositoryState::ApplyMailbox,
        GitRepositoryState::ApplyMailboxOrRebase,
    ];
    for state in foreign {
        let mut live = live_for(&record);
        live.repository_state = state;
        assert_eq!(
            project_participant_drift("mem_app", &record, &live)
                .drift
                .into_iter()
                .map(|item| item.kind)
                .collect::<Vec<_>>(),
            vec![ParticipantDriftKind::ForeignIntegrationState],
            "repository_state={state:?}"
        );
    }
}

#[test]
fn dirty_components_distinguish_conflict_resolution_from_ordinary_drift() {
    for state in [ParticipantState::Planned, ParticipantState::Conflicted] {
        for (field, expected_drift, expected_continue_blocker) in [
            ("staged", state != ParticipantState::Conflicted, false),
            (
                "unstaged",
                state != ParticipantState::Conflicted,
                state == ParticipantState::Conflicted,
            ),
            (
                "unresolved",
                state != ParticipantState::Conflicted,
                state == ParticipantState::Conflicted,
            ),
            ("untracked", true, false),
        ] {
            let record = participant(state);
            let mut live = live_for(&record);
            live.status.is_dirty = true;
            match field {
                "staged" => live.status.staged = 1,
                "unstaged" => live.status.unstaged = 1,
                "unresolved" => live.status.unresolved = 1,
                "untracked" => live.status.untracked = 1,
                _ => unreachable!(),
            }
            let observation = observation_from_projection(
                &record,
                project_participant_drift("mem_app", &record, &live),
            );
            assert_eq!(
                observation.drift.iter().any(|item| matches!(
                    item.kind,
                    ParticipantDriftKind::IndexModified | ParticipantDriftKind::WorktreeModified
                )),
                expected_drift,
                "state={state:?}, field={field}"
            );
            assert_eq!(
                observation
                    .continue_eligibility
                    .blockers
                    .contains(&ParticipantDriftKind::IndexModified),
                expected_continue_blocker
                    || (state != ParticipantState::Conflicted
                        && matches!(field, "staged" | "unresolved")),
                "state={state:?}, field={field}"
            );
        }
    }
}
