use std::fs;

use sha2::Digest;

use super::super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, ObservationKind,
    PhysicalActionKind, V1LifecycleRequest,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::forward::ForwardRuntime;
use super::super::service::{ExactObserver, PhysicalExecutor};
use super::super::store::CheckedV1Store;
use crate::artifact::{LOCK_PATH, ManifestArtifact};
use crate::git::{Git2Backend, GitBackend};
use crate::model::{ErrorCode, ModelResult};
use crate::operation::{ActionKind, OperationContext};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::MergeExecutionMode;
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, ParticipantRollbackKindV1, PendingRollbackActionV1, RecoveryContextV1,
    RecoveryOriginStateV1, test_record,
};
use crate::workspace_ops::merge::{
    OperationState, ParticipantState, PendingMergeAction, PendingMergeActionKind,
    PendingMergeExpectedResult,
};
use crate::workspace_ops::tests::{TempDir, commit_file};

#[test]
fn concrete_forward_runtime_fast_forwards_and_finishes_through_finalization() {
    let fixture = fixture("merge-v1-forward-fast-forward", Kind::FastForward);
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(row.state, ParticipantState::FastForwarded);
    assert_eq!(
        row.resulting_commit.as_deref(),
        Some(fixture.source.as_str())
    );
    assert!(row.pending_action.is_none());
    assert_eq!(
        fixture
            .backend
            .head(&fixture.member)
            .unwrap()
            .commit
            .as_deref(),
        Some(fixture.source.as_str())
    );
}

#[test]
fn up_to_date_action_is_adopted_without_participant_execution() {
    let mut fixture = fixture("merge-v1-forward-up-to-date", Kind::FastForward);
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .source_commit = fixture.before.clone();
    seed_open(&fixture);
    let context = context();
    let mut runtime = CountingRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(
        response.current().record().participants["mem_a"].state,
        ParticipantState::UpToDate
    );
    assert_eq!(runtime.executions, 0, "no Git action is required");
}

#[test]
fn no_ff_fast_forward_creates_a_two_parent_merge_commit() {
    let mut fixture = fixture("merge-v1-forward-no-ff", Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Merged);
    let result = row.resulting_commit.as_deref().unwrap();
    let repository = git2::Repository::open(&fixture.member).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(result).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), fixture.before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), fixture.source);
}

#[test]
fn no_ff_restart_adopts_the_exact_prepared_merge_commit() {
    let mut fixture = fixture("merge-v1-forward-no-ff-restart", Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    seed_open(&fixture);
    let context = context();
    let mut crashing = CrashAfterParticipant {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&fixture, &mut crashing)
        }))
        .is_err()
    );
    let committed = fixture
        .backend
        .head(&fixture.member)
        .unwrap()
        .commit
        .unwrap();

    let mut resumed = CountingRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };
    let response = run(&fixture, &mut resumed).unwrap();

    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(
        response.current().record().participants["mem_a"]
            .resulting_commit
            .as_deref(),
        Some(committed.as_str())
    );
    assert_eq!(resumed.executions, 5, "only finalization actions execute");
}

#[test]
fn no_ff_up_to_date_adopts_verify_up_to_date_without_execution() {
    let mut fixture = fixture("merge-v1-forward-no-ff-up-to-date", Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .source_commit = fixture.before.clone();
    seed_open(&fixture);
    let context = context();
    let mut runtime = CountingRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };

    let response = run(&fixture, &mut runtime).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(row.state, ParticipantState::UpToDate);
    assert_eq!(
        row.resulting_commit.as_deref(),
        Some(fixture.before.as_str())
    );
    assert_eq!(
        runtime.executions, 0,
        "the no-ff up-to-date row requires no Git action"
    );
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str())
    );
}

#[test]
fn no_ff_clean_true_merge_matches_normal_mode_bytes() {
    let mut trees = Vec::new();
    for (index, mode) in [MergeExecutionMode::Normal, MergeExecutionMode::NoFf]
        .into_iter()
        .enumerate()
    {
        let mut fixture = fixture(
            &format!("merge-v1-forward-clean-true-merge-{index}"),
            Kind::CleanDivergent,
        );
        fixture.model.mode = mode;
        seed_open(&fixture);

        let frozen = freeze_without_mutation(&fixture);
        assert_eq!(frozen.kind, PendingMergeActionKind::TrueMerge);
        assert_eq!(
            frozen.expected_result,
            Some(PendingMergeExpectedResult::Commit)
        );
        let tree = frozen.commit_spec.as_ref().unwrap().tree_oid.clone();
        assert_ne!(
            tree,
            commit_tree(&fixture.member, &fixture.source),
            "a clean true merge freezes the merge-index tree, never the source tree"
        );

        let context = context();
        let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
        let response =
            run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();

        let row = &response.current().record().participants["mem_a"];
        assert_eq!(row.state, ParticipantState::Merged);
        let facts = commit_facts(&fixture.member, row.resulting_commit.as_deref().unwrap());
        assert_eq!(
            facts.parents,
            [fixture.before.clone(), fixture.source.clone()]
        );
        assert_eq!(facts.tree, tree);
        trees.push(tree);
    }
    assert_eq!(
        trees[0], trees[1],
        "the clean-true-merge matrix cell is mode-blind"
    );
}

