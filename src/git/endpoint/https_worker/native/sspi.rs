//! Caller capture and the gwz_sspi supervisor binding of the native Port/Session seam.
use super::*;
use crate::git::{Git2Backend, spawn_gate::SpawnGate};

/// A host captures this on the original entry, before any fanout or detach.
/// Availability is retained so anonymous, Basic and SSH do not require SSPI.
#[derive(Clone)]
pub struct NativeCaller {
    pub(super) port: Result<Arc<dyn Port>, gwz_sspi::ErrorKind>,
    /// The WinHTTP machine proxy, read with the caller (Windows only; plan step 4.6).
    pub(super) machine_proxy: Option<crate::transport_host::endpoint_environment::MachineProxy>,
    /// The host's spawn gate (WH2 contract C9; operator decisions D25 and D31). It is the one the supervisor's
    /// launch guard takes, and the transport the caller enters builds its helper slots, and so every backend of
    /// its requests, on it.
    pub(super) gate: SpawnGate,
}

/// A host's native route: the SSPI supervisor and the spawn gate its worker launches take (contract C9; operator
/// decision D31). A host makes one, shares it by reference with every operation it runs, and takes a
/// [`NativeCaller`] from it on each operation's original entry thread. The gate and the supervisor's launch guard
/// are made together here, so the gate the host's other spawns hold is the gate a worker's creation window holds,
/// by construction. A host makes no gate of its own.
#[derive(Clone)]
pub struct NativeHost {
    supervisor: Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind>,
    gate: SpawnGate,
}
impl NativeHost {
    /// The host's native route over `worker`, the installed worker's descriptor or why there is none. Where SSPI
    /// is unavailable (other than Windows, or no worker) the callers are still made, refusing the native route.
    pub fn new(worker: Result<gwz_sspi::WorkerExecutable, gwz_sspi::ErrorKind>) -> Self {
        Self::on_gate(SpawnGate::new(), |guard| {
            worker.and_then(|worker| {
                gwz_sspi::Supervisor::with_launch_guard(worker, gwz_sspi::Options::default(), guard)
                    .map(Arc::new)
                    .map_err(|error| error.kind())
            })
        })
    }

    /// `new` over `gate`, with the supervisor made by `supervisor` from the guard that takes `gate`.
    pub(crate) fn on_gate(
        gate: SpawnGate,
        supervisor: impl FnOnce(
            gwz_sspi::LaunchGuard,
        ) -> Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind>,
    ) -> Self {
        let supervisor = supervisor(launch_guard(&gate));
        Self { supervisor, gate }
    }

    /// The caller of the operation that is on its original entry thread, before any fanout or detach.
    pub fn caller(&self) -> NativeCaller {
        NativeCaller::captured(&self.supervisor, &self.gate)
    }

    /// A backend for an operation that runs outside the transport (a non-network request, or one under the transport
    /// off switch), carrying this host's gate: its Git CLI spawns (`git commit`, `git tag`) and libgit2's credential
    /// helper take the gate the supervisor's worker launches take, so they do not overlap a launch's window (contract
    /// C9; operator decision D47). It is `Git2Backend::new()` otherwise, made when this is called.
    pub fn backend(&self) -> Git2Backend {
        let mut backend = Git2Backend::new();
        backend.spawn_gate = self.gate.clone();
        backend
    }

    /// Whether `backend` is on this host's gate: one made by [`Self::backend`], or by the transport this host's
    /// callers enter, and not one of another host's or a plain `Git2Backend::new()`.
    pub fn gates(&self, backend: &Git2Backend) -> bool {
        self.gate.same_gate(&backend.spawn_gate)
    }

    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn supervisor(&self) -> &Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind> {
            &self.supervisor
        }
        pub(crate) fn gate(&self) -> &SpawnGate {
            &self.gate
        }
    } }
}

/// The guard gwz-sspi takes around each worker's creation window: it holds `gate`, as every other spawn of the
/// host that names no handle list does, so none of them overlaps a window.
pub(crate) fn launch_guard(gate: &SpawnGate) -> gwz_sspi::LaunchGuard {
    let gate = gate.clone();
    gwz_sspi::LaunchGuard::new(move || gate.hold())
}

