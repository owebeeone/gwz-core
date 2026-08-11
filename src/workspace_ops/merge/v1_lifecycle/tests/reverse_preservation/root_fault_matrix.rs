use super::*;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationRefResetPhaseV1 as R, PreservationStashPhaseV1 as S,
    RecoveryContextV1, RecoveryOriginStateV1,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{ExactObserver, PhysicalExecutor, run};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn every_root_physical_and_successor_boundary_recovers_without_repeating_mutation() {
    for owner in [RootOwner::Publication, RootOwner::Selected] {
        let cases = physical_cases(owner);
        assert_eq!(
            cases.iter().map(|(_, target)| *target).collect::<Vec<_>>(),
            expected_targets(),
            "{owner:?} physical action set drifted",
        );
        for (target_index, (handoff, target)) in cases.into_iter().enumerate() {
            for (boundary_index, boundary) in [
                Boundary::BeforePhysical,
                Boundary::AfterPhysical,
                Boundary::AfterDurableSuccessor,
            ]
            .into_iter()
            .enumerate()
            {
                let name = format!("v1-root-matrix-{owner:?}-{target_index}-{boundary_index}",);
                let mut fixture = root_fixture(owner, &name);
                install_root_handoff(&mut fixture, handoff.prefix, handoff.index);
                fixture.base.seed_open();
                let context = fixture.base.context();
                let mut interrupt = InterruptRuntime {
                    inner: ReverseRuntime::new(&fixture.base.backend, &context),
                    target,
                    boundary,
                    physical_complete: false,
                    interrupted: false,
                };
                let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        &CheckedV1Store::default(),
                        &fixture.base.root.path,
                        &fixture.base.model.merge_id,
                        V1LifecycleRequest::Preserve,
                        &mut interrupt,
                    )
                }));
                assert!(interrupt.interrupted, "{owner:?} {target:?} was not issued");
                match boundary {
                    Boundary::BeforePhysical => match &first {
                        Ok(Err(_)) => {}
                        Ok(Ok(response)) => assert_eq!(
                            response.disposition(),
                            V1ResponseDisposition::Stopped(OperationState::RecoveryRequired),
                            "{target:?} {boundary:?} did not retain a recoverable journal",
                        ),
                        Err(_) => panic!("{target:?} {boundary:?} unexpectedly panicked"),
                    },
                    Boundary::AfterPhysical | Boundary::AfterDurableSuccessor => {
                        assert!(first.is_err(), "{target:?} {boundary:?} did not interrupt")
                    }
                }
                let interrupted = CheckedV1Store::default()
                    .load_open(&fixture.base.root.path, &fixture.base.model.merge_id)
                    .unwrap();
                assert!(
                    matches!(
                        interrupted.record().state,
                        OperationState::Preserving | OperationState::RecoveryRequired
                    ),
                    "{target:?} {boundary:?} left a non-recoverable state"
                );
                assert!(
                    interrupted
                        .record()
                        .pending_preservation
                        .as_ref()
                        .is_some_and(|action| target
                            .matches(&PhysicalActionKind::Preservation(action.clone())))
                        || boundary == Boundary::AfterDurableSuccessor
                );
                if interrupted.record().pending_preservation.is_some() {
                    seed_recovery(&fixture.base.root.path, interrupted.record());
                }
                let context = fixture.base.context();
                let mut resume = CountingRuntime {
                    inner: ReverseRuntime::new(&fixture.base.backend, &context),
                    target,
                    executions: 0,
                };
                let store = CheckedV1Store::default();
                let mut response = run(
                    &store,
                    &fixture.base.root.path,
                    &fixture.base.model.merge_id,
                    if boundary_index % 2 == 0 {
                        V1LifecycleRequest::Abort
                    } else {
                        V1LifecycleRequest::Preserve
                    },
                    &mut resume,
                )
                .unwrap();
                for retry in 0..8 {
                    if response.disposition()
                        != V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
                    {
                        break;
                    }
                    response = run(
                        &store,
                        &fixture.base.root.path,
                        &fixture.base.model.merge_id,
                        if retry % 2 == 0 {
                            V1LifecycleRequest::Preserve
                        } else {
                            V1LifecycleRequest::Abort
                        },
                        &mut resume,
                    )
                    .unwrap();
                }
                assert_eq!(
                    response.disposition(),
                    V1ResponseDisposition::Terminal(OperationState::Aborted),
                    "{owner:?} {target:?} {boundary:?}",
                );
                assert!(response.current().record().pending_preservation.is_none());
                let expected = match boundary {
                    Boundary::BeforePhysical => 1,
                    Boundary::AfterPhysical if target.has_parent_durability() => 1,
                    Boundary::AfterPhysical | Boundary::AfterDurableSuccessor => 0,
                };
                assert_eq!(resume.executions, expected, "{target:?} {boundary:?}");
                assert_eq!(
                fixture
                    .base
                    .backend
                    .preservation_stashes(
                        &fixture.base.root.path,
                        &fixture.base.model.merge_id,
                    )
                    .unwrap()
                    .len(),
                1,
                "{target:?} {boundary:?}"
            );
            }
        }
    }
}

fn seed_recovery(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let mut recovery = model.clone();
    recovery.state = OperationState::RecoveryRequired;
    recovery.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Preserving,
    });
    std::fs::write(
        root.join(format!(".gwz/merge/{}.yaml", recovery.merge_id)),
        serde_yaml::to_string(&recovery).unwrap(),
    )
    .unwrap();
}

#[derive(Clone, Copy, Debug)]
enum RootOwner {
    Publication,
    Selected,
}

