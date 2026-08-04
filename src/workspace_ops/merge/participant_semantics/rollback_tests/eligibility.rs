use super::*;

fn facts() -> RollbackEligibilityFacts {
    RollbackEligibilityFacts {
        native_merge_matches: false,
        exact_before_clean: false,
        target_ref_matches_before: false,
        repository_is_clean: false,
        worktree_is_clean: false,
    }
}

#[test]
fn eligibility_with_adverse_facts_only_accepts_no_owned_mutation_states() {
    let kinds = vec![ParticipantDriftKind::ForeignIntegrationState];
    for state in STATES {
        let eligibility = rollback_eligibility(state, facts(), kinds.clone());
        let expected = matches!(rollback_class(state), RollbackClass::NoOwnedMutation);
        assert_eq!(eligibility.eligible, expected, "state={state:?}");
        assert_eq!(
            eligibility.blockers,
            if expected { Vec::new() } else { kinds.clone() },
            "state={state:?}"
        );
    }
}

#[test]
fn clean_drift_free_rows_are_eligible_for_abort() {
    let facts = RollbackEligibilityFacts {
        native_merge_matches: true,
        exact_before_clean: true,
        repository_is_clean: true,
        worktree_is_clean: true,
        target_ref_matches_before: true,
    };
    for state in STATES {
        let eligibility = rollback_eligibility(state, facts, Vec::new());
        assert!(eligibility.eligible, "state={state:?}");
        assert!(eligibility.blockers.is_empty(), "state={state:?}");
    }
}

#[test]
fn conflict_and_integrated_restore_exceptions_retain_existing_behavior() {
    let conflict = rollback_eligibility(
        ParticipantState::Conflicted,
        RollbackEligibilityFacts {
            exact_before_clean: true,
            ..facts()
        },
        vec![ParticipantDriftKind::MergeStateMissing],
    );
    assert_eq!(
        conflict,
        RollbackEligibility {
            eligible: true,
            blockers: Vec::new()
        }
    );

    for state in [
        ParticipantState::FastForwarded,
        ParticipantState::Merged,
        ParticipantState::Continued,
    ] {
        let integrated = rollback_eligibility(
            state,
            RollbackEligibilityFacts {
                exact_before_clean: true,
                ..facts()
            },
            vec![
                ParticipantDriftKind::TargetRefChanged,
                ParticipantDriftKind::HeadRewound,
            ],
        );
        assert_eq!(
            integrated,
            RollbackEligibility {
                eligible: true,
                blockers: Vec::new()
            },
            "state={state:?}"
        );
    }
}

#[test]
fn native_conflict_requires_matching_state_and_no_drift() {
    let matching = RollbackEligibilityFacts {
        native_merge_matches: true,
        ..facts()
    };
    assert!(rollback_eligibility(ParticipantState::Conflicted, matching, Vec::new()).eligible);
    let blocked = rollback_eligibility(
        ParticipantState::Conflicted,
        matching,
        vec![ParticipantDriftKind::MergeHeadChanged],
    );
    assert_eq!(
        blocked,
        RollbackEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::MergeHeadChanged],
        }
    );
}

#[test]
fn planned_and_failed_require_the_exact_before_fact() {
    let apparently_clean = RollbackEligibilityFacts {
        repository_is_clean: true,
        worktree_is_clean: true,
        ..facts()
    };
    for state in [ParticipantState::Planned, ParticipantState::Failed] {
        assert!(
            !rollback_eligibility(state, apparently_clean, Vec::new()).eligible,
            "state={state:?}"
        );
        assert!(
            rollback_eligibility(
                state,
                RollbackEligibilityFacts {
                    exact_before_clean: true,
                    ..apparently_clean
                },
                Vec::new()
            )
            .eligible,
            "state={state:?}"
        );
    }
}

#[test]
fn complete_rows_accept_verified_target_restore_despite_later_worktree_changes() {
    for state in [ParticipantState::Aborted, ParticipantState::RolledBack] {
        let eligibility = rollback_eligibility(
            state,
            RollbackEligibilityFacts {
                target_ref_matches_before: true,
                ..facts()
            },
            vec![ParticipantDriftKind::WorktreeModified],
        );
        assert_eq!(
            eligibility,
            RollbackEligibility {
                eligible: true,
                blockers: Vec::new()
            },
            "state={state:?}"
        );
    }
}

#[test]
fn missing_repository_table_covers_every_state_and_pending_status() {
    for state in STATES {
        for has_pending in [false, true] {
            let eligibility = missing_repository_abort_eligibility(state, has_pending);
            let expected = !has_pending && rollback_class(state) == RollbackClass::NoOwnedMutation;
            assert_eq!(
                eligibility,
                RollbackEligibility {
                    eligible: expected,
                    blockers: if expected {
                        Vec::new()
                    } else {
                        vec![ParticipantDriftKind::RepositoryMissing]
                    },
                },
                "state={state:?}, has_pending={has_pending}"
            );
        }
    }
}

#[test]
fn block_abort_disables_and_deduplicates_the_appended_blocker() {
    let mut eligibility = RollbackEligibility {
        eligible: true,
        blockers: vec![ParticipantDriftKind::WorktreeModified],
    };
    block_abort(&mut eligibility, ParticipantDriftKind::IndexModified);
    block_abort(&mut eligibility, ParticipantDriftKind::IndexModified);
    assert_eq!(
        eligibility,
        RollbackEligibility {
            eligible: false,
            blockers: vec![
                ParticipantDriftKind::WorktreeModified,
                ParticipantDriftKind::IndexModified,
            ],
        }
    );
}
