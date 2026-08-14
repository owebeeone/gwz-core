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
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn every_root_phase_durable_successor_restarts_without_repeating_the_phase() {
    for owner in [RootOwner::Publication, RootOwner::Selected] {
        let targets = expected_targets();
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
            let mut response = run(
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
                        V1LifecycleRequest::Abort
                    } else {
                        V1LifecycleRequest::Preserve
                    },
                    &mut resume,
                )
                .unwrap();
            }
            assert_eq!(
                response.disposition(),
                V1ResponseDisposition::Terminal(OperationState::Aborted),
                "{owner:?} {target:?}",
            );
            assert_eq!(
                resume.executions, 0,
                "{owner:?} {target:?} repeated after its durable successor",
            );
            assert!(response.current().record().pending_preservation.is_none());
        }
    }
}

#[test]
fn every_legal_root_handoff_form_exhausts_the_complete_phase_graph() {
    use crate::workspace_ops::merge::model::v1::{
        PublicationIndexFormV1 as I, PublicationPrefixV1 as P,
    };

    for owner in [RootOwner::Publication, RootOwner::Selected] {
        for (form_index, (prefix, index)) in [
            (P::Marker, I::Staged),
            (P::Baseline, I::Pre),
            (P::Marker, I::Pre),
            (P::Lock, I::Pre),
            (P::Boundary, I::Pre),
            (P::Boundary, I::Staged),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("v1-root-handoff-{owner:?}-{form_index}");
            let mut fixture = if (prefix, index) == (P::Marker, I::Staged) {
                dirty_root_degenerate_handoff_fixture(&name, matches!(owner, RootOwner::Selected))
            } else {
                root_fixture(owner, &name)
            };
            install_root_handoff(&mut fixture, prefix, index);
            fixture.base.seed_open();
            let operation_context = fixture.base.context();
            let mut runtime = TraceRuntime {
                inner: ReverseRuntime::new(&fixture.base.backend, &operation_context),
                targets: Vec::new(),
            };
            let store = CheckedV1Store::default();
            let mut response = run(
                &store,
                &fixture.base.root.path,
                &fixture.base.model.merge_id,
                V1LifecycleRequest::Preserve,
                &mut runtime,
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
                        V1LifecycleRequest::Abort
                    } else {
                        V1LifecycleRequest::Preserve
                    },
                    &mut runtime,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{owner:?} {prefix:?}/{index:?} retry={retry}: {error:?}; state={:?}; preservation={:?}; rollback={:?}; targets={:?}",
                        response.current().record().state,
                        response.current().record().pending_preservation,
                        response.current().record().pending_rollback,
                        runtime.targets,
                    )
                });
            }
            assert_eq!(
                response.disposition(),
                V1ResponseDisposition::Terminal(OperationState::Aborted),
                "{owner:?} {prefix:?}/{index:?}"
            );
            assert_eq!(
                runtime.targets,
                expected_targets(),
                "{owner:?} {prefix:?}/{index:?}"
            );
        }
    }
}

#[test]
fn selected_root_no_candidate_handoff_exhausts_the_short_phase_graph() {
    use crate::workspace_ops::merge::model::v1::PreservationPublicationHandoffV1 as H;

    let handoff = H::NoCandidate;
    let mut fixture = dirty_selected_root_handoff_fixture("v1-selected-root-no-candidate");
    install_selected_root_no_candidate_handoff(&mut fixture);
    fixture.base.seed_open();
    let operation_context = fixture.base.context();
    let mut runtime = TraceRuntime {
        inner: ReverseRuntime::new(&fixture.base.backend, &operation_context),
        targets: Vec::new(),
    };
    let response = run(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::ResumeStart,
        &mut runtime,
    )
    .unwrap_or_else(|error| panic!("{handoff:?}: {error:?}"));
    assert_eq!(response.current().record().state, OperationState::Aborted);
    assert_eq!(runtime.targets, expected_absent_targets());
}

#[test]
fn evidence_pending_handoff_exhausts_a_non_root_short_phase_graph() {
    let fixture = evidence_pending_non_root_fixture("v1-evidence-pending-non-root");
    fixture.seed_open();
    let operation_context = fixture.context();
    let mut runtime = TraceRuntime {
        inner: ReverseRuntime::new(&fixture.backend, &operation_context),
        targets: Vec::new(),
    };
    let response = run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Archive,
        &mut runtime,
    )
    .unwrap();
    assert_eq!(response.current().record().state, OperationState::Aborted);
    assert_eq!(runtime.targets, expected_evidence_pending_targets());
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
pub(super) enum PhaseTarget {
    Backup,
    Stash(S),
    Reset(R),
}

impl PhaseTarget {
    pub(super) fn from_record(record: &MergeOperationRecordV1) -> Option<Self> {
        match record.pending_preservation.as_ref()? {
            PendingPreservationActionV1::BackupRef { .. } => Some(Self::Backup),
            PendingPreservationActionV1::Stash { phase, .. } => Some(Self::Stash(*phase)),
            PendingPreservationActionV1::ResetAttachedRef { phase, .. } => {
                Some(Self::Reset(*phase))
            }
        }
    }

    pub(super) fn matches_action(self, action: &PhysicalActionKind) -> bool {
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

pub(super) fn expected_targets() -> Vec<PhaseTarget> {
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

fn expected_absent_targets() -> Vec<PhaseTarget> {
    vec![
        PhaseTarget::Backup,
        PhaseTarget::Stash(S::CreateStash),
        PhaseTarget::Stash(S::WriteBundle),
        PhaseTarget::Stash(S::Complete),
        PhaseTarget::Reset(R::ResetRef),
        PhaseTarget::Reset(R::Complete),
    ]
}

fn expected_evidence_pending_targets() -> Vec<PhaseTarget> {
    vec![
        PhaseTarget::Stash(S::CreateStash),
        PhaseTarget::Stash(S::WriteBundle),
        PhaseTarget::Stash(S::Complete),
    ]
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
