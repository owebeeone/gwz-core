use std::path::Path;

use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    ResolvedV1Action, V1Invocation, V1LifecycleRequest, V1NextAction, V1ResponseDisposition,
    next_action, resolve_observation,
};
use super::checked::{StoredV1Record, V1MutationLease};
use super::events::{LifecycleEvents, observation_member};
use super::finalization::FinalizationRuntime;
use super::forward::ForwardRuntime;
use super::reverse::ReverseRuntime;
use super::store::CheckedV1Store;
use super::transition::prepare;
use crate::checked_artifact::entry::CrashRecoveryDecision;
use crate::git::MergeAuthorityBackend;
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

mod sealed {
    pub trait ProductionRuntime {}
}

impl<B: MergeAuthorityBackend> sealed::ProductionRuntime for ForwardRuntime<'_, B> {}
impl<B: MergeAuthorityBackend> sealed::ProductionRuntime for ReverseRuntime<'_, B> {}
impl<B: MergeAuthorityBackend> sealed::ProductionRuntime for FinalizationRuntime<'_, B> {}

#[allow(private_bounds)]
pub(super) trait V1Runtime:
    ExactObserver + PhysicalExecutor + sealed::ProductionRuntime
{
}

impl<T: ExactObserver + PhysicalExecutor + sealed::ProductionRuntime> V1Runtime for T {}

pub(super) struct V1ServiceResponse {
    current: StoredV1Record,
    disposition: V1ResponseDisposition,
}

impl V1ServiceResponse {
    #[allow(
        dead_code,
        reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item."
    )]
    pub(super) fn current(&self) -> &StoredV1Record {
        &self.current
    }

    pub(super) fn disposition(&self) -> V1ResponseDisposition {
        self.disposition
    }
}

/// Run one v1 service invocation.
///
/// DR-1 ship (1) W3 (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03):
/// `crash_recovery` is the decision the CALLER already made for this process —
/// a parameter, never a second probe. "Decide once" is what makes it a
/// parameter: a start's `ResumeStart` runs inside the same process as the
/// start's own decision and must neither re-probe nor re-warn. The reverse arms
/// ignore it entirely; `None` means "no decision was made for this invocation"
/// and takes today's behaviour (the forward arms activate).
pub(super) fn run<R: V1Runtime>(
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    request: V1LifecycleRequest,
    crash_recovery: Option<&CrashRecoveryDecision>,
    runtime: &mut R,
    events: &mut LifecycleEvents<'_>,
) -> ModelResult<V1ServiceResponse> {
    run_with_runtime(store, root, merge_id, request, crash_recovery, runtime, events)
}

#[cfg(test)]
pub(super) fn run_test<R: ExactObserver + PhysicalExecutor>(
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    request: V1LifecycleRequest,
    runtime: &mut R,
) -> ModelResult<V1ServiceResponse> {
    run_with_runtime(
        store,
        root,
        merge_id,
        request,
        None,
        runtime,
        &mut LifecycleEvents::silent(),
    )
}

fn run_with_runtime<R: ExactObserver + PhysicalExecutor>(
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    request: V1LifecycleRequest,
    crash_recovery: Option<&CrashRecoveryDecision>,
    runtime: &mut R,
    events: &mut LifecycleEvents<'_>,
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
    // E4.1 review [P1-1]/[P2-1] cure: only the FORWARD arms mutate a record
    // toward v1 semantics and so require the catalog; the reverse arms are on
    // E0.2 §5.2's capability-free list, and abort is the in-code exit for a v1
    // record stranded on a filesystem the catalog cannot use. SCOPED BY PATH
    // (2026-09-02, CapabilityFreeAmendment §6): a reverse arm re-verifying a
    // checked artifact — a bundle, a selected root's manifest and lock, or the
    // published evidence — still takes the legacy identity probe on this lease.
    //
    // DR-1 ship (1) W3 (charter §3.1): a forward arm activates only when the
    // caller's decision says crash recovery is SUPPORTED. Below the bar the
    // catalog cannot bind this volume's identity at all, so the forward arms
    // take the same plain, capability-free lease the reverse arms take — the
    // merge runs, without crash recovery, having warned once.
    let lease = match (request, crash_recovery) {
        (
            V1LifecycleRequest::ResumeStart | V1LifecycleRequest::Continue,
            Some(CrashRecoveryDecision::Unsupported { .. }),
        ) => V1MutationLease::acquire(root)?,
        (V1LifecycleRequest::ResumeStart | V1LifecycleRequest::Continue, _) => {
            V1MutationLease::acquire_activated(root)?
        }
        _ => V1MutationLease::acquire(root)?,
    };
    let mut current = store.load_open(root, merge_id)?;
    let mut attempt = None;
    let mut invocation = V1Invocation::new();

    loop {
        match invocation.next_action(&current, request)? {
            V1NextAction::Observe(observation_request) => {
                // M5d charter §4: `member_started` belongs to the participant
                // this observation selects, and lands at the first moment that
                // participant's work becomes visible — see `events.rs`.
                if let Some(member_id) = observation_member(observation_request.kind()) {
                    events.selected(member_id);
                }
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
                        events.before_commit(current.record(), rewrite.next());
                        let previous = current;
                        current = store.commit(&lease, &previous, rewrite)?;
                        events.committed(previous.record(), current.record());
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
                            events,
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
                        complete_response(&lease, store, &current, disposition, events)?;
                        return Ok(V1ServiceResponse {
                            current,
                            disposition,
                        });
                    }
                    ResolvedV1Action::Reject(error) => return Err(error),
                    // v0's `finalize_dispatch.rs:222-225`: persist the
                    // diagnostic, then re-raise the error that produced it. The
                    // record stays exactly where it was -- only
                    // `operation_drift` moves -- so the merge remains durably
                    // `Finalizing`, retryable and abortable.
                    ResolvedV1Action::RecordAndReject(transition, error) => {
                        let rewrite = prepare(&lease, &current, transition)?;
                        events.before_commit(current.record(), rewrite.next());
                        let previous = current;
                        current = store.commit(&lease, &previous, rewrite)?;
                        events.committed(previous.record(), current.record());
                        return Err(*error);
                    }
                }
            }
            V1NextAction::Apply(transition) => {
                let rewrite = prepare(&lease, &current, transition)?;
                events.before_commit(current.record(), rewrite.next());
                let previous = current;
                current = store.commit(&lease, &previous, rewrite)?;
                events.committed(previous.record(), current.record());
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
