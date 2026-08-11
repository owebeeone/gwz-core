use super::super::super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1;

pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_recovery_origin<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<VerifiedRecoveryOrigin> {
    let context = current
        .record()
        .recovery_context
        .as_ref()
        .ok_or_else(|| recovery_error("preserving recovery has no retained origin context"))?;
    if context.origin_state != RecoveryOriginStateV1::Preserving {
        return Err(recovery_error(
            "preservation verifier received a different recovery origin",
        ));
    }
    if !super::preservation::pending_recovery_is_exact(backend, current)? {
        return Err(recovery_error(
            "live preservation state is neither the exact before nor after state",
        ));
    }
    VerifiedRecoveryOrigin::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "resume_recovery",
        "verified",
        RecoveryOriginStateV1::Preserving,
    )
}

fn recovery_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::RecoveryEvidenceMismatch, detail.into())
}
