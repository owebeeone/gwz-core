use std::collections::BTreeMap;
use std::fs;

use crate::git::{Git2Backend, GitBackend, GitPreparedCommit, GitPreparedMerge};
use crate::model::ErrorCode;

use super::{
    MergeParticipantRecord, MergeTargetKind, OperationState, ParticipantState, PendingCommitSpec,
    PendingGitSignature, PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};

#[rustfmt::skip]
const OPERATION_STATES: [OperationState; 9] = [
    OperationState::Executing, OperationState::AwaitingResolution, OperationState::Halted,
    OperationState::Finalizing, OperationState::Preserving, OperationState::RollingBack,
    OperationState::Completed, OperationState::Aborted, OperationState::RecoveryRequired,
];

#[rustfmt::skip]
const PARTICIPANT_STATES: [ParticipantState; 10] = [
    ParticipantState::Planned, ParticipantState::UpToDate, ParticipantState::FastForwarded,
    ParticipantState::Merged, ParticipantState::Conflicted, ParticipantState::Failed,
    ParticipantState::Unattempted, ParticipantState::Continued, ParticipantState::Aborted,
    ParticipantState::RolledBack,
];

#[test]
fn v0_operation_transition_matrix_is_exhaustive() {
    use OperationState as S;
    #[rustfmt::skip]
    let accepted: &[(S, &[S])] = &[
        (S::Executing, &[S::Executing, S::AwaitingResolution, S::Halted, S::Finalizing, S::Preserving, S::RecoveryRequired]),
        (S::AwaitingResolution, &[S::AwaitingResolution, S::Executing, S::Finalizing, S::Preserving, S::RollingBack, S::RecoveryRequired]),
        (S::Halted, &[S::Halted, S::Executing, S::Preserving, S::RollingBack, S::RecoveryRequired]),
        (S::Finalizing, &[S::Finalizing, S::Completed, S::Preserving, S::RollingBack, S::RecoveryRequired]),
        (S::Preserving, &[S::Preserving, S::RollingBack, S::RecoveryRequired]),
        (S::RollingBack, &[S::RollingBack, S::Aborted, S::RecoveryRequired]),
        (S::Completed, &[S::Completed]),
        (S::Aborted, &[S::Aborted]),
        (S::RecoveryRequired, &[S::RecoveryRequired, S::Executing, S::RollingBack, S::Preserving]),
    ];

    assert_eq!(accepted.len(), OPERATION_STATES.len());
    for from in OPERATION_STATES {
        let legal = accepted
            .iter()
            .find_map(|(state, next)| (*state == from).then_some(*next))
            .unwrap();
        for to in OPERATION_STATES {
            let result = from.transition(to);
            assert_eq!(result.is_ok(), legal.contains(&to), "{from:?} -> {to:?}");
            if legal.contains(&to) {
                assert_eq!(result.unwrap(), to);
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::MergeRecoveryRequired);
            }
        }
    }
}

#[test]
fn v0_participant_transition_matrix_is_exhaustive() {
    use ParticipantState as S;
    let attempted = [
        S::UpToDate,
        S::FastForwarded,
        S::Merged,
        S::Conflicted,
        S::Failed,
    ];
    let accepted: &[(S, &[S])] = &[
        (
            S::Planned,
            &[
                S::Planned,
                S::UpToDate,
                S::FastForwarded,
                S::Merged,
                S::Conflicted,
                S::Failed,
                S::Unattempted,
                S::Aborted,
            ],
        ),
        (S::Unattempted, &[S::Unattempted]),
        (S::Failed, &[]),
        (S::UpToDate, &[S::UpToDate, S::Aborted]),
        (S::FastForwarded, &[S::FastForwarded, S::RolledBack]),
        (S::Merged, &[S::Merged, S::RolledBack]),
        (S::Conflicted, &[S::Conflicted, S::Continued, S::Aborted]),
        (S::Continued, &[S::Continued, S::RolledBack]),
        (S::Aborted, &[S::Aborted]),
        (S::RolledBack, &[S::RolledBack]),
    ];

    assert_eq!(accepted.len(), PARTICIPANT_STATES.len());
    for from in PARTICIPANT_STATES {
        let explicit = accepted
            .iter()
            .find_map(|(state, next)| (*state == from).then_some(*next))
            .unwrap();
        for to in PARTICIPANT_STATES {
            let legal = explicit.contains(&to)
                || matches!(from, S::Unattempted | S::Failed)
                    && (attempted.contains(&to) || to == S::Aborted);
            let result = from.transition(to);
            assert_eq!(result.is_ok(), legal, "{from:?} -> {to:?}");
            if legal {
                assert_eq!(result.unwrap(), to);
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::MergeRecoveryRequired);
            }
        }
    }
}