fn root_fixture(owner: RootOwner, name: &str) -> RootPreservationFixture {
    match owner {
        RootOwner::Publication => dirty_root_handoff_fixture(name),
        RootOwner::Selected => dirty_selected_root_handoff_fixture(name),
    }
}

#[derive(Clone, Copy, Debug)]
struct Handoff {
    prefix: crate::workspace_ops::merge::model::v1::PublicationPrefixV1,
    index: crate::workspace_ops::merge::model::v1::PublicationIndexFormV1,
}

fn handoffs() -> [Handoff; 5] {
    use crate::workspace_ops::merge::model::v1::{
        PublicationIndexFormV1 as I, PublicationPrefixV1 as P,
    };
    [
        Handoff {
            prefix: P::Baseline,
            index: I::Pre,
        },
        Handoff {
            prefix: P::Marker,
            index: I::Pre,
        },
        Handoff {
            prefix: P::Lock,
            index: I::Pre,
        },
        Handoff {
            prefix: P::Boundary,
            index: I::Pre,
        },
        Handoff {
            prefix: P::Boundary,
            index: I::Staged,
        },
    ]
}

fn physical_cases(owner: RootOwner) -> Vec<(Handoff, Target)> {
    let mut cases = Vec::new();
    for (form_index, handoff) in handoffs().into_iter().enumerate() {
        for target in emitted_root_targets(owner, handoff, form_index) {
            if !cases.iter().any(|(_, existing)| *existing == target) {
                cases.push((handoff, target));
            }
        }
    }
    cases
}

fn emitted_root_targets(owner: RootOwner, handoff: Handoff, form_index: usize) -> Vec<Target> {
    let mut fixture = root_fixture(
        owner,
        &format!("v1-root-matrix-trace-{owner:?}-{form_index}"),
    );
    install_root_handoff(&mut fixture, handoff.prefix, handoff.index);
    fixture.base.seed_open();
    let context = fixture.base.context();
    let mut runtime = TraceRuntime {
        inner: ReverseRuntime::new(&fixture.base.backend, &context),
        targets: Vec::new(),
    };
    let response = run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();
    assert!(matches!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted)
            | V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
    ));
    assert!(response.current().record().pending_preservation.is_none());
    runtime.targets
}

fn expected_targets() -> Vec<Target> {
    vec![
        Target::Backup,
        Target::Stash(S::NormalizeParent),
        Target::Stash(S::NormalizeMarker),
        Target::Stash(S::NormalizeLock),
        Target::Stash(S::NormalizeIndex),
        Target::Stash(S::CreateStash),
        Target::Stash(S::RestoreIndex),
        Target::Stash(S::RestoreLock),
        Target::Stash(S::RestoreMarker),
        Target::Stash(S::WriteBundle),
        Target::Reset(R::PrepareParent),
        Target::Reset(R::PrepareMarker),
        Target::Reset(R::PrepareLock),
        Target::Reset(R::PrepareIndex),
        Target::Reset(R::ResetRef),
        Target::Reset(R::RestoreIndex),
        Target::Reset(R::RestoreLock),
        Target::Reset(R::RestoreMarker),
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Backup,
    Stash(S),
    Reset(R),
}

impl Target {
    fn from_action(action: &PhysicalActionKind) -> Option<Self> {
        match action {
            PhysicalActionKind::Preservation(PendingPreservationActionV1::BackupRef { .. }) => {
                Some(Self::Backup)
            }
            PhysicalActionKind::Preservation(PendingPreservationActionV1::Stash {
                phase, ..
            }) if *phase != S::Complete => Some(Self::Stash(*phase)),
            PhysicalActionKind::Preservation(PendingPreservationActionV1::ResetAttachedRef {
                phase,
                ..
            }) if *phase != R::Complete => Some(Self::Reset(*phase)),
            _ => None,
        }
    }

    fn matches(self, action: &PhysicalActionKind) -> bool {
        match (self, action) {
            (
                Target::Backup,
                PhysicalActionKind::Preservation(PendingPreservationActionV1::BackupRef { .. }),
            ) => true,
            (
                Target::Stash(expected),
                PhysicalActionKind::Preservation(PendingPreservationActionV1::Stash {
                    phase, ..
                }),
            ) => expected == *phase,
            (
                Target::Reset(expected),
                PhysicalActionKind::Preservation(PendingPreservationActionV1::ResetAttachedRef {
                    phase,
                    ..
                }),
            ) => expected == *phase,
            _ => false,
        }
    }

    fn has_parent_durability(self) -> bool {
        matches!(
            self,
            Target::Stash(S::NormalizeParent) | Target::Reset(R::PrepareParent)
        )
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    BeforePhysical,
    AfterPhysical,
    AfterDurableSuccessor,
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
                .pending_preservation
                .as_ref()
                .is_some_and(|action| {
                    self.target
                        .matches(&PhysicalActionKind::Preservation(action.clone()))
                })
        {
            panic!("injected crash after durable successor");
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
        if self.target.matches(action) && !self.interrupted {
            self.interrupted = true;
            if self.boundary == Boundary::BeforePhysical {
                return ExecutionDiagnostic::Failed {
                    code: ErrorCode::GitCommandFailed,
                    message: "injected pre-physical failure".into(),
                    detail: None,
                };
            }
            let result = self.inner.execute(lease, current, action);
            assert_eq!(result, ExecutionDiagnostic::Success);
            self.physical_complete = true;
            if self.boundary == Boundary::AfterPhysical {
                panic!("injected crash after physical mutation");
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
        if self.target.matches(action) {
            self.executions += 1;
        }
        self.inner.execute(lease, current, action)
    }
}
