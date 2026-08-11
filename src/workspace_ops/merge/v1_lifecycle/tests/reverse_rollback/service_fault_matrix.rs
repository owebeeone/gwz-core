use super::*;
use crate::artifact::LOCK_PATH;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{ActionKind, OperationContext};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1 as E, ParticipantRollbackKindV1 as P, PendingRollbackActionV1,
    RecoveryContextV1, RecoveryOriginStateV1, RootMetadataRollbackStepV1 as R,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{ExactObserver, PhysicalExecutor, run};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use sha2::{Digest, Sha256};

#[test]
fn every_emitted_rollback_physical_and_successor_boundary_recovers_exactly_once() {
    for lane in [Lane::Participant, Lane::Evidence, Lane::SelectedRoot] {
        let targets = emitted_targets(lane);
        assert_eq!(
            targets,
            expected_targets(lane),
            "{lane:?} action set drifted"
        );
        for (target_index, target) in targets.into_iter().enumerate() {
            for (boundary_index, boundary) in [
                Boundary::BeforePhysical,
                Boundary::AfterPhysical,
                Boundary::AfterDurableSuccessor,
            ]
            .into_iter()
            .enumerate()
            {
                let fixture = fixture(
                    lane,
                    &format!("v1-rollback-matrix-{lane:?}-{target_index}-{boundary_index}"),
                );
                seed_open(&fixture.root.path, &fixture.model);
                let operation_context = context(&fixture.model);
                let mut interrupt = InterruptRuntime {
                    inner: ReverseRuntime::new(&fixture.backend, &operation_context),
                    target,
                    boundary,
                    physical_complete: false,
                    interrupted: false,
                };
                let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        &CheckedV1Store::default(),
                        &fixture.root.path,
                        &fixture.model.merge_id,
                        V1LifecycleRequest::Abort,
                        &mut interrupt,
                    )
                }));
                assert!(interrupt.interrupted, "{lane:?} {target:?} was not issued");
                match boundary {
                    Boundary::BeforePhysical => assert!(
                        matches!(first, Ok(Err(_))),
                        "{lane:?} {target:?} did not stop before observation"
                    ),
                    Boundary::AfterPhysical | Boundary::AfterDurableSuccessor => assert!(
                        first.is_err(),
                        "{lane:?} {target:?} {boundary:?} did not interrupt"
                    ),
                }

                let store = CheckedV1Store::default();
                let interrupted = store
                    .load_open(&fixture.root.path, &fixture.model.merge_id)
                    .unwrap();
                assert_eq!(interrupted.record().state, OperationState::RollingBack);
                assert!(
                    interrupted
                        .record()
                        .pending_rollback
                        .as_ref()
                        .is_some_and(|action| target.matches(action))
                        || boundary == Boundary::AfterDurableSuccessor
                );

                if interrupted.record().pending_rollback.is_some() {
                    seed_recovery(&fixture.root.path, interrupted.record());
                }
                let operation_context = context(&fixture.model);
                let mut resume = CountingRuntime {
                    inner: ReverseRuntime::new(&fixture.backend, &operation_context),
                    target,
                    executions: 0,
                };
                let response = run(
                    &store,
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    V1LifecycleRequest::Abort,
                    &mut resume,
                )
                .unwrap();
                assert_eq!(
                    response.disposition(),
                    V1ResponseDisposition::Terminal(OperationState::Aborted),
                    "{lane:?} {target:?} {boundary:?}"
                );
                assert_eq!(
                    resume.executions,
                    usize::from(boundary == Boundary::BeforePhysical),
                    "{lane:?} {target:?} {boundary:?}"
                );
                assert!(response.current().record().pending_rollback.is_none());
                assert!(response.current().record().recovery_context.is_none());
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Lane {
    Participant,
    Evidence,
    SelectedRoot,
}

fn expected_targets(lane: Lane) -> Vec<Target> {
    match lane {
        Lane::Participant => vec![Target::Participant(P::ResetIntegrated)],
        Lane::Evidence => vec![
            Target::Evidence(E::EvidenceCommit),
            Target::Evidence(E::Boundary),
            Target::Evidence(E::Lock),
            Target::Evidence(E::Marker),
            Target::Evidence(E::Index),
        ],
        Lane::SelectedRoot => vec![Target::Root(R::Manifest), Target::Root(R::Lock)],
    }
}

struct MatrixFixture {
    root: TempDir,
    backend: Git2Backend,
    model: MergeOperationRecordV1,
}

fn fixture(lane: Lane, name: &str) -> MatrixFixture {
    match lane {
        Lane::Participant => {
            let value = integrated_fixture(name);
            MatrixFixture {
                root: value.root,
                backend: value.backend,
                model: value.model,
            }
        }
        Lane::Evidence => {
            let mut value = staged_evidence_fixture(name, true, true);
            let row = value.model.participants.get_mut("mem_a").unwrap();
            row.state = if row.resulting_commit.as_deref() == Some(row.before_commit.as_str()) {
                ParticipantState::Aborted
            } else {
                ParticipantState::RolledBack
            };
            MatrixFixture {
                root: value.root,
                backend: value.backend,
                model: value.model,
            }
        }
        Lane::SelectedRoot => selected_root_fixture(name),
    }
}