#[test]
fn no_ff_true_merge_conflict_row_and_resolution_commit() {
    let mut rows = Vec::new();
    for (index, mode) in [MergeExecutionMode::Normal, MergeExecutionMode::NoFf]
        .into_iter()
        .enumerate()
    {
        let mut fixture = fixture(
            &format!("merge-v1-forward-no-ff-conflict-{index}"),
            Kind::Conflict,
        );
        fixture.model.mode = mode;
        seed_open(&fixture);
        let context = context();
        let mut stopping =
            CapturingRuntime::new(ForwardRuntime::new(&fixture.backend, &context), Crash::None);

        let stopped = run(&fixture, &mut stopping).unwrap();

        assert_eq!(
            stopped.current().record().state,
            OperationState::AwaitingResolution
        );
        assert_eq!(
            stopped.current().record().participants["mem_a"].state,
            ParticipantState::Conflicted
        );
        let conflict = stopping.frozen.take().expect("the conflict row is frozen");

        fs::write(fixture.member.join("README.md"), "resolved\n").unwrap();
        fixture
            .backend
            .stage_paths(&fixture.member, &["README.md"])
            .unwrap();
        let mut resolving =
            CapturingRuntime::new(ForwardRuntime::new(&fixture.backend, &context), Crash::None);
        let completed = run(&fixture, &mut resolving).unwrap();

        let row = &completed.current().record().participants["mem_a"];
        assert_eq!(row.state, ParticipantState::Continued);
        let resolution = resolving.frozen.take().expect("the resolution is frozen");
        let facts = commit_facts(&fixture.member, row.resulting_commit.as_deref().unwrap());
        assert_eq!(
            facts.parents,
            [fixture.before.clone(), fixture.source.clone()]
        );
        rows.push((conflict, resolution));
    }

    let (normal_conflict, normal_resolution) = &rows[0];
    let (no_ff_conflict, no_ff_resolution) = &rows[1];
    assert_eq!(no_ff_conflict.kind, PendingMergeActionKind::TrueMerge);
    assert_eq!(
        no_ff_conflict.expected_result,
        Some(PendingMergeExpectedResult::ExpectedConflict)
    );
    assert!(no_ff_conflict.commit_spec.is_none());
    assert_eq!(
        no_ff_resolution.kind,
        PendingMergeActionKind::ResolveConflict
    );
    for (no_ff, normal) in [
        (no_ff_conflict, normal_conflict),
        (no_ff_resolution, normal_resolution),
    ] {
        assert_eq!(no_ff.kind, normal.kind, "the divergent row is mode-blind");
        assert_eq!(no_ff.expected_result, normal.expected_result);
        assert_eq!(no_ff.commit_message, normal.commit_message);
        assert_eq!(
            no_ff.commit_spec.as_ref().map(|spec| &spec.tree_oid),
            normal.commit_spec.as_ref().map(|spec| &spec.tree_oid)
        );
    }
}

#[test]
fn no_ff_external_fast_forward_is_ambiguous_never_adopted() {
    let mut fixture = fixture("merge-v1-forward-no-ff-external-ff", Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    seed_open(&fixture);
    let frozen = freeze_without_mutation(&fixture);
    assert_eq!(frozen.kind, PendingMergeActionKind::TrueMerge);
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str())
    );

    // An external agent fast-forwards the target to the source commit while
    // the two-parent action is still pending (design §5.2).
    fixture
        .backend
        .fast_forward(&fixture.member, "main", &fixture.source)
        .unwrap();

    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    let response = run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_ne!(row.state, ParticipantState::Merged);
    assert!(
        row.resulting_commit.is_none(),
        "an external fast-forward is never adopted as the frozen two-parent result"
    );
    assert!(
        row.pending_action.is_some(),
        "the frozen action stays pending under ambiguity"
    );
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.source.as_str())
    );
}

