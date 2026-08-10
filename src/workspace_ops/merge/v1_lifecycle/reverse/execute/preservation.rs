use super::super::*;
use crate::git::GitCheckedPreservationMutation;

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn execute<B: GitBackend>(
    _backend: &B,
    _lease: &V1MutationLease,
    _current: &StoredV1Record,
    _action: &PhysicalActionKind,
) -> ExecutionDiagnostic {
    failure(route_error("preservation execution is not implemented"))
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn durability_diagnostic(
    result: ModelResult<GitCheckedPreservationMutation>,
) -> ExecutionDiagnostic {
    match result {
        Ok(GitCheckedPreservationMutation::Applied)
        | Ok(GitCheckedPreservationMutation::AlreadyComplete) => ExecutionDiagnostic::Success,
        Ok(_) => failure(route_error(
            "durability barrier returned a non-parent preservation mutation",
        )),
        Err(error) => failure(error),
    }
}
