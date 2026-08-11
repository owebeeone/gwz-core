use super::service_fault_matrix::{Lane, Target, context, expected_targets, fixture, seed_open};
use super::*;
use crate::model::{ErrorCode, ModelResult};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, V1ResponseDisposition,
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
            for (request_index, request) in admitted_requests().into_iter().enumerate() {
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
                install_real_ambiguity(&fixture, pending.record(), target);
                let operation_context = context(&fixture.model);
                let mut ambiguous = ReverseRuntime::new(&fixture.backend, &operation_context);
                let response = run(
                    &store,
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    admitted_requests()[(request_index + 1) % admitted_requests().len()],
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

fn admitted_requests() -> [V1LifecycleRequest; 5] {
    [
        V1LifecycleRequest::ResumeStart,
        V1LifecycleRequest::Continue,
        V1LifecycleRequest::Abort,
        V1LifecycleRequest::Preserve,
        V1LifecycleRequest::Archive,
    ]
}

fn install_real_ambiguity(
    fixture: &super::service_fault_matrix::MatrixFixture,
    model: &MergeOperationRecordV1,
    target: Target,
) {
    use crate::artifact::LOCK_PATH;
    use crate::workspace::WORKSPACE_MANIFEST;
    use crate::workspace_ops::merge::model::v1::{
        EvidenceRollbackStepV1 as E, ParticipantRollbackKindV1 as P,
        RootMetadataRollbackStepV1 as R,
    };

    match target {
        Target::Participant(P::AbortConflict) | Target::Participant(P::ResetIntegrated) => {
            let row = &model.participants["mem_a"];
            std::fs::write(
                fixture.root.path.join(&row.path).join("README.md"),
                "real third-form participant checkout\n",
            )
            .unwrap();
        }
        Target::Evidence(E::EvidenceCommit) => {
            let publication = model.publication.as_ref().unwrap();
            let candidate = publication.candidate.as_ref().unwrap();
            let repository = git2::Repository::open(&fixture.root.path).unwrap();
            let composition = repository
                .find_commit(
                    publication
                        .composition_commit
                        .as_deref()
                        .unwrap()
                        .parse()
                        .unwrap(),
                )
                .unwrap();
            let tree = composition.tree().unwrap();
            let signature = git2::Signature::now("GWZ Test", "gwz-test@example.invalid").unwrap();
            let third = repository
                .commit(
                    None,
                    &signature,
                    &signature,
                    "real third-form evidence head",
                    &tree,
                    &[&composition],
                )
                .unwrap();
            let mut reference = repository
                .find_reference(&format!("refs/heads/{}", candidate.root_branch))
                .unwrap();
            reference
                .set_target(third, "install real third-form evidence HEAD")
                .unwrap();
        }
        Target::Evidence(E::Boundary) => std::fs::write(
            crate::workspace_ops::workspace_exclude_path(&fixture.root.path),
            "real third-form boundary\n",
        )
        .unwrap(),
        Target::Evidence(E::Lock) => std::fs::write(
            fixture.root.path.join(LOCK_PATH),
            "real third-form evidence lock\n",
        )
        .unwrap(),
        Target::Evidence(E::Marker) => {
            let marker = model
                .publication
                .as_ref()
                .unwrap()
                .candidate_marker_path
                .as_ref()
                .unwrap();
            std::fs::write(
                fixture.root.path.join(marker),
                "real third-form evidence marker\n",
            )
            .unwrap();
        }
        Target::Evidence(E::Index) => {
            let marker = model
                .publication
                .as_ref()
                .unwrap()
                .candidate_marker_path
                .as_ref()
                .unwrap();
            std::fs::write(
                fixture.root.path.join(marker),
                "real third-form evidence index\n",
            )
            .unwrap();
            fixture
                .backend
                .stage_paths(&fixture.root.path, &[marker])
                .unwrap();
        }
        Target::Evidence(E::Complete) => unreachable!(),
        Target::Root(R::Manifest) => std::fs::write(
            fixture.root.path.join(WORKSPACE_MANIFEST),
            "real third-form selected-root manifest\n",
        )
        .unwrap(),
        Target::Root(R::Lock) => std::fs::write(
            fixture.root.path.join(LOCK_PATH),
            "real third-form selected-root lock\n",
        )
        .unwrap(),
        Target::Root(R::Complete) => unreachable!(),
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
