use super::matrix_spec::REQUESTS;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExactObservationFact, ExecutionDiagnostic,
    NotStartedObservation, PhysicalActionKind, PreservationCursorPosition, ResolvedV1Action,
    V1NextAction, next_action, resolve_observation,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::workspace_ops::merge::v1_lifecycle::tests::dispatcher_attempt_matrix::{
    preservation_record, rollback_action, rollback_record,
};
use crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::{
    backup_action, preservation_prefix,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn every_admitted_request_reaches_both_reverse_physical_relations() {
    let root = TempDir::new_git("merge-v1-c7-request-dispatch");

    for request in REQUESTS {
        let current = StoredV1Record::for_test(&root.path, preservation_record()).unwrap();
        let V1NextAction::Observe(observation_request) = next_action(&current, request).unwrap()
        else {
            panic!("{request:?} did not observe the preserving owner")
        };
        let observation = BoundExactObservation::for_test(
            &current,
            &observation_request,
            ExactObservationFact::NotStarted(NotStartedObservation::Preservation {
                action: backup_action(),
                prefix: preservation_prefix(&current, PreservationCursorPosition::BackupRef),
            }),
        )
        .unwrap();
        assert!(matches!(
            resolve_observation(&current, request, observation_request, observation, None,)
                .unwrap(),
            ResolvedV1Action::Execute(_)
        ));

        let current = StoredV1Record::for_test(&root.path, rollback_record()).unwrap();
        let V1NextAction::Observe(observation_request) = next_action(&current, request).unwrap()
        else {
            panic!("{request:?} did not observe the rolling-back owner")
        };
        let observation = BoundExactObservation::for_test(
            &current,
            &observation_request,
            ExactObservationFact::NotStarted(NotStartedObservation::Rollback(rollback_action())),
        )
        .unwrap();
        assert!(matches!(
            resolve_observation(&current, request, observation_request, observation, None,)
                .unwrap(),
            ResolvedV1Action::Execute(_)
        ));
    }
}

#[test]
fn operational_observer_errors_retain_both_reverse_owners_for_every_request() {
    for (request_index, request) in REQUESTS.into_iter().enumerate() {
        for (lane, model) in [
            ("preserving", preservation_record()),
            ("rolling-back", rollback_record()),
        ] {
            let root = TempDir::new_git(&format!("merge-v1-c7-operational-{lane}-{request_index}"));
            let merge_root = root.path.join(".gwz/merge");
            std::fs::create_dir_all(&merge_root).unwrap();
            let record_path = merge_root.join(format!("{}.yaml", model.merge_id));
            std::fs::write(&record_path, serde_yaml::to_string(&model).unwrap()).unwrap();
            let before = std::fs::read(&record_path).unwrap();
            let mut runtime = OperationalErrorRuntime { executions: 0 };

            let error = match run(
                &CheckedV1Store::default(),
                &root.path,
                &model.merge_id,
                request,
                &mut runtime,
            ) {
                Ok(_) => panic!("{lane}/{request:?} ignored its observer error"),
                Err(error) => error,
            };
            assert_eq!(
                error.code,
                ErrorCode::GitCommandFailed,
                "{lane}/{request:?}"
            );
            assert_eq!(runtime.executions, 0, "{lane}/{request:?}");
            assert_eq!(
                std::fs::read(record_path).unwrap(),
                before,
                "{lane}/{request:?}"
            );
        }
    }
}

struct OperationalErrorRuntime {
    executions: usize,
}

impl ExactObserver for OperationalErrorRuntime {
    fn observe(
        &mut self,
        _current: &StoredV1Record,
        _request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "injected operational observer error",
        ))
    }
}

impl PhysicalExecutor for OperationalErrorRuntime {
    fn execute(
        &mut self,
        _lease: &V1MutationLease,
        _current: &StoredV1Record,
        _action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        self.executions += 1;
        ExecutionDiagnostic::Success
    }
}
