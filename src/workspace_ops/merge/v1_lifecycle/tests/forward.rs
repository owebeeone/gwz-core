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
use crate::workspace_ops::merge::{OperationState, ParticipantState, PendingMergeAction};
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

fn run<R: ExactObserver + PhysicalExecutor>(
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

#[derive(Clone, Copy)]
enum Kind {
    FastForward,
    Conflict,
}

struct Fixture {
    root: TempDir,
    backend: Git2Backend,
    member: std::path::PathBuf,
    model: MergeOperationRecordV1,
    before: String,
    source: String,
}

fn fixture(name: &str, kind: Kind) -> Fixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
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
    let before = if matches!(kind, Kind::Conflict) {
        commit_file(
            &member,
            "README.md",
            "local\n",
            "local",
            &[git2::Oid::from_str(&common).unwrap()],
        )
        .unwrap()
    } else {
        common
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

fn seed_open(fixture: &Fixture) {
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

fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_1".into(),
        request_id: "req_1".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

struct CrashAfterParticipant<'a> {
    inner: ForwardRuntime<'a, Git2Backend>,
    executions: usize,
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

struct CountingRuntime<'a> {
    inner: ForwardRuntime<'a, Git2Backend>,
    executions: usize,
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
