use super::*;

const STATES: [ParticipantState; 10] = [
    ParticipantState::Planned,
    ParticipantState::UpToDate,
    ParticipantState::FastForwarded,
    ParticipantState::Merged,
    ParticipantState::Conflicted,
    ParticipantState::Failed,
    ParticipantState::Unattempted,
    ParticipantState::Continued,
    ParticipantState::Aborted,
    ParticipantState::RolledBack,
];

#[test]
fn disposition_exhaustively_classifies_all_participant_states() {
    let expected = [
        ContinueDisposition::RetryIntegration,
        ContinueDisposition::Settled,
        ContinueDisposition::Settled,
        ContinueDisposition::Settled,
        ContinueDisposition::ResolveConflict,
        ContinueDisposition::RetryIntegration,
        ContinueDisposition::RetryIntegration,
        ContinueDisposition::Settled,
        ContinueDisposition::RejectedTerminal,
        ContinueDisposition::RejectedTerminal,
    ];

    for (state, expected) in STATES.into_iter().zip(expected) {
        assert_eq!(continue_disposition(state), expected, "state={state:?}");
    }
}

#[test]
fn post_start_exhaustively_classifies_every_single_state() {
    let expected = [
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
    ];

    for (state, expected) in STATES.into_iter().zip(expected) {
        assert_eq!(post_start_state([state]), expected, "state={state:?}");
    }
}

#[test]
fn post_continue_exhaustively_classifies_every_single_state() {
    let expected = [
        OperationState::AwaitingResolution,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::AwaitingResolution,
        OperationState::Finalizing,
        OperationState::Finalizing,
        OperationState::Finalizing,
    ];

    for (state, expected) in STATES.into_iter().zip(expected) {
        assert_eq!(post_continue_state([state]), expected, "state={state:?}");
    }
}

#[test]
fn failed_wins_mixed_aggregate_precedence() {
    assert_eq!(
        post_start_state([
            ParticipantState::Conflicted,
            ParticipantState::Failed,
            ParticipantState::Merged,
        ]),
        OperationState::Halted
    );
    assert_eq!(
        post_continue_state([
            ParticipantState::Planned,
            ParticipantState::Conflicted,
            ParticipantState::Failed,
            ParticipantState::Merged,
        ]),
        OperationState::Halted
    );
}

#[test]
fn continue_eligibility_preserves_facts_and_blocker_order() {
    let ordinary_clean = ContinueEligibilityFacts {
        conflicted: false,
        native_merge_matches: false,
        conflict_has_unresolved_or_unstaged: false,
        repository_is_clean: true,
        worktree_is_clean: true,
    };
    assert_eq!(
        continue_eligibility(ordinary_clean, Vec::new()),
        RetryEligibility {
            eligible: true,
            blockers: Vec::new(),
        }
    );

    let drift = vec![
        ParticipantDriftKind::BranchChanged,
        ParticipantDriftKind::WorktreeModified,
        ParticipantDriftKind::WorktreeModified,
    ];
    assert_eq!(
        continue_eligibility(ordinary_clean, drift.clone()),
        RetryEligibility {
            eligible: false,
            blockers: drift,
        }
    );

    for facts in [
        ContinueEligibilityFacts {
            repository_is_clean: false,
            ..ordinary_clean
        },
        ContinueEligibilityFacts {
            worktree_is_clean: false,
            ..ordinary_clean
        },
    ] {
        assert!(!continue_eligibility(facts, Vec::new()).eligible);
    }
}

#[test]
fn conflict_eligibility_requires_matching_native_state_and_staged_resolution() {
    let ready = ContinueEligibilityFacts {
        conflicted: true,
        native_merge_matches: true,
        conflict_has_unresolved_or_unstaged: false,
        repository_is_clean: false,
        worktree_is_clean: false,
    };
    assert!(continue_eligibility(ready, Vec::new()).eligible);

    let native_mismatch = ContinueEligibilityFacts {
        native_merge_matches: false,
        ..ready
    };
    assert!(!continue_eligibility(native_mismatch, Vec::new()).eligible);

    let unresolved = ContinueEligibilityFacts {
        conflict_has_unresolved_or_unstaged: true,
        ..ready
    };
    assert_eq!(
        continue_eligibility(unresolved, Vec::new()),
        RetryEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::IndexModified],
        }
    );
    assert_eq!(
        continue_eligibility(
            unresolved,
            vec![
                ParticipantDriftKind::MergeHeadChanged,
                ParticipantDriftKind::IndexModified,
            ],
        ),
        RetryEligibility {
            eligible: false,
            blockers: vec![
                ParticipantDriftKind::MergeHeadChanged,
                ParticipantDriftKind::IndexModified,
            ],
        }
    );
}

#[test]
fn shared_blockers_and_missing_repository_policy_are_stable() {
    assert_eq!(
        missing_repository_continue_eligibility(),
        RetryEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::RepositoryMissing],
        }
    );

    let mut eligibility = RetryEligibility {
        eligible: true,
        blockers: vec![ParticipantDriftKind::BranchChanged],
    };
    block_continue(&mut eligibility, ParticipantDriftKind::IndexModified);
    block_continue(&mut eligibility, ParticipantDriftKind::IndexModified);
    assert_eq!(
        eligibility,
        RetryEligibility {
            eligible: false,
            blockers: vec![
                ParticipantDriftKind::BranchChanged,
                ParticipantDriftKind::IndexModified,
            ],
        }
    );
}
