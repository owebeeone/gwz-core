use crate::workspace_ops::merge::{PendingMergeAction, PendingMergeActionKind};

use super::*;

pub(super) fn pending_record(
    state: ParticipantState,
    before: &str,
    source: &str,
    message: &str,
    kind: PendingMergeActionKind,
) -> MergeParticipantRecord {
    let mut record = participant(state);
    record.before_commit = before.to_owned();
    record.source_commit = source.to_owned();
    record.commit_message = message.to_owned();
    record.pending_action = Some(PendingMergeAction {
        kind,
        target_branch: "main".to_owned(),
        before_commit: before.to_owned(),
        source_commit: source.to_owned(),
        commit_message: message.to_owned(),
        expected_result: None,
        commit_spec: None,
        extensions: BTreeMap::new(),
    });
    record
}

pub(super) fn participant(state: ParticipantState) -> MergeParticipantRecord {
    let yaml = format!(
        "path: repos/app\ntarget_kind: member\ntarget_branch: main\nbefore_commit: before\nsource_commit: source\ncommit_message: merge\nstate: {}\n",
        serde_yaml::to_string(&state).unwrap().trim()
    );
    serde_yaml::from_str(&yaml).unwrap()
}

#[test]
fn unattempted_post_plan_work_is_structured_and_blocks_recovery() {
    let record = participant(ParticipantState::Unattempted);
    let live = ParticipantLiveState {
        branch: Some("main".into()),
        head: Some("later".into()),
        target_ref: Some("later".into()),
        status: GitStatus {
            is_dirty: true,
            unstaged: 1,
            ..GitStatus::clean()
        },
        repository_state: GitRepositoryState::Clean,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Advanced,
    };
    let observed = classify_participant("mem_app", &record, &live);
    let kinds: Vec<_> = observed.drift.iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ParticipantDriftKind::TargetRefChanged,
            ParticipantDriftKind::HeadAdvanced,
            ParticipantDriftKind::WorktreeModified,
        ]
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(observed.abort_eligibility.eligible);
    assert!(observed.drift[1].message.contains("or abort"));
}

#[test]
fn missing_unattempted_repo_is_visible_but_does_not_block_abort() {
    let observed = missing_observation("mem_app", &participant(ParticipantState::Unattempted));
    assert_eq!(
        observed.drift[0].kind,
        ParticipantDriftKind::RepositoryMissing
    );
    assert!(observed.abort_eligibility.eligible);
}

#[test]
fn planned_drift_fails_closed_after_an_ambiguous_crash_window() {
    let record = participant(ParticipantState::Planned);
    let live = ParticipantLiveState {
        branch: Some("main".into()),
        head: Some("later".into()),
        target_ref: Some("later".into()),
        status: GitStatus::clean(),
        repository_state: GitRepositoryState::Clean,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Advanced,
    };
    let observed = classify_participant("mem_app", &record, &live);
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);
    assert!(
        observed
            .abort_eligibility
            .blockers
            .contains(&ParticipantDriftKind::HeadAdvanced)
    );
}

#[test]
fn divergent_head_has_its_own_structured_drift() {
    let record = participant(ParticipantState::Merged);
    let live = ParticipantLiveState {
        branch: Some("main".into()),
        head: Some("other-line".into()),
        target_ref: Some("other-line".into()),
        status: GitStatus::clean(),
        repository_state: crate::git::GitRepositoryState::Clean,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Diverged,
    };
    let observed = classify_participant("mem_app", &record, &live);

    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::HeadDiverged)
    );
    assert!(!observed.continue_eligibility.eligible);
    assert!(!observed.abort_eligibility.eligible);
}

#[test]
fn foreign_native_state_blocks_rows_that_require_mutation() {
    let record = participant(ParticipantState::Merged);
    let live = ParticipantLiveState {
        branch: Some("main".into()),
        head: None,
        target_ref: None,
        status: GitStatus::clean(),
        repository_state: crate::git::GitRepositoryState::CherryPick,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Missing,
    };
    let observed = classify_participant("mem_app", &record, &live);

    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::ForeignIntegrationState)
    );
    assert!(!observed.abort_eligibility.eligible);
}

#[test]
fn externally_restored_conflict_is_abort_eligible() {
    let record = participant(ParticipantState::Conflicted);
    let live = ParticipantLiveState {
        branch: Some("main".into()),
        head: Some("before".into()),
        target_ref: Some("before".into()),
        status: GitStatus::clean(),
        repository_state: crate::git::GitRepositoryState::Clean,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Equal,
    };
    let observed = classify_participant("mem_app", &record, &live);

    assert!(!observed.continue_eligibility.eligible);
    assert!(observed.abort_eligibility.eligible);
    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::MergeStateMissing)
    );
}

#[test]
fn durably_restored_row_ignores_later_worktree_dirt_for_abort() {
    let record = participant(ParticipantState::RolledBack);
    let live = ParticipantLiveState {
        branch: Some("other".into()),
        head: Some("later".into()),
        target_ref: Some("before".into()),
        status: GitStatus {
            is_dirty: true,
            staged: 1,
            untracked: 1,
            ..GitStatus::clean()
        },
        repository_state: crate::git::GitRepositoryState::Clean,
        merge_state: None,
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Advanced,
    };
    let observed = classify_participant("mem_app", &record, &live);

    assert!(observed.abort_eligibility.eligible);
    assert!(
        observed
            .drift
            .iter()
            .any(|drift| drift.kind == ParticipantDriftKind::WorktreeModified)
    );
}

#[test]
fn invalid_record_path_is_rejected_before_repository_observation() {
    let root = std::env::temp_dir().join(format!("gwz-status-invalid-path-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let backend = crate::git::Git2Backend::new();
    for invalid in ["../outside", "/tmp/outside"] {
        let mut record = participant(ParticipantState::Unattempted);
        record.path = invalid.to_owned();
        let error = observe_participant(&backend, &root, "mem_app", &record).unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
        assert_eq!(error.member_id.as_deref(), Some("mem_app"));
        assert_eq!(error.member_path.as_deref(), Some(invalid));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rewound_detached_and_missing_heads_have_distinct_evidence() {
    let record = participant(ParticipantState::Unattempted);
    let cases = [
        (
            Some("main"),
            Some("older"),
            HeadRelation::Rewound,
            ParticipantDriftKind::HeadRewound,
        ),
        (
            None,
            Some("other"),
            HeadRelation::Advanced,
            ParticipantDriftKind::HeadAdvanced,
        ),
        (
            Some("main"),
            None,
            HeadRelation::Missing,
            ParticipantDriftKind::ObjectMissing,
        ),
    ];
    for (branch, head, relation, expected) in cases {
        let live = ParticipantLiveState {
            branch: branch.map(str::to_owned),
            head: head.map(str::to_owned),
            target_ref: head.map(str::to_owned),
            status: GitStatus::clean(),
            repository_state: GitRepositoryState::Clean,
            merge_state: None,
            native_detail_error: None,
            missing_objects: Vec::new(),
            head_relation: relation,
        };
        let observed = classify_participant("mem_app", &record, &live);
        assert!(observed.drift.iter().any(|drift| drift.kind == expected));
        if branch.is_none() {
            assert!(
                observed
                    .drift
                    .iter()
                    .any(|drift| drift.kind == ParticipantDriftKind::BranchChanged)
            );
        }
    }
}
