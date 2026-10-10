use super::*;
use crate::git::endpoint::ssh_setup_context;
pub(in crate::git::endpoint) struct OpenRequest {
    pub(in crate::git::endpoint) key: Key,
    pub(in crate::git::endpoint) identity: Identity,
    pub(in crate::git::endpoint) service: GitService,
    pub(in crate::git::endpoint) path: String,
    pub(in crate::git::endpoint) deadline: Option<u64>,
    pub(in crate::git::endpoint) cancelled: Arc<AtomicBool>,
    pub(in crate::git::endpoint) reply: Option<SyncSender<OpenOutcome>>,
    pub(in crate::git::endpoint) selected: Option<PathBuf>,
    /// Pins the admitted key snapshot, which the registry holds only weakly,
    /// until the open completes: the connector finds it there by identity.
    pub(in crate::git::endpoint) authority: Option<Arc<Entry>>,
    /// What the open's URL holds beyond its destination, which the setup of a
    /// connection opened for it uses (TR2.18).
    pub(in crate::git::endpoint) url: Option<Arc<UrlExtras>>,
    pub(in crate::git::endpoint) progress: Progress,
    pub(in crate::git::endpoint) setup_slot:
        Arc<Mutex<Option<Arc<ssh_setup_context::SetupContext>>>>,
    pub(in crate::git::endpoint) permit: Permit,
    pub(in crate::git::endpoint) context: BridgeContext,
    /// Names the member the open's connection serves, for the limit machines.
    pub(in crate::git::endpoint) tag: Option<String>,
    /// The open carries a test of the site's limit: never deferred.
    pub(in crate::git::endpoint) carrier: bool,
}
pub(in crate::git::endpoint) type OpenOutcome = io::Result<(EndpointAttachment, Opened)>;
/// Sanitized endpoint outcome; native diagnostics and credential paths stay
/// local. `phase` says whether a started setup failed, which only this side
/// of the transport knows (the retry plan's §4).
#[derive(Debug)]
pub(crate) struct EndpointOpenFailure {
    pub(crate) failure: Failure,
    pub(crate) phase: Phase,
}
impl std::fmt::Display for EndpointOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SSH endpoint open failed: {:?}", self.failure.code)
    }
}
impl std::error::Error for EndpointOpenFailure {}
impl EndpointOpenFailure {
    pub(in crate::git::endpoint) fn capture(
        error: io::Error,
        facts: Facts,
        setup: Option<Arc<ssh_setup_context::SetupContext>>,
    ) -> io::Error {
        use gwz_transport::pool::Error as PoolError;
        use gwz_transport::protocol::SetupFailureCause;
        let pool = error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<PoolError>());
        let phase = pool.map_or(Phase::Other, setup_retry::phase_of);
        if let (Some(PoolError::SetupEnded(record)), Some(context)) = (pool, setup) {
            let mut failure = context.failure(*record);
            failure.facts = Some(facts);
            return io::Error::new(error.kind(), Self { failure, phase });
        }
        let (code, effect, setup_cause) = match pool {
            Some(PoolError::ConnectFailed {
                code,
                effect,
                setup_cause,
            }) => (*code, *effect, *setup_cause),
            Some(PoolError::AllocationTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Allocation),
            ),
            Some(PoolError::ConnectTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Aggregate),
            ),
            Some(PoolError::InteractionTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Interaction),
            ),
            Some(PoolError::Capacity | PoolError::WouldBlock | PoolError::LocalWaitExpired) => {
                (ErrorCode::Capacity, Effect::None, None)
            }
            Some(PoolError::Cancelled) => (ErrorCode::Cancelled, Effect::None, None),
            // The setup authenticated as an identity other than the one asked.
            Some(PoolError::IdentityMismatch) => (ErrorCode::Authentication, Effect::None, None),
            Some(PoolError::DriverLost | PoolError::Shutdown) => {
                (ErrorCode::CarrierLost, Effect::None, None)
            }
            _ => (
                match error.kind() {
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
                        ErrorCode::Unavailable
                    }
                    io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
                    io::ErrorKind::TimedOut => ErrorCode::Timeout,
                    io::ErrorKind::WouldBlock => ErrorCode::Capacity,
                    io::ErrorKind::ConnectionAborted => ErrorCode::Cancelled,
                    io::ErrorKind::BrokenPipe => ErrorCode::CarrierLost,
                    _ => ErrorCode::Io,
                },
                Effect::None,
                None,
            ),
        };
        io::Error::new(
            error.kind(),
            Self {
                failure: Failure {
                    detail: None,
                    setup_cause,
                    code,
                    effect,
                    facts: Some(facts),
                },
                phase,
            },
        )
    }
}
impl OpenRequest {
    pub(in crate::git::endpoint) fn has_reply(&self) -> bool {
        self.reply.is_some()
    }
    pub(in crate::git::endpoint) fn reject(&mut self, kind: io::ErrorKind) {
        if let Some(reply) = self.reply.take() {
            let facts = self
                .progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let setup = self
                .setup_slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let error = Self::expiry_error(setup.as_ref(), kind);
            let _ = reply.send(Err(EndpointOpenFailure::capture(error, facts, setup)));
            // The bridge's owner polls for the reply between its passes.
            if let Some(waker) = &self.context.waker {
                waker.wake_by_ref();
            }
        }
    }

    /// What ends an expired request: its setup's own terminal, which names a
    /// cancellation or a budget, else `kind`.
    fn expiry_error(
        setup: Option<&Arc<ssh_setup_context::SetupContext>>,
        kind: io::ErrorKind,
    ) -> io::Error {
        setup
            .and_then(|context| match context.clock.observe().deliver() {
                gwz_transport::pool::Observation::Terminal(record) => Some(io::Error::other(
                    gwz_transport::pool::Error::SetupEnded(record),
                )),
                _ => None,
            })
            .unwrap_or_else(|| kind.into())
    }

    /// Completes a request that `expired` found expired, with the same failure
    /// `reject` gives it: whichever pass of the worker sees a cancellation
    /// first, the open fails as cancelled, not as `kind`.
    pub(in crate::git::endpoint) fn complete_expired(self, kind: io::ErrorKind) {
        let setup = self
            .setup_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let error = Self::expiry_error(setup.as_ref(), kind);
        self.complete(Err(error));
    }

    pub(in crate::git::endpoint) fn expired(&self, now: u64) -> bool {
        let setup = self
            .setup_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(setup) = setup {
            if self.cancelled.load(Ordering::Acquire) {
                setup
                    .clock
                    .terminate(gwz_transport::pool::SetupCause::Cancelled)
                    .deliver();
                return true;
            }
            // Connecting expiry is settled by the shared authority's owners.
            return false;
        }
        self.cancelled.load(Ordering::Acquire) || self.deadline.is_some_and(|at| now >= at)
    }
    pub(in crate::git::endpoint) fn complete(self, result: OpenOutcome) {
        // Release admission before publishing the reply. The physical pool now
        // bounds an active stream; pending admission remains independently bounded.
        let Self {
            reply,
            permit,
            progress,
            context,
            setup_slot,
            ..
        } = self;
        drop(permit);
        if let Some(reply) = reply {
            let result = result.map_err(|error| {
                let facts = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let setup = setup_slot.lock().unwrap_or_else(|e| e.into_inner()).clone();
                EndpointOpenFailure::capture(error, facts, setup)
            });
            let _ = reply.send(result);
            // The bridge's owner polls for the reply between its passes.
            if let Some(waker) = context.waker {
                waker.wake();
            }
        }
    }
}
