use super::service_fault_matrix::{Lane, Target, context, fixture, seed_open, seed_recovery};
use super::*;
use crate::checked_artifact::{CheckedArtifactFault, fail_next_checked_artifact_at};
use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::{
    EvidenceRollbackStepV1 as E, RootMetadataRollbackStepV1 as R,
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
fn checked_rollback_consumers_recover_across_both_durability_sides() {
    for (lane, targets) in [
        (
            Lane::Evidence,
            vec![
                Target::Evidence(E::Boundary),
                Target::Evidence(E::Lock),
                Target::Evidence(E::Marker),
            ],
        ),
        (
            Lane::SelectedRoot,
            vec![Target::Root(R::Manifest), Target::Root(R::Lock)],
        ),
    ] {
        for target in targets {
            for fault in [
                CheckedArtifactFault::BeforeDurability,
                CheckedArtifactFault::AfterDurability,
            ] {
                let fixture = fixture(
                    lane,
                    &format!("v1-rollback-durability-{lane:?}-{target:?}-{fault:?}"),
                );
                seed_open(&fixture.root.path, &fixture.model);
                let operation_context = context(&fixture.model);
                let mut interrupted = DurabilityRuntime {
                    inner: ReverseRuntime::new(&fixture.backend, &operation_context),
                    target,
                    fault,
                    injected: false,
                };
                let _ = run(
                    &CheckedV1Store::default(),
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    V1LifecycleRequest::Abort,
                    &mut interrupted,
                );
                assert!(interrupted.injected, "{lane:?} {target:?} {fault:?}");
                let store = CheckedV1Store::default();
                let retained = store
                    .load_open(&fixture.root.path, &fixture.model.merge_id)
                    .unwrap();
                assert!(
                    retained
                        .record()
                        .pending_rollback
                        .as_ref()
                        .is_some_and(|action| target.matches(action)),
                    "{lane:?} {target:?} {fault:?} advanced before recovery"
                );
                seed_recovery(&fixture.root.path, retained.record());
                let operation_context = context(&fixture.model);
                let mut resume = ReverseRuntime::new(&fixture.backend, &operation_context);
                let response = run(
                    &store,
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    V1LifecycleRequest::Preserve,
                    &mut resume,
                )
                .unwrap_or_else(|error| panic!("{lane:?} {target:?} {fault:?}: {error:?}"));
                assert_eq!(
                    response.disposition(),
                    V1ResponseDisposition::Terminal(OperationState::Aborted),
                    "{lane:?} {target:?} {fault:?}"
                );
                assert_final_artifacts(lane, &fixture);
            }
        }
    }
}

fn assert_final_artifacts(lane: Lane, fixture: &super::service_fault_matrix::MatrixFixture) {
    match lane {
        Lane::Evidence => {
            let publication = fixture.model.publication.as_ref().unwrap();
            let candidate = publication.candidate.as_ref().unwrap();
            assert_eq!(
                std::fs::read_to_string(crate::workspace_ops::workspace_exclude_path(
                    &fixture.root.path,
                ))
                .unwrap(),
                candidate.baseline_boundary_text
            );
            assert_eq!(
                std::fs::read_to_string(fixture.root.path.join(crate::artifact::LOCK_PATH),)
                    .unwrap(),
                candidate.baseline_lock_yaml
            );
            assert!(
                !fixture
                    .root
                    .path
                    .join(publication.candidate_marker_path.as_ref().unwrap())
                    .exists()
            );
        }
        Lane::SelectedRoot => {
            assert_eq!(
                std::fs::read_to_string(
                    fixture.root.path.join(crate::workspace::WORKSPACE_MANIFEST),
                )
                .unwrap(),
                fixture.model.baseline.manifest_yaml.as_deref().unwrap()
            );
            assert_eq!(
                std::fs::read_to_string(fixture.root.path.join(crate::artifact::LOCK_PATH),)
                    .unwrap(),
                fixture.model.baseline.lock_yaml.as_deref().unwrap()
            );
        }
        Lane::AbortConflict | Lane::ResetIntegrated => unreachable!(),
    }
}

struct DurabilityRuntime<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: Target,
    fault: CheckedArtifactFault,
    injected: bool,
}

impl ExactObserver for DurabilityRuntime<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for DurabilityRuntime<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if !self.injected
            && matches!(
                action,
                PhysicalActionKind::Rollback(pending) if self.target.matches(pending)
            )
        {
            self.injected = true;
            fail_next_checked_artifact_at(self.fault);
        }
        self.inner.execute(lease, current, action)
    }
}