#[test]
fn no_ff_preparation_persists_the_frozen_action_before_any_git_mutation() {
    let mut fixture = fixture("merge-v1-forward-no-ff-freeze-first", Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    seed_open(&fixture);

    let frozen = freeze_without_mutation(&fixture);

    assert_eq!(stored_action(&fixture).as_ref(), Some(&frozen));
    assert_eq!(frozen.kind, PendingMergeActionKind::TrueMerge);
    assert_eq!(
        frozen.expected_result,
        Some(PendingMergeExpectedResult::Commit)
    );
    assert_eq!(
        head_commit(&fixture).as_deref(),
        Some(fixture.before.as_str()),
        "the action is durable before any Git mutation"
    );
    let spec = frozen.commit_spec.clone().unwrap();

    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    let response = run_production(&fixture, &mut runtime, V1LifecycleRequest::Continue).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Merged);
    let facts = commit_facts(&fixture.member, row.resulting_commit.as_deref().unwrap());
    assert_eq!(
        facts.parents,
        [fixture.before.clone(), fixture.source.clone()]
    );
    assert_eq!(facts.tree, spec.tree_oid);
    assert_eq!(facts.author.2, spec.author.time_seconds);
    assert_eq!(facts.committer.2, spec.committer.time_seconds);
}

#[test]
fn ff_only_and_normal_matrices_are_unchanged_under_the_m5b_tree() {
    let rows = [
        (
            MergeExecutionMode::Normal,
            Kind::FastForward,
            Some(PendingMergeActionKind::FastForward),
            ParticipantState::FastForwarded,
        ),
        (
            MergeExecutionMode::Normal,
            Kind::CleanDivergent,
            Some(PendingMergeActionKind::TrueMerge),
            ParticipantState::Merged,
        ),
        (
            MergeExecutionMode::FfOnly,
            Kind::FastForward,
            Some(PendingMergeActionKind::FastForward),
            ParticipantState::FastForwarded,
        ),
        (
            MergeExecutionMode::NoFf,
            Kind::FastForward,
            Some(PendingMergeActionKind::TrueMerge),
            ParticipantState::Merged,
        ),
        (
            MergeExecutionMode::FfOnly,
            Kind::CleanDivergent,
            None,
            ParticipantState::Failed,
        ),
    ];
    for (index, (mode, kind, expected_kind, expected_state)) in rows.into_iter().enumerate() {
        let mut fixture = fixture(&format!("merge-v1-forward-mode-matrix-{index}"), kind);
        fixture.model.mode = mode;
        seed_open(&fixture);
        let context = context();
        let mut runtime =
            CapturingRuntime::new(ForwardRuntime::new(&fixture.backend, &context), Crash::None);

        let response = run(&fixture, &mut runtime).unwrap();

        let row = &response.current().record().participants["mem_a"];
        assert_eq!(row.state, expected_state, "row {index} ({mode:?}) state");
        match expected_kind {
            Some(kind) => {
                let frozen = runtime.frozen.as_ref().expect("an action was frozen");
                assert_eq!(frozen.kind, kind, "row {index} ({mode:?}) durable kind");
                assert_eq!(
                    frozen.commit_spec.is_some(),
                    kind == PendingMergeActionKind::TrueMerge,
                    "row {index} ({mode:?}) commit spec"
                );
            }
            None => {
                assert!(
                    runtime.frozen.is_none(),
                    "ff_only must never freeze a true-merge action"
                );
                assert_eq!(
                    response.current().record().state,
                    OperationState::Halted,
                    "row {index}"
                );
                assert_eq!(
                    row.error.as_ref().unwrap().code,
                    ErrorCode::MergeValidationFailed,
                    "row {index}"
                );
            }
        }
    }
}

#[test]
fn restart_after_git_mutation_adopts_the_exact_pending_result_without_reexecution() {
    let fixture = fixture("merge-v1-forward-restart", Kind::FastForward);
    seed_open(&fixture);
    let context = context();
    let mut crashing = CrashAfterParticipant {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&fixture, &mut crashing)
    }));
    assert!(crashed.is_err());
    assert_eq!(crashing.executions, 1);
    assert_eq!(
        fixture
            .backend
            .head(&fixture.member)
            .unwrap()
            .commit
            .as_deref(),
        Some(fixture.source.as_str())
    );
    let interrupted = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert!(
        interrupted.record().participants["mem_a"]
            .pending_action
            .is_some()
    );

    let mut resumed = CountingRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        executions: 0,
    };
    let response = run(&fixture, &mut resumed).unwrap();
    assert_eq!(response.current().record().state, OperationState::Completed);
    assert_eq!(resumed.executions, 5, "only finalization actions execute");
    assert_eq!(
        response.current().record().participants["mem_a"].state,
        ParticipantState::FastForwarded
    );
}

