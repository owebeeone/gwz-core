//! 1.1.0 S6.1: the cancellable entry, the variant of `with_local_transport`
//! for a host process such as gwz-py's extension (gwz-core
//! `dev-docs/GwzV110PlanAmendment.md` §3.4, as
//! `dev-docs/GwzTransportReleasePlanAmendment-2.md` §3.17 extends it; gwz-py
//! `dev-docs/GwzPyPerOperationTransportDesign.md` §2.2).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The runtime's endpoint configuration, TLS and proxy included, comes from the caller's environment snapshot, and the runtime reads no process environment. The transport timeout, which `configure_transport_runtime` sets, is read once, as the runtime starts | amendment 2 §3.17; design §2.2 |
//! | A token cancelled before the start refuses with `Cancelled` before anything is built. The request's registration with the token refuses once it is cancelled, and so does the check before the action runs | S6.1; contract §5.2 "Transport entry" |
//! | Cancelling the token cancels the running request, so its network I/O fails with `Cancelled`. The token is the request's only cancellation authority | S6.1; contract §5.2, §5.3 |
//! | Otherwise it builds, runs, finishes and shuts down as `with_local_transport` does, and every path that built a runtime ends it and returns its cleanup report | S6.1 |
//! | A panic in the action is caught first. Finish and shutdown then run outside the unwind, each under its own panic guard, and nothing finishes from a drop. The entry reports `internal_error` with cleanup unconfirmed | S6.1 "Library safety"; contract §5.2 "Panics" |

use super::{
    CleanupReport, HelperSlots, TransportRequest, TransportRuntime, endpoint_environment,
    unavailable,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::session_host::EnvironmentSnapshot;
use crate::{RequestMeta, git::Git2Backend};
use gwz_session_host::{CancellationToken, Refused};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `action` with a backend whose transport is one runtime of its own,
/// as `with_local_transport` does, for a caller that cancels it through
/// `token` and runs inside a process it must not take down.
///
/// The runtime is built from `environment`, never from the process
/// environment. The operation's request registers with `token`: once the
/// token is cancelled the entry refuses with `Cancelled`, before the action
/// runs, and while the action runs a cancel fails its network I/O with
/// `Cancelled`. Every path returns the cleanup report of what it built:
/// `(0, false)` when it built nothing, since no peer cleanup occurred.
///
/// The result is the action's value, or the entry's own failure: a refusal, a
/// setup failure, or `internal_error` when the action, finish or shutdown
/// panicked. The entry does not judge the action's value: an action that
/// returns after a cancel returns whatever it made of the cancelled I/O.
pub fn with_cancellable_local_transport<T>(
    meta: RequestMeta,
    operation: String,
    environment: &EnvironmentSnapshot,
    token: &CancellationToken,
    action: impl FnOnce(&Git2Backend) -> T,
) -> (ModelResult<T>, CleanupReport) {
    run(meta, operation, environment, token, action, &|_| {})
}

/// Uses provenance captured at the native Python entry before GIL detach/submission.
pub fn with_cancellable_local_transport_native<T>(
    meta: RequestMeta,
    operation: String,
    environment: &EnvironmentSnapshot,
    token: &CancellationToken,
    native: Option<super::NativeCaller>,
    action: impl FnOnce(&Git2Backend) -> T,
) -> (ModelResult<T>, CleanupReport) {
    run_native(meta, operation, environment, token, native, action, &|_| {})
}

/// The points at which `run` calls its observer. The public entry observes
/// nothing; a test cancels the token or injects a panic at one of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Step {
    /// The runtime is built, and the request is not yet registered.
    Built,
    /// The request is registered with the token, and the action has not
    /// started.
    Registered,
    /// The request's finish is about to run.
    Finish,
    /// The runtime's shutdown is about to run.
    Shutdown,
}

