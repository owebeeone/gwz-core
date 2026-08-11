use super::*;
use crate::model::ModelResult;
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
fn every_root_phase_durable_successor_restarts_without_repeating_the_phase() {
    for owner in [RootOwner::Publication, RootOwner::Selected] {
        let targets = emitted_phase_targets(owner);
        assert_eq!(targets, expected_targets(), "{owner:?} phase graph drifted");
        for (index, target) in targets.into_iter().enumerate() {
            let fixture = root_fixture(owner, &format!("v1-root-successor-{owner:?}-{index}"));
            fixture.base.seed_open();
            let context = fixture.base.context();
            let mut interrupt = SuccessorInterruptRuntime {
                inner: ReverseRuntime::new(&fixture.base.backend, &context),
                target,
                seen: false,
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
            assert!(interrupt.seen, "{owner:?} {target:?} was not observed");
            assert!(first.is_err(), "{owner:?} {target:?} did not interrupt");

            let store = CheckedV1Store::default();
            let interrupted = store
                .load_open(&fixture.base.root.path, &fixture.base.model.merge_id)
                .unwrap();
            assert_ne!(
                PhaseTarget::from_record(interrupted.record()),
                Some(target),
                "{owner:?} {target:?} successor was not durable",
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
            let response = run(
                &store,
                &fixture.base.root.path,
                &fixture.base.model.merge_id,
                if index % 2 == 0 {
                    V1LifecycleRequest::Abort
                } else {
                    V1LifecycleRequest::Preserve
                },
                &mut resume,
            )
            .unwrap();
            match owner {
                RootOwner::Publication => assert_eq!(
                    response.disposition(),
                    V1ResponseDisposition::Terminal(OperationState::Aborted),
                    "{owner:?} {target:?}",
                ),
                RootOwner::Selected => assert!(
                    matches!(
                        response.disposition(),
                        V1ResponseDisposition::Terminal(OperationState::Aborted)
                            | V1ResponseDisposition::Stopped(OperationState::RecoveryRequired)
                    ),
                    "{owner:?} {target:?}",
                ),
            }
            assert_eq!(
                resume.executions, 0,
                "{owner:?} {target:?} repeated after its durable successor",
            );
            assert!(response.current().record().pending_preservation.is_none());
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhaseTarget {
    Backup,
    Stash(S),
    Reset(R),
}

impl PhaseTarget {
    fn from_record(record: &MergeOperationRecordV1) -> Option<Self> {
        match record.pending_preservation.as_ref()? {
            PendingPreservationActionV1::BackupRef { .. } => Some(Self::Backup),
            PendingPreservationActionV1::Stash { phase, .. } => Some(Self::Stash(*phase)),
            PendingPreservationActionV1::ResetAttachedRef { phase, .. } => {
                Some(Self::Reset(*phase))
            }
        }
    }

    fn matches_action(self, action: &PhysicalActionKind) -> bool {
        match action {
            PhysicalActionKind::Preservation(action) => match action {
                PendingPreservationActionV1::BackupRef { .. } => self == Self::Backup,
                PendingPreservationActionV1::Stash { phase, .. } => self == Self::Stash(*phase),
                PendingPreservationActionV1::ResetAttachedRef { phase, .. } => {
                    self == Self::Reset(*phase)
                }
            },
            PhysicalActionKind::Rollback(_)
            | PhysicalActionKind::Participant { .. }
            | PhysicalActionKind::Publication(_)
            | PhysicalActionKind::Archive => false,
        }
    }
}

fn expected_targets() -> Vec<PhaseTarget> {
    vec![
        PhaseTarget::Backup,
        PhaseTarget::Stash(S::NormalizeParent),
        PhaseTarget::Stash(S::NormalizeMarker),
        PhaseTarget::Stash(S::NormalizeLock),
        PhaseTarget::Stash(S::NormalizeIndex),
        PhaseTarget::Stash(S::CreateStash),
        PhaseTarget::Stash(S::RestoreIndex),
        PhaseTarget::Stash(S::RestoreLock),
        PhaseTarget::Stash(S::RestoreParent),
        PhaseTarget::Stash(S::RestoreMarker),
        PhaseTarget::Stash(S::WriteBundle),
        PhaseTarget::Stash(S::Complete),
        PhaseTarget::Reset(R::PrepareParent),
        PhaseTarget::Reset(R::PrepareMarker),
        PhaseTarget::Reset(R::PrepareLock),
        PhaseTarget::Reset(R::PrepareIndex),
        PhaseTarget::Reset(R::ResetRef),
        PhaseTarget::Reset(R::RestoreIndex),
        PhaseTarget::Reset(R::RestoreLock),
        PhaseTarget::Reset(R::RestoreParent),
        PhaseTarget::Reset(R::RestoreMarker),
        PhaseTarget::Reset(R::Complete),
    ]
}

fn emitted_phase_targets(owner: RootOwner) -> Vec<PhaseTarget> {
    let fixture = root_fixture(owner, &format!("v1-root-successor-trace-{owner:?}"));
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
    runtime.targets
}

struct TraceRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    targets: Vec<PhaseTarget>,
}

impl ExactObserver for TraceRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        if let Some(target) = PhaseTarget::from_record(current.record())
            && self.targets.last() != Some(&target)
        {
            self.targets.push(target);
        }
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
        self.inner.execute(lease, current, action)
    }
}

struct SuccessorInterruptRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: PhaseTarget,
    seen: bool,
}

impl ExactObserver for SuccessorInterruptRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let observed = PhaseTarget::from_record(current.record());
        if self.seen && observed != Some(self.target) {
            panic!("injected crash after durable phase successor");
        }
        if observed == Some(self.target) {
            self.seen = true;
        }
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for SuccessorInterruptRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.inner.execute(lease, current, action)
    }
}

struct CountingRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: PhaseTarget,
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
        if self.target.matches_action(action) {
            self.executions += 1;
        }
        self.inner.execute(lease, current, action)
    }
}
