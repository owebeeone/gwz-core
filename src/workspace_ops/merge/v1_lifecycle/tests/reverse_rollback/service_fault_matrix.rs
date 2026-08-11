use super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1 as E, ParticipantRollbackKindV1 as P, PendingRollbackActionV1,
    RootMetadataRollbackStepV1 as R,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{ExactObserver, PhysicalExecutor, run};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

mod fixtures;
pub(super) use fixtures::{Lane, MatrixFixture, context, fixture, seed_open, seed_recovery};

#[test]
fn every_emitted_rollback_physical_and_successor_boundary_recovers_exactly_once() {
    for lane in [
        Lane::AbortConflict,
        Lane::ResetIntegrated,
        Lane::Evidence,
        Lane::SelectedRoot,
    ] {
        let targets = emitted_targets(lane);
        assert_eq!(
            targets,
            expected_targets(lane),
            "{lane:?} action set drifted"
        );
        for (target_index, target) in targets.into_iter().enumerate() {
            for (request_index, request) in
                [V1LifecycleRequest::Abort, V1LifecycleRequest::Preserve]
                    .into_iter()
                    .enumerate()
            {
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
                        &format!(
                            "v1-rollback-matrix-{lane:?}-{target_index}-{request_index}-{boundary_index}"
                        ),
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
                            request,
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
                        if request == V1LifecycleRequest::Abort {
                            V1LifecycleRequest::Preserve
                        } else {
                            V1LifecycleRequest::Abort
                        },
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
}

pub(super) fn expected_targets(lane: Lane) -> Vec<Target> {
    match lane {
        Lane::AbortConflict => vec![Target::Participant(P::AbortConflict)],
        Lane::ResetIntegrated => vec![Target::Participant(P::ResetIntegrated)],
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Target {
    Participant(P),
    Evidence(E),
    Root(R),
}

impl Target {
    pub(super) fn from_action(action: &PhysicalActionKind) -> Option<Self> {
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

    pub(super) fn matches(self, action: &PendingRollbackActionV1) -> bool {
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