#[test]
fn v0_pending_action_decode_matrix_is_closed() {
    use PendingMergeActionKind as K;
    use PendingMergeExpectedResult as R;
    let kinds = [
        K::VerifyUpToDate,
        K::FastForward,
        K::TrueMerge,
        K::ResolveConflict,
    ];
    let results = [
        None,
        Some(R::Unchanged),
        Some(R::FastForward),
        Some(R::ExpectedConflict),
        Some(R::Commit),
    ];

    for kind in kinds {
        for expected_result in results {
            for with_spec in [false, true] {
                let action = pending_action(kind, expected_result, with_spec);
                let legal = matches!(
                    (kind, expected_result, with_spec),
                    (K::VerifyUpToDate, None | Some(R::Unchanged), false)
                        | (K::FastForward, None | Some(R::FastForward), false)
                        | (K::TrueMerge, Some(R::ExpectedConflict), false)
                        | (K::TrueMerge | K::ResolveConflict, Some(R::Commit), true)
                );
                assert_eq!(
                    super::pending::decode_durable_prepared_action(&action).is_ok(),
                    legal,
                    "kind={kind:?} result={expected_result:?} with_spec={with_spec}"
                );
            }
        }
    }
}

#[test]
fn v0_pending_shapes_have_a_closed_observation_matrix() {
    use super::PendingActionObservationState as O;

    let mut observed = BTreeMap::<&str, u8>::new();
    exercise_unchanged(&mut observed);
    exercise_fast_forward(&mut observed);
    exercise_true_merge(false, &mut observed);
    exercise_true_merge(true, &mut observed);
    exercise_resolution(&mut observed);

    let expected = [
        ("verify_up_to_date", mask(&[O::NotStarted, O::Ambiguous])),
        (
            "fast_forward",
            mask(&[O::NotStarted, O::CompletedExactly, O::Ambiguous]),
        ),
        (
            "true_merge_conflict",
            mask(&[O::NotStarted, O::ExpectedConflict, O::Ambiguous]),
        ),
        (
            "true_merge_commit",
            mask(&[O::NotStarted, O::CompletedExactly, O::Ambiguous]),
        ),
        (
            "resolve_conflict",
            mask(&[O::NotStarted, O::CompletedExactly, O::Ambiguous]),
        ),
    ];
    assert_eq!(observed, BTreeMap::from(expected));
}

fn exercise_unchanged(observed: &mut BTreeMap<&'static str, u8>) {
    use PendingMergeActionKind as K;
    use PendingMergeExpectedResult as R;
    let (root, repo) = repository("pending-verify");
    let before = commit(&repo, "base.txt", "base\n", "base");
    let record = participant(
        K::VerifyUpToDate,
        Some(R::Unchanged),
        false,
        &before,
        &before,
    );
    record_observation("verify_up_to_date", &root, &record, observed);
    record_ambiguous("verify_up_to_date", &root, &record, observed);
}

