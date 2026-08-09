use std::path::Path;

use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    ResolvedV1Action, V1Invocation, V1LifecycleRequest, V1NextAction, V1ResponseDisposition,
    next_action, resolve_observation,
};
use super::checked::{StoredV1Record, V1MutationLease};
use super::store::CheckedV1Store;
use super::transition::prepare;
use crate::model::ModelResult;

mod execution;

use execution::{ExecutionOutcome, complete_response, execute_owned};

pub(super) trait ExactObserver {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation>;
}

pub(super) trait PhysicalExecutor {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic;
}

pub(super) trait V1Runtime: ExactObserver + PhysicalExecutor {}

impl<T: ExactObserver + PhysicalExecutor> V1Runtime for T {}

pub(super) struct V1ServiceResponse {
    current: StoredV1Record,
    disposition: V1ResponseDisposition,
}

impl V1ServiceResponse {
    pub(super) fn current(&self) -> &StoredV1Record {
        &self.current
    }

    pub(super) fn disposition(&self) -> V1ResponseDisposition {
        self.disposition
    }
}

pub(super) fn run<R: V1Runtime>(
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    request: V1LifecycleRequest,
    runtime: &mut R,
) -> ModelResult<V1ServiceResponse> {
    let initial = store.load_open(root, merge_id)?;
    match next_action(&initial, request)? {
        V1NextAction::Respond(disposition) => {
            return Ok(V1ServiceResponse {
                current: initial,
                disposition,
            });
        }
        V1NextAction::Reject(error) => return Err(error),
        V1NextAction::Observe(_) | V1NextAction::Apply(_) => {}
    }
    let lease = V1MutationLease::acquire(root)?;
    let mut current = store.load_open(root, merge_id)?;
    let mut attempt = None;
    let mut invocation = V1Invocation::new();

    loop {
        match invocation.next_action(&current, request)? {
            V1NextAction::Observe(observation_request) => {
                invocation.observe(&current, &observation_request)?;
                let observation = runtime.observe(&current, &observation_request)?;
                match resolve_observation(
                    &current,
                    request,
                    observation_request,
                    observation,
                    attempt.take(),
                )? {
                    ResolvedV1Action::Apply(transition) => {
                        let rewrite = prepare(&lease, &current, transition)?;
                        current = store.commit(&lease, &current, rewrite)?;
                        if let Some(disposition) = invocation.after_commit(&current) {
                            return Ok(V1ServiceResponse {
                                current,
                                disposition,
                            });
                        }
                    }
                    ResolvedV1Action::Execute(action) => {
                        match execute_owned(
                            &lease,
                            store,
                            &current,
                            action,
                            &mut invocation,
                            runtime,
                        )? {
                            ExecutionOutcome::Attempt(value) => {
                                attempt = Some(*value);
                                current = store.reload_unchanged(&current)?;
                            }
                            ExecutionOutcome::Respond(disposition) => {
                                return Ok(V1ServiceResponse {
                                    current,
                                    disposition,
                                });
                            }
                        }
                    }
                    ResolvedV1Action::Respond(disposition) => {
                        complete_response(&lease, store, &current, disposition)?;
                        return Ok(V1ServiceResponse {
                            current,
                            disposition,
                        });
                    }
                    ResolvedV1Action::Reject(error) => return Err(error),
                }
            }
            V1NextAction::Apply(transition) => {
                let rewrite = prepare(&lease, &current, transition)?;
                current = store.commit(&lease, &current, rewrite)?;
                if let Some(disposition) = invocation.after_commit(&current) {
                    return Ok(V1ServiceResponse {
                        current,
                        disposition,
                    });
                }
            }
            V1NextAction::Respond(disposition) => {
                return Ok(V1ServiceResponse {
                    current,
                    disposition,
                });
            }
            V1NextAction::Reject(error) => return Err(error),
        }
    }
}

#[cfg(test)]
#[path = "tests/service.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/service_sequence.rs"]
mod sequence_tests;