fn selected_root_fixture(name: &str) -> MatrixFixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    std::fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    let manifest = model.baseline.manifest_yaml.clone().unwrap();
    let lock = model.baseline.lock_yaml.clone().unwrap();
    let manifest_commit = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &manifest,
        "baseline manifest",
        &[],
    )
    .unwrap();
    let before = commit_file(
        &root.path,
        LOCK_PATH,
        &lock,
        "baseline lock",
        &[manifest_commit.parse().unwrap()],
    )
    .unwrap();
    let result_manifest = format!("{manifest}# selected-root result\n");
    let result_lock = format!("{lock}# selected-root result\n");
    let result_manifest_commit = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &result_manifest,
        "result manifest",
        &[before.parse().unwrap()],
    )
    .unwrap();
    let result = commit_file(
        &root.path,
        LOCK_PATH,
        &result_lock,
        "result lock",
        &[result_manifest_commit.parse().unwrap()],
    )
    .unwrap();

    model.state = OperationState::RollingBack;
    model.baseline.root_head = Some(before.clone());
    model.baseline.root_branch = Some("main".into());
    model.baseline.manifest_commit_sha256 = Some(digest(&manifest));
    model.baseline.lock_commit_sha256 = Some(digest(&lock));
    model.selected_targets = vec!["@root".into()];
    git2::Repository::open(&root.path)
        .unwrap()
        .find_reference("refs/heads/main")
        .unwrap()
        .set_target(before.parse().unwrap(), "seed post-participant rollback")
        .unwrap();
    let mut row = model.participants.remove("mem_a").unwrap();
    row.path = ".".into();
    row.target_kind = MergeTargetKind::Root;
    row.target_branch = "main".into();
    row.before_commit = before;
    row.source_commit = result.clone();
    row.state = ParticipantState::RolledBack;
    row.resulting_commit = Some(result);
    model.participants.clear();
    model.participants.insert("@root".into(), row);
    MatrixFixture {
        root,
        backend,
        model,
    }
}

fn digest(bytes: &str) -> String {
    format!("{:x}", Sha256::digest(bytes.as_bytes()))
}

fn seed_open(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let merge_root = root.join(".gwz/merge");
    std::fs::create_dir_all(&merge_root).unwrap();
    std::fs::write(
        merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap(),
    )
    .unwrap();
}

fn seed_recovery(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let mut recovery = model.clone();
    recovery.state = OperationState::RecoveryRequired;
    recovery.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::RollingBack,
    });
    seed_open(root, &recovery);
}

fn context(model: &MergeOperationRecordV1) -> OperationContext {
    OperationContext {
        operation_id: model.operation_id.clone(),
        request_id: format!("req_{}", model.merge_id),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Participant(P),
    Evidence(E),
    Root(R),
}

impl Target {
    fn from_action(action: &PhysicalActionKind) -> Option<Self> {
        match action {
            PhysicalActionKind::Rollback(PendingRollbackActionV1::Participant {
                action, ..
            }) => Some(Self::Participant(*action)),
            PhysicalActionKind::Rollback(PendingRollbackActionV1::PublicationEvidence {
                next_step,
            }) if *next_step != E::Complete => Some(Self::Evidence(*next_step)),
            PhysicalActionKind::Rollback(PendingRollbackActionV1::SelectedRootMetadata {
                next_step,
            }) if *next_step != R::Complete => Some(Self::Root(*next_step)),
            _ => None,
        }
    }

    fn matches(self, action: &PendingRollbackActionV1) -> bool {
        Self::from_action(&PhysicalActionKind::Rollback(action.clone())) == Some(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    BeforePhysical,
    AfterPhysical,
    AfterDurableSuccessor,
}

fn emitted_targets(lane: Lane) -> Vec<Target> {
    let fixture = fixture(lane, &format!("v1-rollback-matrix-trace-{lane:?}"));
    seed_open(&fixture.root.path, &fixture.model);
    let context = context(&fixture.model);
    let mut runtime = TraceRuntime {
        inner: ReverseRuntime::new(&fixture.backend, &context),
        targets: Vec::new(),
    };
    let response = run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Abort,
        &mut runtime,
    )
    .unwrap();
    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted)
    );
    runtime.targets
}

struct TraceRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    targets: Vec<Target>,
}

impl ExactObserver for TraceRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for TraceRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if let Some(target) = Target::from_action(action)
            && !self.targets.contains(&target)
        {
            self.targets.push(target);
        }
        self.inner.execute(lease, current, action)
    }
}

struct InterruptRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: Target,
    boundary: Boundary,
    physical_complete: bool,
    interrupted: bool,
}

impl ExactObserver for InterruptRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if self.boundary == Boundary::BeforePhysical && self.interrupted {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "injected stop after a pre-physical failure",
            ));
        }
        if self.boundary == Boundary::AfterDurableSuccessor
            && self.physical_complete
            && !current
                .record()
                .pending_rollback
                .as_ref()
                .is_some_and(|action| self.target.matches(action))
        {
            panic!("injected crash after durable rollback successor");
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for InterruptRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if Target::from_action(action) == Some(self.target) && !self.interrupted {
            self.interrupted = true;
            if self.boundary == Boundary::BeforePhysical {
                return ExecutionDiagnostic::Failed {
                    code: ErrorCode::GitCommandFailed,
                    message: "injected pre-physical rollback failure".into(),
                    detail: None,
                };
            }
            let result = self.inner.execute(lease, current, action);
            assert_eq!(result, ExecutionDiagnostic::Success);
            self.physical_complete = true;
            if self.boundary == Boundary::AfterPhysical {
                panic!("injected crash after rollback physical mutation");
            }
            return result;
        }
        self.inner.execute(lease, current, action)
    }
}

struct CountingRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: Target,
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
        if Target::from_action(action) == Some(self.target) {
            self.executions += 1;
        }
        self.inner.execute(lease, current, action)
    }
}