fn exercise_fast_forward(observed: &mut BTreeMap<&'static str, u8>) {
    use PendingMergeActionKind as K;
    use PendingMergeExpectedResult as R;
    let (root, repo) = repository("pending-ff");
    let before = commit(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source = commit(&repo, "source.txt", "source\n", "source");
    run_git(&repo, &["checkout", "main"]);
    let record = participant(
        K::FastForward,
        Some(R::FastForward),
        false,
        &before,
        &source,
    );
    record_observation("fast_forward", &root, &record, observed);
    record_ambiguous("fast_forward", &root, &record, observed);
    let backend = Git2Backend::new();
    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, "merge", &prepared,
        )
        .unwrap();
    record_observation("fast_forward", &root, &record, observed);
}

fn exercise_true_merge(conflict: bool, observed: &mut BTreeMap<&'static str, u8>) {
    use PendingMergeActionKind as K;
    use PendingMergeExpectedResult as R;
    let name = if conflict {
        "true_merge_conflict"
    } else {
        "true_merge_commit"
    };
    let (root, repo, before, source) = divergent_repository(name, conflict);
    let backend = Git2Backend::new();
    let prepared = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let mut record = participant(K::TrueMerge, None, false, &before, &source);
    match &prepared {
        GitPreparedMerge::ExpectedConflict => {
            record.pending_action.as_mut().unwrap().expected_result = Some(R::ExpectedConflict);
        }
        GitPreparedMerge::Commit(spec) => set_commit_spec(&mut record, spec),
        other => panic!("divergent fixture produced {other:?}"),
    }
    record_observation(name, &root, &record, observed);
    record_ambiguous(name, &root, &record, observed);
    backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, "merge", &prepared,
        )
        .unwrap();
    record_observation(name, &root, &record, observed);
}

fn exercise_resolution(observed: &mut BTreeMap<&'static str, u8>) {
    use PendingMergeActionKind as K;
    let (root, repo, before, source) = divergent_repository("pending-resolution", true);
    let backend = Git2Backend::new();
    let conflict = backend
        .prepare_merge_upstream_checked(&repo, "main", &before, &source, None)
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &repo, "main", &before, &source, "merge", &conflict,
        )
        .unwrap();
    fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
    run_git(&repo, &["add", "shared.txt"]);
    let prepared = backend
        .prepare_merge_resolution_checked(&repo, "main", &before, &source, None)
        .unwrap();
    let mut record = participant(K::ResolveConflict, None, false, &before, &source);
    record.state = ParticipantState::Conflicted;
    record.expected_merge_head = Some(source.clone());
    set_commit_spec(&mut record, &prepared);
    record_observation("resolve_conflict", &root, &record, observed);
    record_ambiguous("resolve_conflict", &root, &record, observed);
    backend
        .commit_prepared_merge_resolution_checked(
            &repo, "main", &before, &source, "merge", &prepared,
        )
        .unwrap();
    record_observation("resolve_conflict", &root, &record, observed);
}