#[test]
fn ambiguous_pending_participant_enters_recovery_without_git_mutation() {
    let mut fixture = fixture("merge-v1-forward-ambiguous", Kind::FastForward);
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(fast_forward_action(&fixture.model));
    seed_open(&fixture);
    fs::write(fixture.member.join("untracked.txt"), "drift\n").unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        response
            .current()
            .record()
            .recovery_context
            .as_ref()
            .unwrap()
            .origin_state,
        RecoveryOriginStateV1::Executing
    );
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert_eq!(
        fs::read_to_string(fixture.member.join("untracked.txt")).unwrap(),
        "drift\n"
    );
}

#[test]
fn semantic_preparation_drift_enters_executing_recovery_before_owner_or_git_mutation() {
    let fixture = fixture("merge-v1-forward-preparation-drift", Kind::FastForward);
    seed_open(&fixture);
    fs::write(fixture.member.join("untracked.txt"), "drift\n").unwrap();
    let head_before = fixture.backend.head(&fixture.member).unwrap();
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        response
            .current()
            .record()
            .recovery_context
            .as_ref()
            .unwrap()
            .origin_state,
        RecoveryOriginStateV1::Executing
    );
    let row = &response.current().record().participants["mem_a"];
    assert_eq!(row.state, ParticipantState::Planned);
    assert!(row.pending_action.is_none());
    assert_eq!(fixture.backend.head(&fixture.member).unwrap(), head_before);
    assert_eq!(
        fs::read_to_string(fixture.member.join("untracked.txt")).unwrap(),
        "drift\n"
    );
}

#[test]
fn recovery_resume_restores_the_literal_origin_then_reobserves_the_owner() {
    let mut fixture = fixture("merge-v1-forward-recovery-resume", Kind::FastForward);
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Executing,
    });
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(fast_forward_action(&fixture.model));
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(response.current().record().state, OperationState::Completed);
    assert!(response.current().record().recovery_context.is_none());
    assert_eq!(
        response.current().record().participants["mem_a"].state,
        ParticipantState::FastForwarded
    );
}

#[test]
fn recovery_with_an_exact_owner_rejects_drift_in_another_selected_participant() {
    let mut fixture = fixture("merge-v1-forward-recovery-cross-member", Kind::FastForward);
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Executing,
    });
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(fast_forward_action(&fixture.model));

    let member_b = fixture.root.path.join("members/b");
    fixture.backend.create_repo(&member_b).unwrap();
    let before_b = commit_file(&member_b, "README.md", "base\n", "base", &[]).unwrap();
    let mut row_b = fixture.model.participants["mem_a"].clone();
    row_b.path = "members/b".into();
    row_b.before_commit = before_b.clone();
    row_b.source_commit = before_b;
    row_b.resulting_commit = None;
    row_b.state = ParticipantState::Planned;
    row_b.pending_action = None;
    fixture.model.selected_targets.push("mem_b".into());
    fixture.model.participants.insert("mem_b".into(), row_b);
    let mut manifest =
        ManifestArtifact::from_yaml(fixture.model.baseline.manifest_yaml.as_deref().unwrap())
            .unwrap();
    let mut member_b_manifest = manifest.members[0].clone();
    member_b_manifest.id = "mem_b".into();
    member_b_manifest.path = "members/b".into();
    member_b_manifest.source_id = "src_b".into();
    manifest.members.push(member_b_manifest);
    let manifest_yaml = manifest.to_yaml().unwrap();
    fixture.model.baseline.manifest_sha256 =
        format!("{:x}", sha2::Sha256::digest(manifest_yaml.as_bytes()));
    fixture.model.baseline.manifest_yaml = Some(manifest_yaml);
    fs::write(member_b.join("untracked.txt"), "drift\n").unwrap();
    seed_open(&fixture);
    let record_path = fixture
        .root
        .path
        .join(".gwz/merge")
        .join(format!("{}.yaml", fixture.model.merge_id));
    let before_bytes = fs::read(&record_path).unwrap();
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let error = match run(&fixture, &mut runtime) {
        Ok(_) => panic!("recovery must reject drift in a non-owner participant"),
        Err(error) => error,
    };

    assert_eq!(
        error.code,
        ErrorCode::RecoveryEvidenceMismatch,
        "{}",
        error.message
    );
    assert_eq!(error.member_id.as_deref(), Some("mem_b"));
    assert_eq!(fs::read(record_path).unwrap(), before_bytes);
}

