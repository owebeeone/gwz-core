use super::*;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    ParticipantRollbackKindV1, PendingRollbackActionV1, RecoveryContextV1, RecoveryOriginStateV1,
};
use crate::workspace_ops::merge::v1_lifecycle::authority::rolling_back_verify_recovery_origin;
use crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record;

#[test]
fn rolling_back_recovery_accepts_exact_before_and_rejects_third_state() {
    let mut fixture = integrated_fixture("v1-rollback-recovery");
    fixture.model.state = OperationState::RecoveryRequired;
    fixture.model.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::RollingBack,
    });
    fixture.model.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    });
    let current = StoredV1Record::for_test(&fixture.root.path, fixture.model.clone()).unwrap();
    rolling_back_verify_recovery_origin(&fixture.backend, &current).unwrap();

    std::fs::write(fixture.member.join("untracked"), "drift\n").unwrap();
    let error = match rolling_back_verify_recovery_origin(&fixture.backend, &current) {
        Ok(_) => panic!("ambiguous rollback recovery unexpectedly resumed"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::RecoveryEvidenceMismatch);
}
