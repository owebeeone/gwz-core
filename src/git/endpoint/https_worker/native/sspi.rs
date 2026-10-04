//! Caller capture and the gwz_sspi supervisor binding of the native Port/Session seam.
use super::*;

/// A host captures this on the original entry, before any fanout or detach.
/// Availability is retained so anonymous, Basic and SSH do not require SSPI.
#[derive(Clone)]
pub struct NativeCaller {
    pub(super) port: Result<Arc<dyn Port>, gwz_sspi::ErrorKind>,
    pub(super) qualification_direct: Option<bool>,
}
impl NativeCaller {
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn refused_for_test(kind: gwz_sspi::ErrorKind) -> Self { Self { port: Err(kind), qualification_direct: None } }
    } }

    pub(crate) fn qualification_direct(&self) -> Option<bool> {
        self.qualification_direct
    }

    pub fn capture(supervisor: &Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind>) -> Self {
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
        let qualification_direct =
            crate::transport_host::endpoint_environment::capture_qualification_proxy();
        Self {
            port,
            qualification_direct,
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
