use super::service_fault_matrix::{Lane, Target, context, expected_targets, fixture, seed_open};
use super::*;
use crate::model::{ErrorCode, ModelResult};
use crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, BoundObservationRequest, ExactObservationFact,
    ExecutionDiagnostic, PhysicalActionKind, V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{ExactObserver, PhysicalExecutor, run};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn every_rollback_action_retains_its_journal_on_fresh_ambiguous_observation() {
    for lane in [
        Lane::AbortConflict,
        Lane::ResetIntegrated,
        Lane::Evidence,
        Lane::SelectedRoot,
    ] {
        for (target_index, target) in expected_targets(lane).into_iter().enumerate() {
            for (request_index, request) in
                [V1LifecycleRequest::Abort, V1LifecycleRequest::Preserve]
                    .into_iter()
                    .enumerate()
            {
                let fixture = fixture(
                    lane,
                    &format!("v1-rollback-ambiguous-{lane:?}-{target_index}-{request_index}"),
                );
                seed_open(&fixture.root.path, &fixture.model);
                let operation_context = context(&fixture.model);
                let mut stop = StopAtTarget {
                    inner: ReverseRuntime::new(&fixture.backend, &operation_context),
                    target,
                    stopped: false,
                };
                let error = match run(
                    &CheckedV1Store::default(),
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    request,
                    &mut stop,
                ) {
                    Ok(_) => panic!("{lane:?} {target:?} did not stop at its pending action"),
                    Err(error) => error,
                };
                assert!(
                    stop.stopped,
                    "{lane:?} {target:?} {request:?} stopped early with {error:?}"
                );

                let store = CheckedV1Store::default();
                let pending = store
                    .load_open(&fixture.root.path, &fixture.model.merge_id)
                    .unwrap();
                let action = pending.record().pending_rollback.clone().unwrap();
                assert!(target.matches(&action), "{lane:?} {target:?}");
                let before = serde_yaml::to_string(pending.record()).unwrap();
                let mut ambiguous = AmbiguousRuntime;
                let response = run(
                    &store,
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    if request == V1LifecycleRequest::Abort {
                        V1LifecycleRequest::Preserve
                    } else {
                        V1LifecycleRequest::Abort
                    },
                    &mut ambiguous,
                )
                .unwrap();
                assert_eq!(
                    response.disposition(),
                    V1ResponseDisposition::Stopped(OperationState::RecoveryRequired),
                    "{lane:?} {target:?}"
                );
                assert_eq!(
                    response.current().record().pending_rollback.as_ref(),
                    Some(&action),
                    "{lane:?} {target:?}"
                );
                let after = serde_yaml::to_string(response.current().record()).unwrap();
                assert_ne!(after, before, "ambiguity must persist recovery state");
            }
        }
    }
}

struct StopAtTarget<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: Target,
    stopped: bool,
}

impl ExactObserver for StopAtTarget<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for StopAtTarget<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if Target::from_action(action) == Some(self.target) && !self.stopped {
            self.stopped = true;
            return ExecutionDiagnostic::Failed {
                code: ErrorCode::GitCommandFailed,
                message: "injected stop before ambiguity".into(),
                detail: None,
            };
        }
        self.inner.execute(lease, current, action)
    }
}

struct AmbiguousRuntime;

impl ExactObserver for AmbiguousRuntime {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let proof = BoundAmbiguityEvidence::for_test(
            current,
            "@operation",
            "enter_recovery",
            "ambiguous",
            RecoveryOriginStateV1::RollingBack,
        )?;
        BoundExactObservation::for_test(current, request, ExactObservationFact::Ambiguous(proof))
    }
}

impl PhysicalExecutor for AmbiguousRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        panic!("an ambiguous rollback observation must not execute")
    }
}