impl NativeCaller {
    /// The host's spawn gate, which the transport this caller enters shares with the supervisor's launch guard.
    pub(crate) fn spawn_gate(&self) -> &SpawnGate {
        &self.gate
    }

    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn refused_for_test(kind: gwz_sspi::ErrorKind) -> Self { Self { port: Err(kind), machine_proxy: None, gate: SpawnGate::new() } }
    } }

    pub(crate) fn machine_proxy(
        &self,
    ) -> Option<crate::transport_host::endpoint_environment::MachineProxy> {
        self.machine_proxy.clone()
    }

    fn captured(
        supervisor: &Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind>,
        gate: &SpawnGate,
    ) -> Self {
        let port = supervisor
            .as_ref()
            .map_err(|kind| *kind)
            .and_then(|supervisor| {
                supervisor
                    .capture_caller()
                    .map(|caller| {
                        Arc::new(NativePort {
                            supervisor: supervisor.clone(),
                            caller,
                        }) as Arc<dyn Port>
                    })
                    .map_err(|error| error.kind())
            });
        let machine_proxy = crate::transport_host::endpoint_environment::capture_machine_proxy();
        Self {
            port,
            machine_proxy,
            gate: gate.clone(),
        }
    }
}
struct NativeProbe {
    supervisor: Arc<gwz_sspi::Supervisor>,
    id: gwz_sspi::RecordId,
}
impl Probe for NativeProbe {
    fn confirmed(&self) -> bool {
        self.supervisor.cleanup_status(self.id.clone()) == gwz_sspi::CleanupStatus::Confirmed
    }
}
struct NativePort {
    supervisor: Arc<gwz_sspi::Supervisor>,
    caller: gwz_sspi::CallerCapture,
}
fn bridge_error(error: gwz_sspi::Failure, supervisor: &Arc<gwz_sspi::Supervisor>) -> BridgeError {
    let pending = error
        .record_id()
        .filter(|_| error.cleanup_status() != gwz_sspi::CleanupStatus::Confirmed)
        .map(|id| {
            Box::new(NativeProbe {
                supervisor: supervisor.clone(),
                id: id.clone(),
            }) as Box<dyn Probe>
        });
    BridgeError {
        code: native_code(error.kind()),
        pending,
    }
}
impl Port for NativePort {
    fn start(
        &self,
        request: gwz_sspi::AuthRequest,
        deadline: Instant,
        cancel: gwz_sspi::Cancellation,
    ) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
        let work = self.supervisor.start_captured(
            &self.caller,
            request,
            gwz_sspi::Deadline::new(deadline.into_std()),
            cancel,
        );
        let supervisor = self.supervisor.clone();
        Box::pin(async move {
            work.await
                .map(|conversation| {
                    Box::new(NativeSession {
                        supervisor: supervisor.clone(),
                        conversation,
                    }) as Box<dyn Session>
                })
                .map_err(|error| bridge_error(error, &supervisor))
        })
    }
}
struct NativeSession {
    supervisor: Arc<gwz_sspi::Supervisor>,
    conversation: gwz_sspi::Conversation,
}
impl Session for NativeSession {
    fn step(
        &mut self,
        challenge: Option<gwz_sspi::SecretBytes>,
    ) -> Work<'_, Result<gwz_sspi::TokenStep, BridgeError>> {
        Box::pin(async move {
            self.conversation
                .step(challenge)
                .await
                .map_err(|error| bridge_error(error, &self.supervisor))
        })
    }
    fn finish(self: Box<Self>) -> Work<'static, Result<(), BridgeError>> {
        Box::pin(async move {
            self.conversation
                .finish()
                .await
                .map_err(|error| bridge_error(error, &self.supervisor))
        })
    }
    fn cancel(self: Box<Self>) -> Option<Box<dyn Probe>> {
        let NativeSession {
            supervisor,
            conversation,
        } = *self;
        let receipt = conversation.cancel();
        (receipt.cleanup != gwz_sspi::CleanupStatus::Confirmed).then(|| {
            Box::new(NativeProbe {
                supervisor,
                id: receipt.record_id,
            }) as Box<dyn Probe>
        })
    }
}