#[test]
fn pre_acceptance_finalizing_recovery_is_verified_from_live_inputs() {
    let mut fixture = fixture("merge-v1-forward-finalizing-recovery", Kind::FastForward);
    fixture
        .backend
        .fast_forward(&fixture.member, "main", &fixture.source)
        .unwrap();
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Finalizing,
    });
    let row = fixture.model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some(fixture.source.clone());
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(response.current().record().state, OperationState::Completed);
    assert!(response.current().record().accepted_workspace.is_some());
}

#[test]
fn forward_runtime_rejects_reverse_lifecycle_recovery_origins() {
    let root = TempDir::new("merge-v1-forward-reverse-recovery");
    let backend = Git2Backend::new();
    let context = context();
    for (origin, mut model) in [
        (
            RecoveryOriginStateV1::Preserving,
            super::fixtures::preserving_record(),
        ),
        (RecoveryOriginStateV1::RollingBack, {
            let mut model = test_record();
            let row = model.participants.get_mut("mem_a").unwrap();
            row.state = ParticipantState::FastForwarded;
            row.resulting_commit = Some("d".repeat(40));
            model.pending_rollback = Some(PendingRollbackActionV1::Participant {
                member_id: "mem_a".into(),
                action: ParticipantRollbackKindV1::ResetIntegrated,
                terminal_state: ParticipantState::RolledBack,
            });
            model
        }),
    ] {
        if origin == RecoveryOriginStateV1::Preserving {
            model.pending_preservation = Some(super::fixtures::backup_action());
        }
        model.state = OperationState::RecoveryRequired;
        model.recovery_context = Some(RecoveryContextV1 {
            origin_state: origin,
        });
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        let request = BoundObservationRequest::for_test(
            &current,
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
        )
        .unwrap();
        let mut runtime = ForwardRuntime::new(&backend, &context);

        let error = match runtime.observe(&current, &request) {
            Ok(_) => panic!("forward runtime must not verify {origin:?} recovery"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::MergePhaseUnsupported);
    }
}

#[test]
fn real_conflict_stops_at_awaiting_resolution_and_continue_commits_it() {
    let fixture = fixture("merge-v1-forward-conflict", Kind::Conflict);
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let stopped = run(&fixture, &mut runtime).unwrap();
    let row = &stopped.current().record().participants["mem_a"];
    assert_eq!(
        stopped.current().record().state,
        OperationState::AwaitingResolution
    );
    assert_eq!(row.state, ParticipantState::Conflicted);
    assert_eq!(
        row.expected_merge_head.as_deref(),
        Some(fixture.source.as_str())
    );
    assert!(row.pending_action.is_none());
    assert_eq!(row.conflict_snapshot.len(), 1);
    assert_eq!(row.conflict_snapshot[0].path, "README.md");
    assert_eq!(
        row.conflict_snapshot[0].sha256,
        format!(
            "{:x}",
            sha2::Sha256::digest(fs::read(fixture.member.join("README.md")).unwrap())
        )
    );
    assert!(
        fixture
            .backend
            .merge_state(&fixture.member)
            .unwrap()
            .is_some()
    );

    fs::write(fixture.member.join("README.md"), "resolved\n").unwrap();
    fixture
        .backend
        .stage_paths(&fixture.member, &["README.md"])
        .unwrap();
    let mut resumed = ForwardRuntime::new(&fixture.backend, &context);
    let completed = run(&fixture, &mut resumed).unwrap();
    let row = &completed.current().record().participants["mem_a"];
    assert_eq!(
        completed.current().record().state,
        OperationState::Completed
    );
    assert_eq!(row.state, ParticipantState::Continued);
    assert!(row.resulting_commit.is_some());
    assert!(row.pending_action.is_none());
    assert!(
        fixture
            .backend
            .merge_state(&fixture.member)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unresolved_continue_keeps_the_operation_awaiting_resolution() {
    let fixture = fixture("merge-v1-forward-unresolved", Kind::Conflict);
    seed_open(&fixture);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);
    run(&fixture, &mut runtime).unwrap();

    let error = run(&fixture, &mut runtime).err().unwrap();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    let current = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert_eq!(current.record().state, OperationState::AwaitingResolution);
    assert!(
        current.record().participants["mem_a"]
            .pending_action
            .is_none()
    );
}

#[test]
fn recovery_resume_rejects_live_state_that_is_still_ambiguous() {
    let mut fixture = fixture("merge-v1-forward-recovery-reject", Kind::FastForward);
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Executing,
    });
    fixture
        .model
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(fast_forward_action(&fixture.model));
    seed_open(&fixture);
    fs::write(fixture.member.join("untracked.txt"), "drift\n").unwrap();
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let error = run(&fixture, &mut runtime).err().unwrap();

    assert_eq!(error.code, ErrorCode::RecoveryEvidenceMismatch);
    let current = CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap();
    assert_eq!(current.record().state, OperationState::RecoveryRequired);
}

