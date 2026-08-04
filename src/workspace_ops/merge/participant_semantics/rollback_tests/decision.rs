use super::*;

#[test]
fn rollback_class_is_exhaustive() {
    let cases = [
        (
            ParticipantState::Planned,
            RollbackClass::RequiresExactBeforeNoGit,
        ),
        (ParticipantState::UpToDate, RollbackClass::NoOwnedMutation),
        (
            ParticipantState::FastForwarded,
            RollbackClass::IntegratedResult,
        ),
        (ParticipantState::Merged, RollbackClass::IntegratedResult),
        (ParticipantState::Conflicted, RollbackClass::NativeConflict),
        (
            ParticipantState::Failed,
            RollbackClass::RequiresExactBeforeNoGit,
        ),
        (
            ParticipantState::Unattempted,
            RollbackClass::NoOwnedMutation,
        ),
        (ParticipantState::Continued, RollbackClass::IntegratedResult),
        (ParticipantState::Aborted, RollbackClass::Complete),
        (ParticipantState::RolledBack, RollbackClass::Complete),
    ];
    assert_eq!(cases.len(), STATES.len());
    for (state, expected) in cases {
        assert_eq!(rollback_class(state), expected, "state={state:?}");
    }
}

#[test]
fn participant_rollback_decision_covers_every_state_and_application_status() {
    for state in STATES {
        for already_applied in [false, true] {
            let expected = match rollback_class(state) {
                RollbackClass::RequiresExactBeforeNoGit | RollbackClass::NoOwnedMutation => {
                    ParticipantRollbackDecision {
                        git_action: RollbackGitAction::None,
                        terminal_state: Some(ParticipantState::Aborted),
                    }
                }
                RollbackClass::NativeConflict => ParticipantRollbackDecision {
                    git_action: if already_applied {
                        RollbackGitAction::None
                    } else {
                        RollbackGitAction::AbortConflict
                    },
                    terminal_state: Some(ParticipantState::Aborted),
                },
                RollbackClass::IntegratedResult => ParticipantRollbackDecision {
                    git_action: if already_applied {
                        RollbackGitAction::None
                    } else {
                        RollbackGitAction::ResetIntegrated
                    },
                    terminal_state: Some(ParticipantState::RolledBack),
                },
                RollbackClass::Complete => ParticipantRollbackDecision {
                    git_action: RollbackGitAction::None,
                    terminal_state: None,
                },
            };
            assert_eq!(
                participant_rollback_decision(state, already_applied),
                expected,
                "state={state:?}, already_applied={already_applied}"
            );
        }
    }
}

#[test]
fn abort_preflight_rejects_ineligible_before_terminal_or_no_op_checks() {
    for state in STATES {
        let participant = participant(state);
        let observed = observation(
            false,
            Some("before"),
            &[ParticipantDriftKind::MergeStateMissing],
        );
        assert_eq!(
            abort_preflight_decision(OperationState::RollingBack, &participant, &observed),
            AbortPreflightDecision::Reject,
            "state={state:?}"
        );
    }
}

#[test]
fn abort_preflight_recognizes_terminal_rows_as_already_applied() {
    for state in [ParticipantState::Aborted, ParticipantState::RolledBack] {
        let participant = participant(state);
        let observed = observation(true, Some("unrelated"), &[]);
        assert_eq!(
            abort_preflight_decision(OperationState::Executing, &participant, &observed),
            AbortPreflightDecision::AlreadyApplied
        );
    }
}

#[test]
fn conflict_no_op_requires_exact_before_and_only_missing_merge_state_drift() {
    let participant = participant(ParticipantState::Conflicted);
    for (live, kinds, expected) in [
        (
            Some("before"),
            vec![ParticipantDriftKind::MergeStateMissing],
            AbortPreflightDecision::AlreadyApplied,
        ),
        (
            Some("before"),
            vec![
                ParticipantDriftKind::MergeStateMissing,
                ParticipantDriftKind::MergeStateMissing,
            ],
            AbortPreflightDecision::AlreadyApplied,
        ),
        (Some("before"), vec![], AbortPreflightDecision::Proceed),
        (
            Some("before"),
            vec![
                ParticipantDriftKind::MergeStateMissing,
                ParticipantDriftKind::WorktreeModified,
            ],
            AbortPreflightDecision::Proceed,
        ),
        (
            Some("other"),
            vec![ParticipantDriftKind::MergeStateMissing],
            AbortPreflightDecision::Proceed,
        ),
        (
            None,
            vec![ParticipantDriftKind::MergeStateMissing],
            AbortPreflightDecision::Proceed,
        ),
    ] {
        let observed = observation(true, live, &kinds);
        assert_eq!(
            abort_preflight_decision(OperationState::Executing, &participant, &observed),
            expected,
            "live={live:?}, kinds={kinds:?}"
        );
    }
}

#[test]
fn integrated_no_op_requires_preexisting_rolling_back_state_and_restore_drift() {
    let accepted = [
        ParticipantDriftKind::TargetRefChanged,
        ParticipantDriftKind::HeadRewound,
    ];
    for state in [
        ParticipantState::FastForwarded,
        ParticipantState::Merged,
        ParticipantState::Continued,
    ] {
        let participant = participant(state);
        for operation in OPERATIONS {
            let observed = observation(true, Some("before"), &accepted);
            let expected = if operation == OperationState::RollingBack {
                AbortPreflightDecision::AlreadyApplied
            } else {
                AbortPreflightDecision::Proceed
            };
            assert_eq!(
                abort_preflight_decision(operation, &participant, &observed),
                expected,
                "state={state:?}, operation={operation:?}"
            );
        }
        for (live, kinds) in [
            (Some("before"), Vec::new()),
            (
                Some("before"),
                vec![
                    ParticipantDriftKind::TargetRefChanged,
                    ParticipantDriftKind::WorktreeModified,
                ],
            ),
            (Some("result"), vec![ParticipantDriftKind::TargetRefChanged]),
            (None, vec![ParticipantDriftKind::HeadRewound]),
        ] {
            let observed = observation(true, live, &kinds);
            assert_eq!(
                abort_preflight_decision(OperationState::RollingBack, &participant, &observed),
                AbortPreflightDecision::Proceed,
                "state={state:?}, live={live:?}, kinds={kinds:?}"
            );
        }
    }
}

#[test]
fn other_nonterminal_states_never_use_verified_no_op() {
    for state in [
        ParticipantState::Planned,
        ParticipantState::UpToDate,
        ParticipantState::Failed,
        ParticipantState::Unattempted,
    ] {
        let participant = participant(state);
        let observed = observation(
            true,
            Some("before"),
            &[ParticipantDriftKind::MergeStateMissing],
        );
        assert_eq!(
            abort_preflight_decision(OperationState::RollingBack, &participant, &observed),
            AbortPreflightDecision::Proceed,
            "state={state:?}"
        );
    }
}
