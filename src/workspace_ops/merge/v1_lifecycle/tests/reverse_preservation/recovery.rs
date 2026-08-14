use super::*;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationOwnerV1, RecoveryContextV1, RecoveryOriginStateV1,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    V1LifecycleRequest, preserving_verify_recovery_origin,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::{StoredV1Record, V1MutationLease};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::{
    ExactObserver, PhysicalExecutor, run_test as run,
};
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn preserving_recovery_accepts_exact_before_and_after_backup_states() {
    let mut before = integrated_fixture("v1-preservation-recovery-before");
    enter_backup_recovery(&mut before);
    let current = StoredV1Record::for_test(&before.root.path, before.model.clone()).unwrap();
    preserving_verify_recovery_origin(&before.backend, &current).unwrap();

    let after = integrated_fixture("v1-preservation-recovery-after");
    let mut after = after;
    enter_backup_recovery(&mut after);
    let action = after.model.pending_preservation.as_ref().unwrap();
    let PendingPreservationActionV1::BackupRef {
        name,
        target_commit,
        ..
    } = action
    else {
        unreachable!()
    };
    after
        .backend
        .create_backup_ref(&after.member, name, target_commit)
        .unwrap();
    let current = StoredV1Record::for_test(&after.root.path, after.model.clone()).unwrap();
    preserving_verify_recovery_origin(&after.backend, &current).unwrap();
}

#[test]
fn preserving_recovery_rejects_a_foreign_backup_and_literal_origin_mismatch() {
    let mut foreign = integrated_fixture("v1-preservation-recovery-foreign");
    enter_backup_recovery(&mut foreign);
    let action = foreign.model.pending_preservation.as_ref().unwrap();
    let PendingPreservationActionV1::BackupRef { name, .. } = action else {
        unreachable!()
    };
    foreign
        .backend
        .create_backup_ref(&foreign.member, name, &foreign.result)
        .unwrap();
    let current = StoredV1Record::for_test(&foreign.root.path, foreign.model.clone()).unwrap();
    assert!(preserving_verify_recovery_origin(&foreign.backend, &current).is_err());

    let mut wrong = integrated_fixture("v1-preservation-recovery-origin");
    enter_backup_recovery(&mut wrong);
    wrong.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::RollingBack,
    });
    let error = match StoredV1Record::for_test(&wrong.root.path, wrong.model) {
        Ok(_) => panic!("mismatched recovery origin unexpectedly validated"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::RecoveryEvidenceMismatch);
}

#[test]
fn preserving_recovery_accepts_exact_before_and_after_stash_and_reset_states() {
    for target in [RecoveryTarget::Stash, RecoveryTarget::Reset] {
        for after in [false, true] {
            let fixture = dirty_integrated_fixture(&format!(
                "v1-preservation-recovery-{target:?}-{}",
                if after { "after" } else { "before" }
            ));
            fixture.seed_open();
            let context = fixture.context();
            let mut runtime = InterruptAtPreservationTarget {
                inner: ReverseRuntime::new(&fixture.backend, &context),
                target,
                after,
                interrupted: false,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(
                    &CheckedV1Store::default(),
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    V1LifecycleRequest::Preserve,
                    &mut runtime,
                )
            }));
            if after {
                assert!(result.is_err(), "{target:?} after-state did not interrupt");
            } else {
                assert!(
                    matches!(result, Ok(Err(ref error)) if error.code == ErrorCode::GitCommandFailed),
                    "{target:?} before-state did not stop with the retained journal",
                );
            }
            let open = CheckedV1Store::default()
                .load_open(&fixture.root.path, &fixture.model.merge_id)
                .unwrap();
            assert!(target.matches(open.record().pending_preservation.as_ref().unwrap()));
            let mut model = open.record().clone();
            model.state = OperationState::RecoveryRequired;
            model.recovery_context = Some(RecoveryContextV1 {
                origin_state: RecoveryOriginStateV1::Preserving,
            });
            let recovery = StoredV1Record::for_test(&fixture.root.path, model).unwrap();
            preserving_verify_recovery_origin(&fixture.backend, &recovery).unwrap();
        }
    }
}

#[test]
fn preserving_recovery_rejects_an_action_free_between_action_record() {
    let mut fixture = integrated_fixture("v1-preservation-recovery-action-free");
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Preserving,
    });
    fixture.model.pending_preservation = None;
    let error = match StoredV1Record::for_test(&fixture.root.path, fixture.model) {
        Ok(_) => panic!("action-free preserving recovery unexpectedly validated"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::RecoveryEvidenceMismatch);
}

fn enter_backup_recovery(fixture: &mut PreservationFixture) {
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Preserving,
    });
    fixture.model.pending_preservation = Some(PendingPreservationActionV1::BackupRef {
        owner: PreservationOwnerV1::Participant {
            member_id: "mem_a".into(),
        },
        name: format!("refs/gwz/merge/{}/mem_a/head", fixture.model.merge_id),
        target_commit: fixture.protected.clone(),
    });
}

#[derive(Clone, Copy, Debug)]
enum RecoveryTarget {
    Stash,
    Reset,
}

impl RecoveryTarget {
    fn matches(self, action: &PendingPreservationActionV1) -> bool {
        matches!(
            (self, action),
            (
                RecoveryTarget::Stash,
                PendingPreservationActionV1::Stash { .. }
            ) | (
                RecoveryTarget::Reset,
                PendingPreservationActionV1::ResetAttachedRef { .. }
            )
        )
    }
}

struct InterruptAtPreservationTarget<'a> {
    inner: ReverseRuntime<'a, Git2Backend>,
    target: RecoveryTarget,
    after: bool,
    interrupted: bool,
}

impl ExactObserver for InterruptAtPreservationTarget<'_> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> crate::model::ModelResult<BoundExactObservation> {
        self.inner.observe(current, request)
    }
}

impl PhysicalExecutor for InterruptAtPreservationTarget<'_> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let matches = matches!(
            action,
            PhysicalActionKind::Preservation(action) if self.target.matches(action)
        );
        if matches && !self.interrupted {
            self.interrupted = true;
            if !self.after {
                return ExecutionDiagnostic::Failed {
                    code: ErrorCode::GitCommandFailed,
                    message: "injected pre-mutation recovery boundary".into(),
                    detail: None,
                };
            }
            let diagnostic = self.inner.execute(lease, current, action);
            assert_eq!(diagnostic, ExecutionDiagnostic::Success);
            panic!("injected post-mutation recovery boundary");
        }
        self.inner.execute(lease, current, action)
    }
}