#[cfg(unix)]
#[test]
fn symlinked_member_directory_is_rejected_before_git_execution() {
    use std::os::unix::fs::symlink;

    let fixture = fixture("merge-v1-forward-member-symlink", Kind::FastForward);
    let outside = fixture.root.path.join("outside-member");
    fs::rename(&fixture.member, &outside).unwrap();
    symlink(&outside, &fixture.member).unwrap();
    seed_open(&fixture);
    let row = &fixture.model.participants["mem_a"];
    let path_error = crate::workspace_ops::merge::status::validated_participant_path(
        &fixture.root.path,
        "mem_a",
        row,
    )
    .err()
    .unwrap();
    assert_eq!(path_error.code, ErrorCode::PathEscape);
    let context = context();
    let mut runtime = ForwardRuntime::new(&fixture.backend, &context);

    let response = run(&fixture, &mut runtime).unwrap();

    assert_eq!(response.current().record().state, OperationState::Halted);
    assert_eq!(
        response.current().record().participants["mem_a"]
            .error
            .as_ref()
            .unwrap()
            .code,
        ErrorCode::PathEscape
    );
    assert_eq!(
        fixture.backend.head(&outside).unwrap().commit,
        Some(fixture.before)
    );
}

#[test]
fn executor_error_with_no_progress_is_durably_halted_once() {
    let fixture = fixture("merge-v1-forward-executor-failure", Kind::FastForward);
    seed_open(&fixture);
    let context = context();
    let mut runtime = FailParticipantRuntime {
        inner: ForwardRuntime::new(&fixture.backend, &context),
        failures: 0,
    };

    let response = run(&fixture, &mut runtime).unwrap();

    let row = &response.current().record().participants["mem_a"];
    assert_eq!(response.current().record().state, OperationState::Halted);
    assert_eq!(row.state, ParticipantState::Failed);
    assert!(row.error.is_some());
    assert!(row.pending_action.is_some());
    assert_eq!(runtime.failures, 1);
    assert_eq!(
        fixture
            .backend
            .head(&fixture.member)
            .unwrap()
            .commit
            .as_deref(),
        Some(fixture.before.as_str())
    );
}

pub(super) fn run<R: ExactObserver + PhysicalExecutor>(
    fixture: &Fixture,
    runtime: &mut R,
) -> ModelResult<super::super::service::V1ServiceResponse> {
    super::super::service::run_test(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Continue,
        runtime,
    )
}

/// Drive an unwrapped production runtime through the production
/// `service::run` seam. M5b design §7 (Q5) requires the no-ff forward and
/// determinism suites to enter the lifecycle here, so A1 can re-point them
/// at production dispatch without rewriting them; only fault-injecting
/// wrappers (which cannot be `V1Runtime`) go through `run_test`.
#[allow(private_bounds)]
pub(super) fn run_production<R: super::super::service::V1Runtime>(
    fixture: &Fixture,
    runtime: &mut R,
    request: V1LifecycleRequest,
) -> ModelResult<super::super::service::V1ServiceResponse> {
    super::super::service::run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        request,
        // DR-1 ship (1) W3: these fixtures run on the CI host's own volume,
        // which is above the bar, and `None` is what an undecided invocation
        // passes — the forward arms activate, exactly as before this step.
        None,
        runtime,
        &mut super::super::events::LifecycleEvents::silent(),
    )
}

/// Where an injected crash lands relative to the participant Git mutation.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Crash {
    None,
    /// After the durable action write, before any Git mutation.
    BeforeExecution,
    /// After ref publication, before the durable outcome write.
    AfterExecution,
}

/// Snapshot the durable participant action at the instant the physical
/// executor is invoked — the frozen action of design §4.1 — and optionally
/// stop on either side of the Git mutation it authorizes.
pub(super) struct CapturingRuntime<'a> {
    pub(super) inner: ForwardRuntime<'a, Git2Backend>,
    pub(super) frozen: Option<PendingMergeAction>,
    pub(super) crash: Crash,
}

impl<'a> CapturingRuntime<'a> {
    pub(super) fn new(inner: ForwardRuntime<'a, Git2Backend>, crash: Crash) -> Self {
        Self {
            inner,
            frozen: None,
            crash,
        }
    }
}

