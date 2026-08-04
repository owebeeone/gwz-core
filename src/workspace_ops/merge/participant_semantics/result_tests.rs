use super::*;

#[derive(Clone, Copy)]
struct Case {
    state: ParticipantState,
    wire: crate::MergeParticipantState,
    count: ParticipantCount,
    class: ParticipantResultClass,
    changed_when_result_is_not_before: bool,
}

const CASES: [Case; 10] = [
    Case {
        state: ParticipantState::Planned,
        wire: crate::MergeParticipantState::Planned,
        count: ParticipantCount::Planned,
        class: ParticipantResultClass::None,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::UpToDate,
        wire: crate::MergeParticipantState::UpToDate,
        count: ParticipantCount::UpToDate,
        class: ParticipantResultClass::SuccessfulUnchanged,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::FastForwarded,
        wire: crate::MergeParticipantState::FastForwarded,
        count: ParticipantCount::FastForwarded,
        class: ParticipantResultClass::Integrated,
        changed_when_result_is_not_before: true,
    },
    Case {
        state: ParticipantState::Merged,
        wire: crate::MergeParticipantState::Merged,
        count: ParticipantCount::Merged,
        class: ParticipantResultClass::Integrated,
        changed_when_result_is_not_before: true,
    },
    Case {
        state: ParticipantState::Conflicted,
        wire: crate::MergeParticipantState::Conflicted,
        count: ParticipantCount::Conflicted,
        class: ParticipantResultClass::Conflict,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::Failed,
        wire: crate::MergeParticipantState::Failed,
        count: ParticipantCount::Failed,
        class: ParticipantResultClass::None,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::Unattempted,
        wire: crate::MergeParticipantState::Unattempted,
        count: ParticipantCount::Unattempted,
        class: ParticipantResultClass::None,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::Continued,
        wire: crate::MergeParticipantState::Continued,
        count: ParticipantCount::Continued,
        class: ParticipantResultClass::Integrated,
        changed_when_result_is_not_before: true,
    },
    Case {
        state: ParticipantState::Aborted,
        wire: crate::MergeParticipantState::Aborted,
        count: ParticipantCount::Aborted,
        class: ParticipantResultClass::None,
        changed_when_result_is_not_before: false,
    },
    Case {
        state: ParticipantState::RolledBack,
        wire: crate::MergeParticipantState::RolledBack,
        count: ParticipantCount::RolledBack,
        class: ParticipantResultClass::None,
        changed_when_result_is_not_before: false,
    },
];

#[test]
fn every_state_has_one_wire_count_and_result_class() {
    for case in CASES {
        assert_eq!(wire_state(case.state), case.wire);
        assert_eq!(count_projection(case.state), case.count);
        assert_eq!(result_class(case.state), case.class);
        assert_eq!(is_successful_result(case.state), case.class.is_successful());
        assert_eq!(is_integrated_result(case.state), case.class.is_integrated());
        assert_eq!(is_conflicted_result(case.state), case.class.is_conflict());
    }
}

#[test]
fn every_state_has_one_count_destination() {
    for case in CASES {
        let mut counts = crate::MergeParticipantCounts::default();
        increment_count(&mut counts, case.state);
        assert_eq!(count_value(&counts, case.count), 1);
        assert_eq!(count_sum(&counts), 1);
    }
}

#[test]
fn changed_result_requires_an_integrated_state_and_a_nonmatching_result() {
    for case in CASES {
        let same = participant(case.state, Some("before"));
        let different = participant(case.state, Some("after"));
        let missing = participant(case.state, None);

        assert!(!has_changed_result(&same));
        assert_eq!(
            has_changed_result(&different),
            case.changed_when_result_is_not_before
        );
        assert_eq!(
            has_changed_result(&missing),
            case.changed_when_result_is_not_before
        );
    }
}

fn participant(state: ParticipantState, resulting_commit: Option<&str>) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: "repo".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: "before".to_owned(),
        source_commit: "source".to_owned(),
        commit_message: "message".to_owned(),
        state,
        resulting_commit: resulting_commit.map(str::to_owned),
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: Default::default(),
    }
}

fn count_value(counts: &crate::MergeParticipantCounts, count: ParticipantCount) -> i64 {
    match count {
        ParticipantCount::Planned => counts.planned,
        ParticipantCount::UpToDate => counts.up_to_date,
        ParticipantCount::FastForwarded => counts.fast_forwarded,
        ParticipantCount::Merged => counts.merged,
        ParticipantCount::Conflicted => counts.conflicted,
        ParticipantCount::Failed => counts.failed,
        ParticipantCount::Unattempted => counts.unattempted,
        ParticipantCount::Continued => counts.continued,
        ParticipantCount::Aborted => counts.aborted,
        ParticipantCount::RolledBack => counts.rolled_back,
    }
}

fn count_sum(counts: &crate::MergeParticipantCounts) -> i64 {
    counts.planned
        + counts.up_to_date
        + counts.fast_forwarded
        + counts.merged
        + counts.conflicted
        + counts.failed
        + counts.unattempted
        + counts.continued
        + counts.aborted
        + counts.rolled_back
}
