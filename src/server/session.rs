//! The policy a server applies to every connection it serves, and the call that serves one (plan step 2.3g1; gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §4.3 "Stuck peers"; plan C8).
//!
//! The carrier supplies the mechanisms (its lanes, its inbound bounds, its stall watchdog, its pre-open gate) and this
//! policy their values: the lanes and the registry bound come from the session's limits, so a client that keeps
//! within its own bounds never meets a lane at its frame count; the writer's byte bound (`QUEUE_BYTES`) is not a
//! session limit but backpressure: replies past it are held by the session, within its lanes' frame counts, and sent
//! by its pacer thread as the writer takes frames, so no thread of the session host waits on it and control replies go
//! ahead on the control lane's reserve (`served::stream`'s outbox); the no-progress bound is 60 seconds, after which a peer that has stopped reading ends its session as carrier loss,
//! whatever it has queued; the close bound is the session's own `close_wait`; and an accepted end holds every frame but
//! the handshake's until it queues `SessionOpened`.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gwz_session_host::Limits;

use crate::session_host::HostContext;
use crate::session_host::served::stream::{ServeFault, Served, StreamServing, serve_stream};
use crate::session_host::served::{Dispatch, OperationLog};
use crate::transport_host::carrier::{
    CarrierLimits, InboundLimits, StallBounds, StreamCarrier,
    handshake::{Bounds, Hello, Hooks},
};

/// What a server applies to each session it serves.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) struct SessionPolicy {
    /// The session's limits (session design §1 defaults).
    pub(crate) limits: Limits,
    /// How long the host's carrier may have frames queued with no byte accepted before the session ends as carrier
    /// loss.
    pub(crate) stall: Duration,
    /// The handshake's bounds.
    pub(crate) bounds: Bounds,
    /// What the host says first.
    pub(crate) hello: Hello,
}

#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
impl SessionPolicy {
    /// The no-progress bound of `GwzSplitCommandChannel.md` §4.3: 60 seconds.
    pub(crate) const STALL: Duration = Duration::from_secs(60);

    /// The production policy: the session design's limits, the 60 s stall bound and the handshake's own bounds.
    pub(crate) fn new(hello: Hello) -> Self {
        Self {
            limits: Limits::default(),
            stall: Self::STALL,
            bounds: Bounds::DEFAULT,
            hello,
        }
    }

    /// The carrier's stall bounds: the no-progress bound, and the session's close bound for a closing end's flush.
    pub(crate) fn stall_bounds(&self) -> StallBounds {
        StallBounds {
            no_progress: self.stall,
            close: self.limits.close_wait,
        }
    }

    /// The limits of every carrier the server accepts: the call lane and the control reserve of the session, the
    /// registry bound of both, these stall bounds, and the pre-open gate. The frame counts are the session's; the
    /// writer's byte bound stays the carrier's, and the session holds the replies past it rather than fail them or wait
    /// for them on a thread of the session host. `None` when the session's limits leave a lane without room.
    pub(crate) fn carrier_limits(&self) -> Option<CarrierLimits> {
        let registry_frames = self
            .limits
            .outstanding_calls
            .checked_add(self.limits.control_reserve)?;
        let limits =
            CarrierLimits::new(self.limits.outstanding_calls, self.limits.control_reserve)?
                .with_inbound(InboundLimits {
                    registry_frames,
                    ..InboundLimits::DEFAULT
                })
                .with_stall(self.stall_bounds())
                .awaiting_open();
        Some(limits)
    }

    /// Serves one accepted connection under this policy (`serve_stream`): `outer` is the composition's admission
    /// and control.
    ///
    /// # Errors
    ///
    /// `serve_stream`'s; the carrier is closed either way.
    pub(crate) fn serve(
        &self,
        carrier: Arc<StreamCarrier>,
        session: SessionParts<'_>,
        outer: &mut dyn Hooks,
    ) -> Result<Served, ServeFault> {
        serve_stream(
            carrier,
            StreamServing {
                dispatch: session.dispatch,
                log: session.log,
                root: session.root,
                host: session.host,
                limits: self.limits.clone(),
                hello: &self.hello,
                bounds: &self.bounds,
            },
            outer,
        )
    }
}

/// What a session is served with: the handlers, the log, the workspace and the host context it shares.
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) struct SessionParts<'a> {
    pub(crate) dispatch: Arc<dyn Dispatch>,
    pub(crate) log: Arc<dyn OperationLog>,
    pub(crate) root: PathBuf,
    pub(crate) host: &'a HostContext,
}