impl ExactObserver for CapturingRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for CapturingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let participant_action = matches!(action, PhysicalActionKind::Participant { .. });
        if let PhysicalActionKind::Participant {
            action: participant,
            ..
        } = action
        {
            self.frozen = Some(participant.as_ref().clone());
            if self.crash == Crash::BeforeExecution {
                panic!("injected crash after the durable action write, before Git mutation");
            }
        }
        let result = self.inner.execute(lease, current, action);
        if participant_action && self.crash == Crash::AfterExecution {
            assert_eq!(result, ExecutionDiagnostic::Success);
            panic!("injected crash after ref publication, before the outcome write");
        }
        result
    }
}

/// Run a no-ff continue that stops immediately after the durable action
/// write, returning the frozen action the store now carries.
pub(super) fn freeze_without_mutation(fixture: &Fixture) -> PendingMergeAction {
    let context = context();
    let mut runtime = CapturingRuntime::new(
        ForwardRuntime::new(&fixture.backend, &context),
        Crash::BeforeExecution,
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(fixture, &mut runtime)
        }))
        .is_err(),
        "the injected pre-mutation crash must unwind"
    );
    runtime.frozen.expect("a participant action was dispatched")
}

/// A no-ff fast-forwardable fixture whose two-parent action is frozen and
/// durable, with the member repository still untouched.
pub(super) fn frozen_no_ff(name: &str) -> (Fixture, PendingMergeAction) {
    let mut fixture = fixture(name, Kind::FastForward);
    fixture.model.mode = MergeExecutionMode::NoFf;
    seed_open(&fixture);
    let frozen = freeze_without_mutation(&fixture);
    (fixture, frozen)
}

/// Execute the frozen action, then stop before the durable outcome write.
pub(super) fn execute_then_crash(fixture: &Fixture) -> String {
    let context = context();
    let mut crashing = CapturingRuntime::new(
        ForwardRuntime::new(&fixture.backend, &context),
        Crash::AfterExecution,
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(fixture, &mut crashing)
        }))
        .is_err()
    );
    head_commit(fixture).unwrap()
}

pub(super) fn record_path(fixture: &Fixture) -> std::path::PathBuf {
    fixture
        .root
        .path
        .join(".gwz/merge")
        .join(format!("{}.yaml", fixture.model.merge_id))
}

pub(super) fn record_text(fixture: &Fixture) -> String {
    fs::read_to_string(record_path(fixture)).unwrap()
}

/// Plant an unknown field inside a durable container reached by `path` under
/// the participant row, so its survival and retirement can be observed
/// (record contract §8, row 382).
pub(super) fn inject_unknown_field(fixture: &Fixture, path: &[&str], key: &str) {
    let file = record_path(fixture);
    let mut document: serde_yaml::Value = serde_yaml::from_str(&record_text(fixture)).unwrap();
    let mut cursor = &mut document["participants"]["mem_a"];
    for step in path {
        cursor = &mut cursor[*step];
    }
    cursor[key] = serde_yaml::Value::Bool(true);
    fs::write(file, serde_yaml::to_string(&document).unwrap()).unwrap();
}

/// The durable action the store holds for `mem_a`, read back from disk.
pub(super) fn stored_action(fixture: &Fixture) -> Option<PendingMergeAction> {
    CheckedV1Store::default()
        .load_open(&fixture.root.path, &fixture.model.merge_id)
        .unwrap()
        .record()
        .participants["mem_a"]
        .pending_action
        .clone()
}

pub(super) fn head_commit(fixture: &Fixture) -> Option<String> {
    fixture.backend.head(&fixture.member).unwrap().commit
}

/// Every commit-object input design §4.1 enumerates, read back from Git.
#[allow(
    dead_code,
    reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
)]
pub(super) struct CommitFacts {
    pub(super) parents: Vec<String>,
    pub(super) tree: String,
    pub(super) message: Vec<u8>,
    pub(super) author: (String, String, i64, i32),
    pub(super) committer: (String, String, i64, i32),
}

pub(super) fn commit_facts(member: &std::path::Path, commit: &str) -> CommitFacts {
    let repository = git2::Repository::open(member).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(commit).unwrap())
        .unwrap();
    let identity = |signature: &git2::Signature<'_>| {
        (
            signature.name().unwrap().to_owned(),
            signature.email().unwrap().to_owned(),
            signature.when().seconds(),
            signature.when().offset_minutes(),
        )
    };
    CommitFacts {
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        tree: commit.tree_id().to_string(),
        message: commit.message_bytes().to_vec(),
        author: identity(&commit.author()),
        committer: identity(&commit.committer()),
    }
}