pub(super) fn run<T>(
    meta: RequestMeta,
    operation: String,
    environment: &EnvironmentSnapshot,
    token: &CancellationToken,
    action: impl FnOnce(&Git2Backend) -> T,
    observe: &dyn Fn(Step),
) -> (ModelResult<T>, CleanupReport) {
    run_native(meta, operation, environment, token, None, action, observe)
}
fn run_native<T>(
    meta: RequestMeta,
    operation: String,
    environment: &EnvironmentSnapshot,
    token: &CancellationToken,
    native: Option<super::NativeCaller>,
    action: impl FnOnce(&Git2Backend) -> T,
    observe: &dyn Fn(Step),
) -> (ModelResult<T>, CleanupReport) {
    if token.is_cancelled() {
        return (Err(Refused::Cancelled.into()), CleanupReport::default());
    }
    let owned = match catch_unwind(AssertUnwindSafe(|| Owned::build(environment, native))) {
        Ok(Ok(owned)) => owned,
        Ok(Err(error)) => return (Err(error), CleanupReport::default()),
        Err(_) => return (Err(panicked("setup")), unknown_cleanup()),
    };
    // From here every path ends the runtime through `close`.
    let registered = catch_unwind(AssertUnwindSafe(|| {
        observe(Step::Built);
        let request = owned
            .executor
            .block_on(owned.runtime.request_with_token(meta, operation, token));
        if request.is_ok() {
            observe(Step::Registered);
        }
        request
    }));
    let request = match registered {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => return (Err(error), owned.close(None, observe).report),
        Err(_) => {
            return (
                Err(panicked("setup")),
                owned.close(None, observe).unconfirmed(),
            );
        }
    };
    // A cancel after registration has already cancelled the request: refuse
    // rather than start the action on it.
    let ran = (!token.is_cancelled())
        .then(|| catch_unwind(AssertUnwindSafe(|| action(request.backend()))));
    let closed = owned.close(Some(request), observe);
    match ran {
        None => (Err(Refused::Cancelled.into()), closed.report),
        Some(Ok(value)) if !closed.panicked => (Ok(value), closed.report),
        Some(Ok(_)) => (Err(panicked("cleanup")), closed.report),
        Some(Err(_)) => (Err(panicked("operation")), closed.unconfirmed()),
    }
}

/// One operation's executor and runtime. It has no `Drop` of its own: `close`
/// ends it, outside any unwind. If building it panics, the parts built so far
/// drop without `close`, and their drops only close, never wait.
struct Owned {
    executor: tokio::runtime::Runtime,
    runtime: TransportRuntime,
}

/// What `Owned::close` found.
struct Closed {
    report: CleanupReport,
    /// Finish or shutdown panicked; the report is then unconfirmed.
    panicked: bool,
}

impl Closed {
    fn unconfirmed(self) -> CleanupReport {
        CleanupReport {
            peer_cleanup_confirmed: false,
            ..self.report
        }
    }
}

impl Owned {
    fn build(
        environment: &EnvironmentSnapshot,
        native: Option<super::NativeCaller>,
    ) -> ModelResult<Self> {
        let (ssh, https) = endpoint_environment::endpoint_config(environment)?;
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| unavailable("local transport executor unavailable"))?;
        // The entry is the driver: its HTTPS helper slots are created once
        // here and shared by the endpoints of the sessions it opens.
        let runtime = match native {
            Some(caller) => {
                TransportRuntime::with_https_native(ssh, https, HelperSlots::new(), caller)?
            }
            None => TransportRuntime::with_https(ssh, https, HelperSlots::new())?,
        };
        Ok(Self { executor, runtime })
    }

    /// Finishes `request`, if there is one, and then shuts the runtime down,
    /// each under its own panic guard: `with_local_transport`'s finish,
    /// guarded, and never run from a drop. If finish panics, shutdown still
    /// runs. If shutdown panics, how much local work remains is unknown, and
    /// the report counts at least one job.
    fn close(self, request: Option<TransportRequest>, observe: &dyn Fn(Step)) -> Closed {
        let Self { executor, runtime } = self;
        let finished = request.map(|request| {
            catch_unwind(AssertUnwindSafe(|| {
                observe(Step::Finish);
                executor.block_on(request.finish())
            }))
        });
        let shut = catch_unwind(AssertUnwindSafe(|| {
            observe(Step::Shutdown);
            executor.block_on(runtime.shutdown())
        }));
        let operation = match finished {
            None => Ok(None),
            Some(Ok(report)) => Ok(Some(report)),
            Some(Err(_)) => Err(()),
        };
        match (operation, shut) {
            (Ok(operation), Ok(runtime)) => Closed {
                report: CleanupReport {
                    // Shutdown is the final snapshot of the same owned work,
                    // not a second set of jobs to add to the operation's.
                    pending_local_work: runtime.pending_local_work,
                    peer_cleanup_confirmed: runtime.peer_cleanup_confirmed
                        && operation.is_none_or(|report| report.peer_cleanup_confirmed),
                },
                panicked: false,
            },
            (Err(()), Ok(runtime)) => Closed {
                report: CleanupReport {
                    pending_local_work: runtime.pending_local_work,
                    peer_cleanup_confirmed: false,
                },
                panicked: true,
            },
            (operation, Err(_)) => Closed {
                report: CleanupReport {
                    pending_local_work: operation
                        .ok()
                        .flatten()
                        .map_or(0, |report| report.pending_local_work)
                        .max(1),
                    peer_cleanup_confirmed: false,
                },
                panicked: true,
            },
        }
    }
}

/// The report when a panic left unknown how much local work remains.
fn unknown_cleanup() -> CleanupReport {
    CleanupReport {
        pending_local_work: 1,
        peer_cleanup_confirmed: false,
    }
}

fn panicked(what: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InternalError,
        format!("transport {what} panicked"),
    )
}