fn participant(
    kind: PendingMergeActionKind,
    expected: Option<PendingMergeExpectedResult>,
    with_spec: bool,
    before: &str,
    source: &str,
) -> MergeParticipantRecord {
    let mut action = pending_action(kind, expected, with_spec);
    action.before_commit = before.into();
    action.source_commit = source.into();
    MergeParticipantRecord {
        path: "repos/app".into(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".into(),
        before_commit: before.into(),
        source_commit: source.into(),
        commit_message: "merge".into(),
        state: ParticipantState::Planned,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: Some(action),
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn pending_action(
    kind: PendingMergeActionKind,
    expected_result: Option<PendingMergeExpectedResult>,
    with_spec: bool,
) -> PendingMergeAction {
    PendingMergeAction {
        kind,
        target_branch: "main".into(),
        before_commit: "before".into(),
        source_commit: "source".into(),
        commit_message: "merge".into(),
        expected_result,
        commit_spec: with_spec.then(commit_spec),
        extensions: BTreeMap::new(),
    }
}

fn commit_spec() -> PendingCommitSpec {
    let signature = PendingGitSignature {
        name: "GWZ".into(),
        email: "gwz@example.invalid".into(),
        time_seconds: 1,
        timezone_offset_minutes: 0,
        extensions: BTreeMap::new(),
    };
    PendingCommitSpec {
        tree_oid: "tree".into(),
        author: signature.clone(),
        committer: signature,
        extensions: BTreeMap::new(),
    }
}

fn set_commit_spec(record: &mut MergeParticipantRecord, prepared: &GitPreparedCommit) {
    let pending = record.pending_action.as_mut().unwrap();
    pending.expected_result = Some(PendingMergeExpectedResult::Commit);
    pending.commit_spec = Some(PendingCommitSpec {
        tree_oid: prepared.tree_oid.clone(),
        author: signature(&prepared.author),
        committer: signature(&prepared.committer),
        extensions: BTreeMap::new(),
    });
}

fn signature(value: &crate::git::GitPreparedSignature) -> PendingGitSignature {
    PendingGitSignature {
        name: value.name.clone(),
        email: value.email.clone(),
        time_seconds: value.time_seconds,
        timezone_offset_minutes: value.timezone_offset_minutes,
        extensions: BTreeMap::new(),
    }
}

fn record_observation(
    name: &'static str,
    root: &crate::workspace_ops::tests::TempDir,
    record: &MergeParticipantRecord,
    observed: &mut BTreeMap<&'static str, u8>,
) {
    let state =
        super::status::observe_participant(&Git2Backend::new(), root.path(), "mem_app", record)
            .unwrap()
            .pending_action
            .unwrap()
            .state;
    *observed.entry(name).or_default() |= bit(state);
}

fn record_ambiguous(
    name: &'static str,
    root: &crate::workspace_ops::tests::TempDir,
    record: &MergeParticipantRecord,
    observed: &mut BTreeMap<&'static str, u8>,
) {
    let mut mismatched = record.clone();
    mismatched.pending_action.as_mut().unwrap().target_branch = "other".into();
    record_observation(name, root, &mismatched, observed);
}

fn mask(states: &[super::PendingActionObservationState]) -> u8 {
    states.iter().fold(0, |value, state| value | bit(*state))
}

fn bit(state: super::PendingActionObservationState) -> u8 {
    use super::PendingActionObservationState as O;
    match state {
        O::NotStarted => 1,
        O::ExpectedConflict => 2,
        O::CompletedExactly => 4,
        O::Ambiguous => 8,
    }
}

fn repository(name: &str) -> (crate::workspace_ops::tests::TempDir, std::path::PathBuf) {
    let root = crate::workspace_ops::tests::TempDir::new(name);
    let repo = root.path().join("repos/app");
    Git2Backend::new().create_repo(&repo).unwrap();
    (root, repo)
}

fn divergent_repository(
    name: &str,
    conflict: bool,
) -> (
    crate::workspace_ops::tests::TempDir,
    std::path::PathBuf,
    String,
    String,
) {
    let (root, repo) = repository(name);
    commit(&repo, "shared.txt", "base\n", "base");
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["checkout", "feature"]);
    let source_file = if conflict { "shared.txt" } else { "source.txt" };
    let source = commit(&repo, source_file, "source\n", "source");
    run_git(&repo, &["checkout", "main"]);
    let before_file = if conflict { "shared.txt" } else { "main.txt" };
    let before = commit(&repo, before_file, "main\n", "main");
    (root, repo, before, source)
}

fn commit(repo: &std::path::Path, file: &str, content: &str, message: &str) -> String {
    fs::write(repo.join(file), content).unwrap();
    run_git(repo, &["add", file]);
    run_git(repo, &["commit", "-m", message]);
    run_git(repo, &["rev-parse", "HEAD"])
}

fn run_git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=GWZ",
            "-c",
            "user.email=gwz@example.invalid",
        ])
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