pub(super) fn commit_tree(member: &std::path::Path, commit: &str) -> String {
    commit_facts(member, commit).tree
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Kind {
    FastForward,
    Conflict,
    /// Divergent histories that merge cleanly: the frozen tree is the merge
    /// index tree, never the source tree (M5b design §4.1).
    CleanDivergent,
}

pub(super) struct Fixture {
    pub(super) root: TempDir,
    pub(super) backend: Git2Backend,
    pub(super) member: std::path::PathBuf,
    pub(super) model: MergeOperationRecordV1,
    pub(super) before: String,
    pub(super) source: String,
}

pub(super) fn fixture(name: &str, kind: Kind) -> Fixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    // Pin the boundary mode at creation: the runner images' git template tree
    // ships an executable `info/exclude` that repository creation copies, and
    // every root gate reading it then refuses (see the helper's doctrine).
    crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::pin_fixture_boundary_mode(
        &root.path,
    );
    fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let mut model = test_record();
    let manifest = model.baseline.manifest_yaml.clone().unwrap();
    let lock = model.baseline.lock_yaml.clone().unwrap();
    let manifest_commit = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &manifest,
        "workspace manifest",
        &[],
    )
    .unwrap();
    let root_commit = commit_file(
        &root.path,
        LOCK_PATH,
        &lock,
        "workspace lock",
        &[git2::Oid::from_str(&manifest_commit).unwrap()],
    )
    .unwrap();
    model.baseline.root_head = Some(root_commit);
    model.baseline.root_branch = backend.head(&root.path).unwrap().branch;

    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let common = commit_file(&member, "README.md", "base\n", "base", &[]).unwrap();
    backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(&member, "feature/source").unwrap();
    let source = commit_file(
        &member,
        if matches!(kind, Kind::Conflict) {
            "README.md"
        } else {
            "source.txt"
        },
        "source\n",
        "source",
        &[git2::Oid::from_str(&common).unwrap()],
    )
    .unwrap();
    backend.switch_branch(&member, "main").unwrap();
    let before = match kind {
        Kind::FastForward => common,
        Kind::Conflict | Kind::CleanDivergent => commit_file(
            &member,
            if kind == Kind::Conflict {
                "README.md"
            } else {
                "local.txt"
            },
            "local\n",
            "local",
            &[git2::Oid::from_str(&common).unwrap()],
        )
        .unwrap(),
    };
    let row = model.participants.get_mut("mem_a").unwrap();
    row.before_commit = before.clone();
    row.source_commit = source.clone();
    row.resulting_commit = None;
    row.state = ParticipantState::Planned;
    row.error = None;
    row.pending_action = None;
    Fixture {
        root,
        backend,
        member,
        model,
        before,
        source,
    }
}

pub(super) fn seed_open(fixture: &Fixture) {
    let directory = fixture.root.path.join(".gwz/merge");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{}.yaml", fixture.model.merge_id)),
        serde_yaml::to_string(&fixture.model).unwrap(),
    )
    .unwrap();
}

fn fast_forward_action(model: &MergeOperationRecordV1) -> PendingMergeAction {
    let row = &model.participants["mem_a"];
    crate::workspace_ops::merge::integration::PreparedIntegration {
        intent: crate::workspace_ops::merge::integration::IntegrationIntent::from_record(row),
        action: crate::workspace_ops::merge::integration::PreparedIntegrationAction::FastForward,
    }
    .to_pending()
}

pub(super) fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_1".into(),
        request_id: "req_1".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

pub(super) struct CrashAfterParticipant<'a> {
    pub(super) inner: ForwardRuntime<'a, Git2Backend>,
    pub(super) executions: usize,
}

impl ExactObserver for CrashAfterParticipant<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for CrashAfterParticipant<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let result = self.inner.execute(lease, current, action);
        if matches!(action, PhysicalActionKind::Participant { .. })
            && result == ExecutionDiagnostic::Success
        {
            self.executions += 1;
            panic!("injected crash after participant Git mutation");
        }
        result
    }
}

pub(super) struct CountingRuntime<'a> {
    pub(super) inner: ForwardRuntime<'a, Git2Backend>,
    pub(super) executions: usize,
}

impl ExactObserver for CountingRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for CountingRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.executions += 1;
        self.inner.execute(lease, current, action)
    }
}

struct FailParticipantRuntime<'a> {
    inner: ForwardRuntime<'a, Git2Backend>,
    failures: usize,
}

impl ExactObserver for FailParticipantRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for FailParticipantRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if matches!(action, PhysicalActionKind::Participant { .. }) {
            self.failures += 1;
            ExecutionDiagnostic::Failed {
                code: ErrorCode::GitCommandFailed,
                message: "injected participant executor failure".into(),
                detail: None,
            }
        } else {
            self.inner.execute(lease, current, action)
        }
    }
}
